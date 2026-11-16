---
name: new-dsp-module
description: Step-by-step recipe for adding a hEARo DSP module (de-hum, de-ess, de-click, de-plosive, limiter, ...) from spec to UI, test-first. Use whenever creating or substantially reworking a module in crates/hearo-dsp.
---

# Adding a DSP module

1. **Spec.** Copy the module's spec from docs/HANDOVER.md §7 into the top of a new deep dive
   section (docs/deep-dives/) and decide parameters (key, unit, range, default).
2. **Test signals.** Make sure `hearo-testkit::signals` can generate everything the golden
   test needs (seeded). Add generators + a determinism test if not.
3. **Failing golden test.** Create `crates/hearo-dsp/tests/golden_<key>.rs` asserting every §7
   threshold with exact numbers, at 48 kHz plus 44.1/96 kHz where relevant, and a
   false-positive check on `speech_like` and `music_like`. Run `just test-golden` → red.
4. **Implement** `crates/hearo-dsp/src/modules/<key>.rs` implementing `Module`:
   - all allocation in `prepare`; `process`/`reset`/`set_param` RT-safe;
   - every constant derived from `sample_rate`; smoothed parameter changes;
   - exact `latency_samples()`; telemetry: gain reduction + events.
   Register it in `modules::KEYS` and `modules::create` (chain order!).
5. **Green.** `just test-golden`, then `just check` (the RT-safety and stress tests pick the
   module up automatically through `modules::KEYS`).
6. **Benchmark.** `just bench`: `module_alone/<key>` must leave the whole chain under the 5%
   budget. Note the number in the branch's `docs/progress.d/<branch>.md`.
7. **CLI check.** `just signals` then `just cli process scratch/signals/<sig>.wav out.wav
   --modules <key> --report` and sanity-check the report.
8. **FFI toggle + UI** (once M3 exists): expose enable + params through hearo-ffi; add the row
   to the popover with activity light, trigger count, GR meter.
9. **Deep dive.** Explain the algorithm for a musician (compare to familiar plugins), with a
   diagram and "experiments to try" using the CLI.
10. **Review.** Run the `dsp-reviewer` and `rt-safety-reviewer` subagents; fix findings.
11. **Commit** `feat(dsp): add <module>` with docs, a `changelog.d/` fragment and
    `docs/progress.d/<branch>.md` (not PROGRESS.md or CHANGELOG.md: parallel PRs).
