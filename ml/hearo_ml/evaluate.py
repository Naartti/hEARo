"""Evaluate a run and write its report (`just ml-eval RUN=<run>`; HANDOVER §9.3).

Scores the run's best checkpoint, at the threshold picked on validation, on:

- **val** and **test** (real snapshot days): event recall, precision, F1 with 95 % Wilson
  intervals, false positives per minute of reviewed audio, and hits on human-rejected spans —
  next to the **DSP de-smack** on the very same clips (`hearo dataset eval --json`, re-scored
  here with the same rules);
- **synthetic_test** (`gen-signals`, held-out seeds): F1 on synthetic smacks/clicks, false
  positives per minute on synthetic clean speech and music (the M7 targets: F1 ≥ 0.8,
  ≤ 2 FP/min on music).

Test metrics are refused while validation has no events (nothing to pick a threshold on), and
flagged as rough below 50 test events. The Markdown report has aggregate numbers only — never
clip names or audio (snapshots stay on this Mac) — and goes to `docs/ml/reports/<run>.md`;
`metrics.json` in the run folder feeds the comparison with earlier runs.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from collections.abc import Callable
from pathlib import Path

import torch
from scipy.stats import chi2

from hearo_ml import dataset as ds
from hearo_ml.model import Detector
from hearo_ml.scoring import Score, Span, merge, score, wilson
from hearo_ml.train import score_probs

MIN_TEST_EVENTS = 50


def dsp_detections(hearo: list[str]) -> dict[str, list[Span]]:
    """De-smack's detections per snapshot stem, from `hearo dataset eval --json`."""
    out = subprocess.run([*hearo, "dataset", "eval", "--json"], capture_output=True, text=True, check=True).stdout
    res = {}
    for line in out.splitlines():
        if line.strip():
            d = json.loads(line)
            res[d["stem"]] = [Span(x["start_s"], x["end_s"]) for x in d["detections"]]
    return res


def score_dsp(dets: dict[str, list[Span]], items: list[ds.Item]) -> tuple[Score, int]:
    """The DSP baseline over `items` (merged like the model's runs), and how many clips its
    output lacked (`hearo dataset eval` skips unreadable ones; they're left out here too)."""
    total, missing = Score(), 0
    for it in items:
        if it.name not in dets:
            missing += 1
            continue
        total.add(score(merge(dets[it.name]), it.labels, it.seconds))
    return total, missing


def _baseline(dsp: dict[str, list[Span]] | None, items: list[ds.Item]) -> tuple[dict | None, int]:
    """The DSP summary over `items` (None without a baseline or any scored clip), and how
    many clips its output lacked."""
    if dsp is None:
        return None, 0
    s, missing = score_dsp(dsp, items)
    return (summary(s) if missing < len(items) else None), missing


def summary(s: Score | None) -> dict | None:
    if s is None:
        return None
    judged = s.detections - s.neutral
    return {
        "regions": s.regions,
        "hit_regions": s.hit_regions,
        "detections": s.detections,
        "false_positives": s.false_positives,
        "rejected_hits": s.rejected_hits,
        "minutes": round(s.seconds / 60, 2),
        "recall": s.recall(),
        "recall_ci": wilson(s.hit_regions, s.regions),
        "precision": s.precision(),
        "precision_ci": wilson(s.hits, judged),
        "f1": s.f1(),
        "fp_per_min": s.fp_per_min(),
        "fp_per_min_upper": fp_rate_upper(s.false_positives, s.seconds),
    }


def fp_rate_upper(k: int, seconds: float) -> float | None:
    """Exact 95 % (one-sided 97.5 %) Poisson upper bound on false positives per minute."""
    if seconds <= 0:
        return None
    return float(chi2.ppf(0.975, 2 * k + 2) / 2) / (seconds / 60)


#: Thresholds for the precision/recall sweep on validation (a threshold-free view).
SWEEP = (0.3, 0.5, 0.7, 0.8, 0.9)

