# Open questions / waiting for the human

## H1 — Xcode, signing identity, icon (M0) — DONE 2026-11-13

1. ~~**Xcode**~~ — DONE 2026-11-13. Installed and selected; Swift tests now run in `just check`.
2. **Stable signing identity** — certificate created (an "Apple Development" identity)
   and `SIGN_IDENTITY` set in `.env.local`, but macOS reports it *invalid*: only
   the expired (2023) WWDR intermediate is installed; the cert is issued by **WWDR G3**.
   Fix: install <https://www.apple.com/certificateauthority/AppleWWDRCAG3.cer> into the login
   keychain, then `security find-identity -v -p codesigning` → 1 valid, then `just doctor`.
   Details: [dev/SIGNING.md](dev/SIGNING.md).
3. ~~**Menu bar icon**~~ — DONE: plain ear (SF Symbol). See [DESIGN.md](DESIGN.md).
4. Git remote: not needed for CI (local only); add one only if you want an off-machine backup.

## H2 — Live passthrough listening test (M2) — DONE 2026-11-13

Result (human, MacBook Pro speakers, 44.1 kHz): passthrough vs bypass indistinguishable; gain
−12 dB toggle smooth; `kill -9` → normal audio immediately. Measured output exactly −12.0 dB,
13–23 µs per 512-frame cycle, 0 overruns. Not yet tested: output-device switching (next M2 task).

**Feedback → new top priority:** loud **lip smacks** in speech pauses (lips unsticking after a
pause: short, sharp, bright transient) are "VERY annoying".

## H4 (early) — Capture real lip smacks — OPEN

While `just cli run` is running, press **`§`** (anywhere, no Enter) right after a smack. A
notification + a green terminal line confirm it. Each press saves the last 8 s of *unprocessed*
audio + a JSON sidecar to
`~/Library/Application Support/hEARo/snapshots/` (local only; never committed — snapshots can
contain other people's voices). 5–20 examples from different speakers/shows are ideal; tell me
roughly where in each clip the smack is if you can (e.g. "around 6 s").
Status: 4 clips captured (6 s each, before the 8 s fix); 3 show a clear pause smack
0.6–2 s before the press (5–15 ms, 15–25 dB above the pause).

## MVP app test — DONE 2026-11-13

Human: all MVP checks pass with the menu bar app driving the live engine (permission prompt for
hEARo Dev, status line, bypass A/B, § snapshot with menu bar checkmark, on/off).

## H3 (de-smack) — Listen to the lip-smack remover — OPEN

The app now runs **De-smack**. Listen to the podcasts/shows where you heard smacks:

- Toggle **De-smack** off/on in the popover, or hold **Bypass (A/B)**: smacks in pauses should
  become quiet; speech (especially words starting with t/k/p) must sound unchanged.
- The ear glows in the accent colour each time a smack is removed.
- Still annoying smack → press `§` ("missed it"). Something removed that shouldn't be → run
  `just cli run --modules desmack` and press `f` + Enter ("false hit"), or tell me.
- Latency is now ≈ 43 ms (ADR 0011) — say if video lip sync or calls feel off.

## Morning checklist (after the overnight run, 2026-11-14) — OPEN

1. ~~Confirm or reject ADR 0013~~ — **accepted** by the human 2026-11-14.
2. **Listen (H3)** with the dev app: all six modules are on — de-hum, de-plosive, de-click,
   de-smack, de-ess, limiter (always on). Toggle each in the popover; watch the live waveform
   (red = removed). Report anything dulled (especially t/k/p/s), pumping or clicks.
3. **Latency is now ≈ 44.5 ms** (64-frame IO buffer; ADR 0012) — check video lip sync and calls.
4. **Review:** ⋯ → *Review snapshots…* (the Review tab of the hEARo window) — check keyboard focus (Space/1–6/arrows),
   scrolling/zoom, playback; proposals are pre-marked for 13 of your 21 clips.
5. **Other Mac:** build a fresh installer with `just release` (the old DMG predates this session), follow `docs/user/INSTALL.md`
   (right-click → Open the first time). Move snapshots with ⋯ → Export/Import.
6. A "VS Code wants to control Finder" dialog may be waiting from an early DMG attempt — Don't
   Allow is fine (the DMG no longer uses Finder).
