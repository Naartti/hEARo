# Open questions / waiting for the human

## H1 — Xcode, signing identity, icon (M0) — DONE 2026-11-15

1. ~~**Xcode**~~ — DONE 2026-11-13. Installed and selected; Swift tests now run in `just check`.
2. ~~**Stable signing identity**~~ — DONE (confirmed 2026-11-15): `security find-identity -v -p
   codesigning` lists 1 valid "Apple Development" identity (the WWDR G3 intermediate is
   installed). Troubleshooting stays in [dev/SIGNING.md](dev/SIGNING.md).
3. ~~**Menu bar icon**~~ — DONE: plain ear (SF Symbol). See [DESIGN.md](DESIGN.md).
4. ~~Git remote~~ — DONE 2026-11-14: <https://github.com/Naartti/hEARo> (CI and releases in
   GitHub Actions, ADR 0016).
5. ~~**Bundle-ID prefix**~~ — DONE 2026-11-15: the owner didn't want `dev`; now
   `io.github.naartti.hearo` (ADR 0005 amendment). Re-grant System Audio Recording once after
   installing the build with the new ID, and re-enable *Launch at login* if used.

## H5 — Notarized releases — DECIDED 2026-11-15: no

The owner won't pay for the Apple Developer Program. Releases stay ad-hoc signed: users click
**Open Anyway** once per version and re-grant System Audio Recording after each update
(docs/user/INSTALL.md). Revisit only if that changes.

## GitHub setup — PARTLY OPEN

1. ~~Codecov~~ — DONE (owner, 2026-11-15): enabled; works in PRs and the README badge.
2. Add **just ci** as a required status check in the `main` ruleset (and require a pull request
   before merging, if you want to stop direct pushes to `main`).
3. Optional: Settings → Actions → General → *Require actions to be pinned to a full-length
   commit SHA* (the hygiene check already enforces it locally).

## H2 — Live passthrough listening test (M2) — DONE 2026-11-13

Result (human, MacBook Pro speakers, 44.1 kHz): passthrough vs bypass indistinguishable; gain
−12 dB toggle smooth; `kill -9` → normal audio immediately. Measured output exactly −12.0 dB,
13–23 µs per 512-frame cycle, 0 overruns.

**Feedback → new top priority:** loud **lip smacks** in speech pauses (lips unsticking after a
pause: short, sharp, bright transient) are "VERY annoying".

## H4 (early) — Capture real lip smacks — OPEN (ongoing)

Keep capturing in meetings, podcasts and videos (ADR 0017, docs/ml/README.md): press **`§`**
(anywhere, no Enter) right after a smack, or turn on auto-save in Settings. The ear shows a
checkmark (a notification + green line in `just cli run`); each press saves the last 8 s of
*unprocessed* audio + a JSON sidecar to `~/Library/Application Support/hEARo/snapshots/`
(local only, never committed: snapshots can contain other people's voices). In Review, mark
*every* mouth noise in a clip before marking it reviewed; press `f` on dulled consonants too.
Clips from different speakers/shows help most.
Status 2026-11-15: 111 reviewed clips (incl. auto-saved catches) — see `just cli dataset eval`.

## H3 (de-smack) — Listen to the lip-smack remover — OPEN (tuning)

- **Decided 2026-11-15: (a)** — keep ≈ 43–50 ms latency ("I can't really tell the latency when
  watching YouTube videos"); loud close-mic smacks with a slow "wet" tail are left to the ML
  stage (M7/M8). No 100 ms "max cleanup" mode.
- Still open: report anything dulled (especially t/k/p/s), pumping or clicks; if auto-catches
  are still too eager, try *Below speech* 18 or 15 dB in Settings.

## Morning checklist (2026-11-14) — DONE 2026-11-15

Owner: latency fine for video; Review tab "nice, good shortcuts"; installed the GitHub DMG on
the other Mac following INSTALL.md — "works great", "very easy". Listening notes on individual
modules moved to H3.