#: How each split's clips were picked: the DSP row only means something read with this.
SPLIT_NOTES = {
    "val": (
        "Selection set: the checkpoint and threshold were picked here, so the model's row is optimistic. "
        "Most clips are auto-saved de-smack catches (DSP recall high by construction) and their rejected "
        "spans are DSP's own false hits."
    ),
    "test": (
        "Clips you saved because de-smack *missed* a smack: DSP recall is low by construction, so the DSP "
        "row is context, not a fair head-to-head."
    ),
}


def evaluate(run: Path, data_folder: Path, dsp: dict[str, list[Span]] | None, include_test: bool = False) -> dict:
    model = Detector.from_checkpoint(torch.load(run / "model.pt", weights_only=False, map_location="cpu"))
    best = json.loads((run / "best.json").read_text())
    meta = json.loads((run / "meta.json").read_text())
    sets = {s: ds.load(data_folder, s) for s in ("val", "test", "synthetic_test")}
    probs = {s: {it.name: model.predict(it.feats) for it in items} for s, items in sets.items()}
    thr = best["threshold"]
    val_events = sum(len(it.labels.targets) for it in sets["val"])
    res: dict = {
        "run": run.name,
        "meta": meta,
        "threshold": thr,
        "step": best["step"],
        "warnings": [],
        "splits": {},
    }
    for split in ("val", "test"):
        items = sets[split]
        if split == "test" and val_events == 0:
            res["warnings"].append("validation has no events: test metrics withheld (no threshold to apply)")
            continue
        if split == "test" and not include_test:
            res["warnings"].append(
                "test not scored (pass `--test` / `just ml-eval RUN=… TEST=1`): look at it once a model is "
                "chosen on validation, not every iteration"
            )
            continue

        dsp_all, missing = _baseline(dsp, items)
        if missing:
            res["warnings"].append(
                f"{split}: {missing} clips missing from `hearo dataset eval --json` (unreadable there); "
                "the DSP rows leave them out, the model rows don't"
            )
        entry = {
            "clips": len(items),
            "model": summary(score_probs(probs[split], items, thr)),
            "dsp": dsp_all,
        }
        auto = [it for it in items if it.auto]
        if auto:
            entry["auto_clips"] = {
                "clips": len(auto),
                "model": summary(score_probs(probs[split], auto, thr)),
                "dsp": _baseline(dsp, auto)[0],
            }
        res["splits"][split] = entry
        if split == "test" and entry["model"]["regions"] < MIN_TEST_EVENTS:
            res["warnings"].append(
                f"test has {entry['model']['regions']} events (< {MIN_TEST_EVENTS}): intervals are wide, "
                "don't rank models on test yet"
            )
    syn = sets["synthetic_test"]
    if syn:
        by_role = {
            r: [it for it in syn if it.role.startswith(r)]
            for r in ("speech_smacks", "speech_clicks", "speech.", "music")
        }
        p = probs["synthetic_test"]
        res["synthetic"] = {
            "events": summary(score_probs(p, by_role["speech_smacks"] + by_role["speech_clicks"], thr)),
            "speech": summary(score_probs(p, by_role["speech."], thr)),
            "music": summary(score_probs(p, by_role["music"], thr)),
        }
    if sets["val"] and val_events:
        res["val_sweep"] = []
        for t in SWEEP:
            s = score_probs(probs["val"], sets["val"], t)
            res["val_sweep"].append({"threshold": t, "precision": s.precision(), "recall": s.recall()})
    return res


NA = "n/a"


def _pct(x: float | None) -> str:
    return NA if x is None else f"{100 * x:.0f} %"


def _num(x: float | None) -> str:
    return NA if x is None else f"{x:.1f}"


def _ci(ci) -> str:
    return "" if not ci else f" ({100 * ci[0]:.0f} to {100 * ci[1]:.0f})"


