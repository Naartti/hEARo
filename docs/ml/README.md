# ML

Data sources, training guide and model reports (M7). Reports go in `reports/`.

## Dataset plan: hotkey snapshots (ADR 0017)

Mouth-noise data comes from the human's hotkey snapshots, captured over days of normal use
(meetings, podcasts, videos), not from a recording session: different people smack differently,
and snapshots carry the real speakers, mics, rooms and codecs hEARo will process.

### Capturing

- Press `§` after a missed smack, and `f` ("false hit") when hEARo dulls something it shouldn't
  (a final "t", a breath, an "s") — those are the most valuable negatives.
- Widen the mix: podcasts and YouTube as well as meetings, so it isn't only the same colleagues.
- Expect ~5–20 events per meeting hour; a few days gives ~100–300.

### Labelling (Review tab)

- Mark regions tightly around each event (e.g. a 29 ms smack gets a ~30 ms region); the press
  time alone is too loose for a frame-level detector.
- **Reviewed = exhaustive**: before marking a clip reviewed, mark *every* mouth noise in its
  8 s, not just the one you pressed for. Then everything unmarked is a negative, and no separate
  negative labelling is needed. Clips you didn't go through fully are test-only or skipped.
  The Review tab says this under its Reviewed toggle ("Mark every mouth noise first").
- Mark overlapped smacks (under speech or noise) too, even if messy: they are the honest test
  of the overlap case.

### Auto-saved catches (ADR 0018)

