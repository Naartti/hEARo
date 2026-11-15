# User guide

hEARo is pre-1.0 and has no downloadable releases yet: build it from source as described in
the [README](../../README.md#build-from-source). A fuller user guide (permissions,
troubleshooting, uninstall) arrives with the 1.0 release.

- [INSTALL.md](INSTALL.md) — installing a DMG built with `just release` on another Mac,
  first launch of an unnotarized app, the System Audio Recording permission, snapshots.
- Privacy in short: audio is processed on your Mac and never sent anywhere; only snapshots
  you save yourself with the `§` hotkey are written to disk
  (`~/Library/Application Support/hEARo/snapshots/`) — see the
  [README](../../README.md#privacy).

## How much each module removes

Click the ear in the menu bar. Every cleanup module has a checkbox and, under it, a slider for
how much it does:

| Module | Slider | Higher means |
|---|---|---|
| De-hum | Amount | deeper hum notches |
| De-plosive | Max cut | deeper cut of "p"/"b" pops |
| De-click | Sensitivity | more (fainter) clicks repaired |
| De-smack | Range | lip smacks turned down further |
| De-ess | Range | harsh "s" sounds turned down further |

Changes are heard immediately and remembered across restarts (in
`~/Library/Application Support/hEARo/settings.json`). For every parameter of every module
(thresholds, frequencies, attack/release, the limiter ceiling…), click **Settings…** under the
list, or choose **Settings…** (⌘,) from the ⋯ menu or by right-clicking the menu bar icon.
Each module has **Reset to defaults**; **Reset All…** resets every value (which modules are on
stays as it is). **Bypass (A/B)** in the popover switches to the unprocessed sound and back.

The colour square before each module's name is its colour in the waveform: when a module
turns something down, the outer part of the waveform is painted in that colour, as deep as the
cut — a −12 dB de-smack colours most of the stroke, a −2 dB de-ess only a thin rim. The grey
core is what you hear.

## Trying settings on a snapshot (A/B)

Open **Review snapshots…** from the ⋯ menu. The **Processing** panel on the right (⌥⌘I hides
it) has every module with its on/off checkbox and all its settings, starting from the ones
you use live. Each change re-renders the open clip in a fraction of a second; the waveform
shows, in each module's colour, what it turned down, with a small tick above every removed
sound.

- Press **B** (or flip **Bypass**) while playing to switch between the original and the
  processed clip at the same moment — it keeps playing, and a looping region keeps looping.
- **Apply these to live** makes the panel's settings your live (and remembered) settings;
  **Use my live settings** throws away the experiment. Nothing changes live until you apply.