def _row(label: str, s: dict | None) -> str:
    if s is None:
        return f"| {label} | {NA} | {NA} | {NA} | {NA} | {NA} |"
    fpm = _num(s["fp_per_min"])
    if s.get("fp_per_min_upper") is not None:
        fpm += f" (≤ {s['fp_per_min_upper']:.1f})"
    return (
        f"| {label} | {s['hit_regions']}/{s['regions']} = {_pct(s['recall'])}{_ci(s['recall_ci'])} | "
        f"{_pct(s['precision'])}{_ci(s['precision_ci'])} | {_pct(s['f1'])} | {fpm} | "
        f"{s['false_positives']} ({s['rejected_hits']} rejected) |"
    )


def report(res: dict, previous: dict | None) -> str:
    m = res["meta"]
    sha = m["git_sha"][:10] + ("+dirty" if m["git_sha"].endswith("+dirty") else "")
    mc, tc = m["model"], m["train"]
    lines = [
        f"# {res['run']}",
        "",
        f"Model `{m['name']}`: {m['params']:,} parameters ({mc['channels']} channels, {mc['hidden']} GRU units, "
        f"dropout {mc.get('dropout', 0.0)}), lookahead "
        f"{0 if mc['bidirectional'] else mc['lookahead'] * 10} ms"
        f"{' (teacher: bidirectional, offline only)' if mc['bidirectional'] else ''}. "
        f"{tc['steps']} steps; best validation step {res['step']}, threshold {res['threshold']}.",
        "",
        f"Data: manifest `{m['manifest_sha256'][:12]}`, cache `{m['data_key']}`, git `{sha}`. "
        + ", ".join(f"{k} {v['clips']} clips / {v['minutes']} min" for k, v in m["data_sets"].items())
        + ".",
        "",
    ]
    if res.get("test_views"):
        res["warnings"].append(f"test scored for this run {res['test_views']} time(s)")
    for w in res["warnings"]:
        lines.append(f"> ⚠ {w}")
    if res["warnings"]:
        lines.append("")
    head = (
        "| | Recall (95 % CI) | Precision (95 % CI) | F1 | FP/min (95 % upper) | False positives |\n"
        "|---|---|---|---|---|---|"
    )
    for split, e in res["splits"].items():
        lines += [
            f"## {split} ({e['clips']} real clips)",
            "",
            SPLIT_NOTES[split],
            "",
            head,
            _row("model", e["model"]),
            _row("DSP de-smack", e["dsp"]),
        ]
        if "auto_clips" in e:
            a = e["auto_clips"]
            lines += [
                _row(f"model, auto-saved clips ({a['clips']})", a["model"]),
                _row("DSP, auto-saved clips", a["dsp"]),
            ]
        lines.append("")
    if "synthetic" in res:
        s = res["synthetic"]
        lines += [
            "## Synthetic test (gen-signals, held-out seeds)",
            "",
            "M7 targets: F1 ≥ 80 % on synthetic events, ≤ 2 false positives per minute on music. The smacks and "
            "clicks come from the same generator as the synthetic training events (other seeds), so this is "
            "in-distribution; the clean speech and music were never trained on. Crude stand-ins: real days decide.",
            "",
            head,
            _row("smacks + clicks", s["events"]),
            _row("clean speech (no events)", s["speech"]),
            _row("music (no events)", s["music"]),
            "",
        ]
    if res.get("val_sweep"):
        lines += [
            "Validation precision / recall at other thresholds: "
            + ", ".join(f"{r['threshold']}: {_pct(r['precision'])} / {_pct(r['recall'])}" for r in res["val_sweep"])
            + ".",
            "",
        ]
    if previous:
        pv = previous["splits"].get("val", {}).get("model") or {}
        lines += [
            f"Previous best run by validation F1: `{previous['run']}` — val F1 {_pct(pv.get('f1'))}, "
            f"recall {_pct(pv.get('recall'))}, precision {_pct(pv.get('precision'))}.",
            "",
        ]
    lines += [
        "Event rules match `hearo dataset eval`: ±20 ms tolerance, first 0.5 s ignored, detections on neutral "
        "regions (breaths) not counted, false positives = everything else in exhaustively reviewed clips; "
        "detections < 30 ms apart are one event on both sides. FP/min is over 8 s clips saved around events, "
        "not a rate for normal listening. Wilson intervals treat events as independent; they cluster by clip "
        "and day, so the true uncertainty is wider.",
        "",
    ]
    return "\n".join(lines)