With *Settings → Auto-save de-smack catches* on, hEARo saves an 8 s clip around every de-smack
catch (label `auto`, the catch pre-marked as a proposal). In Review, **A** accepts it (a region:
a verified smack) and **X** marks it a false positive (the sidecar's `rejected` list). The same
"reviewed = exhaustive" rule applies before Return.

- Accepted catches are clean, isolated smacks: the best source for the **mixing pool**.
- Rejected catches are de-smack's real mistakes (a final "t", a breath, an "s"): **hard
  negatives** — train on them as "not a mouth noise" at exactly those spans.
- Auto clips only contain what de-smack already finds, so they don't measure the overlap case:
  keep the **test set** to hotkey clips you marked yourself.
- Untouched auto clips are deleted after the retention set in Settings (default 31 days);
  anything you reviewed or edited stays. `hearo dataset propose` skips auto clips.
- `hearo dataset eval` scores de-smack (or `--module` another detector) against every reviewed
  clip: regions found, false positives, and how many land on rejected spans. It's how DSP
  changes are judged on real audio; clips never leave the Mac, so it isn't a CI test.
  It replays what fed the detector live: new sidecars have a `settings` key (the app's
  `settings.json` shape: the live chain in order, which modules were on, their parameter
  values), and the modules before the scored one run with those values. The scored module
  runs at its defaults plus `--param`. Older clips only list the chain (`modules`), so every
  module listed before the scored one runs at its defaults, even if it was off; the report
  counts them and gives separate totals for the recorded clips — compare runs over time on
  that line, since the mix shifts as new clips arrive. Known limit: settings are recorded when
  the clip is saved, so a toggle or slider moved within the clip's 8 s isn't reflected.
  `--json` prints one line per clip with its detections instead (`{"stem", "detections":
  [{"start_s", "end_s"}]}`); `just ml-eval` uses it to score the DSP baseline per split.

### Using it

`just ml-import-snapshots` (`hearo_ml.snapshots`) turns this Mac's reviewed snapshots into
`ml/data/processed/snapshots/manifest.jsonl` (one line per clip: WAV path, day, split,
regions, rejected spans) and `pool.jsonl` (mixing-pool events), both git-ignored, and prints a
summary with the skipped clips by reason and warnings. It applies the rules below and refuses
to write a manifest that breaks them (`check_no_leakage`).

- **A day** is the local date a listening session started (clips < 1 h apart are one
  session, so a call across midnight stays together); `--tz` fixes the zone.
- **Splits are locked:** the first run writes `splits.lock.json` next to the manifest, and a
  day in it never changes split — a test day is never later trained or tuned on. New days
  get theirs from a stable hash of the date (≈ 20 % test, 10 % validation). Until there are
  enough days, fallbacks fill the missing splits (with three days, one whole day each); a
  fallback test day is the one with the fewest auto-saved clips, which can't be test data.
  Delete the lock only on purpose (a new dataset version), and note it in the model card.
- `meta.json` records the manifest/pool SHA-256, the parameters and the day → split map:
  log it with every training run and report.
- **No tuning on test.** Use validation; with no validation days yet, wait. Fewer than 50
  test events gives intervals too wide to compare models (the summary warns).
- **Known limit:** splitting by day keeps a day's meetings apart, not the people — recurring
  colleagues can be in train and test. Say so in the model card; a per-clip "group" tag
  (meeting series, podcast) would let the split follow speakers.
- Pool events are lip smacks and mouth clicks from training days with no other region (or
  rejected span) within 100 ms and audio 50 ms either side ≥ 20 dB below the event.

- **Splits by recording day/session**, never by clip: the same people recur across meetings,
  and one speaker's smacks in both train and test inflates every metric. A test enforces it.
- **Test set:** held-out days of real, unmixed snapshots, including the overlapped smacks.
- **Mixing pool:** clean, isolated events (quiet before and after) from *training* days only,
  plus Freesound CC0/CC-BY clips (attribution logged), mixed into clean speech and noise at
  random times and levels with codec round-trips (HANDOVER §9.1) for exact frame labels.
- Caveat: pool events are pause smacks — what the DSP de-smack already catches — so overlap
  performance is only measured honestly on the real test set.

### Privacy

Snapshots contain other people's voices. They stay on this Mac (or are merged
between the human's Macs with `hearo dataset export/import`); never committed or uploaded. A
model trained on them is not published without a separate decision.

## Training and evaluation (Phase A detector, ADR 0020)

After a review session, one command does everything:

```sh
just ml-retrain   # import snapshots → build data → train each ml/configs/*.yaml → report each
```

Or step by step:

| Step | Command | Output (git-ignored unless noted) |
|---|---|---|
| Manifest | `just ml-import-snapshots` | `ml/data/processed/snapshots/` |
| Data | `just ml-data [CONFIG=…]` | `ml/data/processed/features/<key>/`, reused while the manifest and the config's `data` section are unchanged |
| Train | `just ml-train CONFIG=ml/configs/detector.yaml` | `ml/runs/<timestamp>-<name>/`: config, `meta.json` (git SHA, manifest hash, data key), `history.jsonl`, `model.pt`, `best.json` |
| Report | `just ml-eval [RUN=<run>] [TEST=1]` (default: the latest run, validation only) | `docs/ml/reports/<run>.md` (committed), `metrics.json` in the run |

Two configs: `detector.yaml` is the **streaming detector** (causal, 30 ms lookahead, the one
that will run live), and `teacher.yaml` is the **offline teacher** (bidirectional, sees the
whole clip; the upper bound, later the auto-save checker). Change hyperparameters in the
YAML, never in code.

What the data step builds:

- **train**: each train-day clip as recorded, plus 12 mixes of it (train-day smacks and
  synthetic `gen-signals` smacks/clicks pasted in at −20 to +10 dB, EQ tilt, gain, Opus/AAC
  round-trips), plus the synthetic speech-with-events signals from train seeds.
- **val / test**: the real clips of those days, as they are.
- **synthetic_test**: `gen-signals` from held-out seeds, 5 min each (the M7 F1 and music
  targets). Its clean speech and music are never trained on.

How to read a report:

- **val** decides: the checkpoint and the threshold are picked there, so its numbers are
  optimistic. **test** is scored only with `TEST=1`, once a model is chosen; each run counts
  how often it was looked at. Below 50 test events its intervals are too wide to rank models.
- Each table puts the model next to the **DSP de-smack on the same clips**, scored by the
  same rules as `hearo dataset eval`, but how the clips were picked biases that comparison:
  test clips are smacks de-smack *missed* (its recall is low there by design), and validation
  is mostly its own auto-saved catches (high recall, and your rejections are its false hits).
  Read the DSP row as context. A fair comparison needs clips saved at random times.
- FP/min is over 8 s clips saved around events, so it isn't a rate for normal listening; the
  bracket is a 95 % upper bound.
- Reports contain aggregate numbers only, never clip names or audio.