def previous_best(runs: Path, exclude: str, manifest: str, name: str) -> dict | None:
    """The best earlier run of the same config name on the same manifest (a teacher is no
    baseline for the streaming detector, and other data isn't comparable)."""
    best, best_f1 = None, -1.0
    for p in runs.glob("*/metrics.json"):
        if p.parent.name == exclude:
            continue
        r = json.loads(p.read_text())
        if r["meta"]["manifest_sha256"] != manifest or r["meta"]["name"] != name:
            continue
        f1 = ((r["splits"].get("val") or {}).get("model") or {}).get("f1") or -1.0
        if f1 > best_f1:
            best, best_f1 = r, f1
    return best


def run_eval(
    run: Path,
    data_root: Path,
    reports: Path,
    dsp_source: Callable[[], dict] | None,
    log=print,
    include_test: bool = False,
) -> Path:
    meta = json.loads((run / "meta.json").read_text())
    folder = data_root / meta["data_key"]
    # One DSP replay per manifest, shared by every config's run (a clip's audio and recorded
    # settings never change; labels come from the run's data cache).
    cache = data_root / f"dsp-{meta['manifest_sha256'][:16]}.json"
    dsp = None
    if cache.exists():
        dsp = {k: [Span(*s) for s in v] for k, v in json.loads(cache.read_text()).items()}
    elif dsp_source is not None:
        dsp = dsp_source()
        cache.write_text(json.dumps({k: [[s.start, s.end] for s in v] for k, v in dsp.items()}))
    res = evaluate(run, folder, dsp, include_test)
    if include_test:
        # How often test has been looked at for this run: every look is a chance to tune on it.
        views = run / "test_views.json"
        n = (json.loads(views.read_text())["views"] if views.exists() else 0) + 1
        views.write_text(json.dumps({"views": n}) + "\n")
        res["test_views"] = n
    (run / "metrics.json").write_text(json.dumps(res, indent=2, default=float) + "\n")
    prev = previous_best(run.parent, run.name, meta["manifest_sha256"], meta["name"])
    reports.mkdir(parents=True, exist_ok=True)
    out = reports / f"{run.name}.md"
    out.write_text(report(res, prev))
    log(f"✔ {out}")
    return out


def latest_run(runs: Path) -> Path | None:
    done = sorted(p.parent for p in runs.glob("*/model.pt"))
    return done[-1] if done else None


def resolve_run(arg: str, runs: Path) -> Path | None:
    """`arg` as a path (as given, or from the repo root, since `just` runs this in `ml/`) or
    a run name in `runs`; empty = the latest run."""
    if not arg:
        return latest_run(runs)
    for p in (Path(arg), ds.REPO / arg, runs / arg):
        if p.is_dir():
            return p
    return None


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="hearo_ml.evaluate", description=__doc__.splitlines()[0])
    ap.add_argument("--run", type=str, default="", help="run folder or name (default: the latest)")
    ap.add_argument("--runs", type=Path, default=ds.ML_DIR / "runs")
    ap.add_argument("--data", type=Path, default=ds.ML_DIR / "data/processed/features")
    ap.add_argument("--reports", type=Path, default=ds.REPO / "docs/ml/reports")
    ap.add_argument("--no-dsp", action="store_true", help="skip the DSP baseline")
    ap.add_argument("--test", action="store_true", help="also score the test days (once a model is chosen)")
    args = ap.parse_args(ds.cli_args(sys.argv[1:] if argv is None else argv))
    run = resolve_run(args.run, args.runs)
    if run is None or not (run / "model.pt").exists():
        print("✖ no finished run: train one with `just ml-train`", file=sys.stderr)
        return 1
    hearo = ds.default_hearo()
    run_eval(
        run, args.data, args.reports, None if args.no_dsp else (lambda: dsp_detections(hearo)), include_test=args.test
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
