# Design

Product and UI design system. The full specification is [HANDOVER §13](HANDOVER.md); this
file records decisions and the current state.

## Principles

- Native macOS: SF Pro, SF Symbols, system materials, light/dark, accent color.
- Show, don't hide, what processing does — the visualizer is a core feature.
- Accessible: VoiceOver labels on every control and meter, keyboard navigation, Reduce Motion.
- Cheap: no telemetry polling while the popover/visualizer is hidden (< 0.5% CPU idle UI).
- **One window** (human request 2026-11-14): everything bigger than the popover lives in the
  single hEARo window as a toolbar tab, never in another window. Anything the popover shows can
  also be kept open there — the popover vanishes on any click, which is useless while testing.
- **Present only while in use** (human request 2026-11-14): hEARo lives in the menu bar; it
  takes a Dock icon, ⌘Tab slot and menu bar only while its window is open. Add chrome only when
  it makes the experience better, and follow macOS convention when it does.
- **One name per place** (2026-11-15): every control that opens a tab or window uses that
  place's name, and the macOS name where there is one (settings are "Settings…", ⌘,).

## Menu bar icon

- **Decided (H1, 2026-11-13): a plain ear**, using SF Symbols — they are template images, so
  macOS tints them for light/dark menu bars, and they scale with the system. No custom artwork
  and no icon-variant setting (supersedes the cape variants in HANDOVER §13.2).
- States (human request 2026-11-13: "visual confirmation that it is active"):

  | State | Icon | Colour |
  |---|---|---|
  | Off | `ear` outline | template (follows menu bar) |
  | On, idle (nothing playing) | `ear.fill` | template |
  | On, processing audio | `ear.fill` | template, subtle steady state |
  | **Just removed something** (smack, click, ...) | `ear.fill` | **accent colour glow for ~0.5 s**, then back; bursts coalesce |
  | Snapshot saved (`§`) | `ear.badge.checkmark` | accent, ~1.2 s |
  | Auto-saved a de-smack catch | `ear.badge.checkmark` | **de-smack orange** `#F2662A` in both appearances (the menu bar follows the wallpaper, not the app), ~1.2 s |
  | Problem (permission denied, fallback) | `ear.trianglebadge.exclamationmark` | template |

  Colour is only used for *momentary* events, so the icon stays a calm template image the rest
  of the time and still reads correctly on light/dark/tinted menu bars. Driven by the module
  trigger counters in the polled status (no extra audio-thread work). Under Reduce Motion: no
  glow animation, just a static accent fill for the same duration.

## The hEARo window (human request 2026-11-14)

"Merge advanced settings and review snapshot so they are the same window but different tabs,
something that looks native. Also the popover as a view there to keep it open — handy for
validation and testing."

- One `Window("hEARo", id: "main")` (`HearoKit/MainWindow.swift`) replaces the separate
  Review snapshots and Advanced Settings windows. Tabs **Controls · Review · Settings** as a
  segmented control in the toolbar (the Activity Monitor pattern; a sidebar would sit next to
  Review's own clip list), **⌘1–⌘3** switch (View menu, see *Dock icon while the window is
  open*; the picker's tooltip lists them). The title
  text is removed from the toolbar so the tabs fit the narrow Controls window (it would push
  them into an overflow menu); the window is still called "hEARo" for Mission Control and
  VoiceOver.
- **Controls** is the popover's content (`ControlsView`, same 360 pt column) — it stays open
  until the window is closed. The popover header has a window button (`macwindow`, "Open in a
  window") that opens it on this tab. The live waveform polls while the popover *or* this tab
  is on screen (`AppModel.setLiveViewVisible`, tracked per view; a view joining doesn't restart
  the timer; Reduce Motion in either view slows it to 10×/s). "On screen" uses AppKit's
  occlusion state, so a minimised, hidden (⌘H) or fully covered window stops polling too.
- Opening: popover window button → Controls; "Settings…" link and ⋯ / right-click
  "Settings…" → Settings; "Review snapshots…" → Review; new menu entry **Open hEARo
  Window** → Controls. Every route activates the app (menu-bar-only apps otherwise open
  windows behind the frontmost app). In the window, "Settings…" just switches tabs.
- **Naming (2026-11-15):** the link and the ⋯ / right-click entry were "Advanced…" /
  "Advanced settings…" while the tab and the app menu said "Settings"; all four are now
  **Settings…**, the macOS name for app settings, and the ⋯ / right-click entry shows ⌘, like
  the app menu (shared title: `AppMenu.Item.settings`). "Advanced" would also suggest a
  second, hidden level of settings that doesn't exist — the tab *is* hEARo's settings.
- **Sizing:** each tab has its own window size, like Xcode's settings tabs — Controls ≈ 460 ×
  640 (so it sits beside the app under test), Review 1300 × 680, Settings 600 × 720. The window
  opens at the size of the tab it opens on; switching tabs resizes it (top-left corner fixed,
  animated unless Reduce Motion) and each tab remembers the size you gave it while the app
  runs. Never below the tab's minimum (Review 820/1100 pt with the inspector). Settings keeps a
  680 pt column centred if the window is made wider.
- **Switching away from Review** only stops playback; the open clip, render, spectrograms and
  trial settings stay. **Closing the window** drops the caches and temporary renders, as
  closing the old review window did (`ReviewModel.tabHidden` / `windowClosed`). Bare digits
  pick a region class; ⌘1–⌘3 never do.

### Dock icon while the window is open (human request 2026-11-14)

"Why isn't there an application icon in the Dock? If it's not needed and doesn't make the
experience better then we shouldn't have it. But if it's best practice then we should."

- **Decided: a Dock icon only while the hEARo window is open** (the Docker Desktop / Tailscale
  / 1Password pattern). With no window, hEARo stays menu-bar-only (`LSUIElement`, no Dock icon,
  not in ⌘Tab) — a background utility in the Dock is clutter. Opening the window switches to a
  regular app (`setActivationPolicy(.regular)` before activating, or the menu bar can fail to
  appear); closing it switches back to `.accessory` and hands focus back to the app you were
  in. The window drives this (`DockPresence`: appear → regular, close → accessory one run-loop
  turn later unless it reopened), so every route into the window is covered. Launch stays
  silent: no Dock flash, and the window is never restored at launch
  (`.restorationBehavior(.disabled)`). Supersedes HANDOVER §13 "no Dock icon" for the window
  case.
- Why: a window kept open for testing must be findable — ⌘Tab, the Dock, Mission Control —
  and gets a real menu bar instead of workarounds.
- **Menu bar while open** (`MainWindowCommands`): app menu "About hEARo" (same panel as the ⋯
  menu) and **Settings… ⌘,** → Settings tab; **View → Controls ⌘1 · Review ⌘2 · Settings ⌘3**
  (replaces the hidden zero-opacity shortcut buttons); standard Hide/Quit/Window menus; Help
  keeps only the system search field (no help book). With the window open the tab items just
  switch tabs; otherwise (e.g. ⌘, while the popover is focused) they open the window on it.
- No "always show in Dock" setting; add one only if asked.

## Popover (360 pt wide) — current state

| Section | Status |
|---|---|
| Header: icon, name, status line ("Processing 3 apps · +18 ms · 1.4% CPU"), master switch | M0 shell |
| **Live waveform** (below the header): original sound scrolling right→left, ~3 s, 60 columns/s, **outside-in colour bands per module** where something was turned down (see *Module colours*); polls only while the popover is open | done (human requests 2026-11-13, 2026-11-14) |
| Full visualizer (spectrum, removed-energy colours) | M5 |
| Modules: colour swatch (the legend), toggle, name, trigger count; under each name an **amount slider** for the module's primary parameter; "Settings…" link | done (human request 2026-11-14), see *Module settings* |
| Sources: apps producing audio with include/exclude | M3 |
| Labeling hints + Review… | Review tab of the hEARo window (⋯ → "Review snapshots…"); hints M6 |
| Footer: Settings…, Visualizer…, Quit | ⋯ menu: About hEARo / Open hEARo Window (not shown inside the window) / Review / Settings (⌘,) / Export / Import / Show in Finder / Launch at login / **Quit (⌘Q)**; the **same menu on right-click** of the menu bar icon (`HearoKit/AppMenu.swift`, human request 2026-11-14) |
| Special states: onboarding, permission denied, engine fallback | denied state done |

## Module colours (human request 2026-11-14)

One colour per cleanup module, defined once in `HearoKit/ModuleStyle.swift` and used
everywhere a module is shown (popover list, live waveform, Settings and Review tabs).
The popover's module list doubles as the legend: a swatch sits before each module's name.

| Module | Light | Dark | Name |
|---|---|---|---|
| De-hum | `#0A4F8C` | `#3B8FD9` | blue |
| De-plosive | `#B8509A` | `#E08DBF` | pink |
| De-click | `#A67200` | `#F2B233` | amber |
| De-smack | `#8F2A00` | `#F2662A` | orange-red |
| De-ess | `#2B86C5` | `#8FD4F7` | sky blue |
| Limiter | `#8E8E93` | `#98989D` | grey (no band) |

Based on the Okabe–Ito colour-blind-safe palette, with lightness spread (L\* 33–54 light,
58–82 dark) so pairs stay apart under simulated protanopia, deuteranopia and tritanopia (minimum
ΔE ≈ 13, not just hue — de-smack is a dark rust in light mode so it can't be confused with
amber under deuteranopia). Every light variant is ≥ 3.3:1 against light window backgrounds
(`#ECECEC`–white; WCAG 1.4.11, asserted in `ModuleUITests`); dark variants are lighter so they
read on dark windows and on the magma spectrogram (≥ 4:1 on `#2A2A2A`, ≥ 6:1 against magma's
darkest colour). Colour is still never the only cue: the swatch is always next to the module
name, the tooltip names the colour, and VoiceOver reads module names and dB values.

### Outside-in waveform bands

"Not the entire waveform in that colour but a portion depending on how much it is triggered."
Each waveform column is drawn as the **original** peak; where cleanup modules turned it down,
the stroke is coloured from the outside in and the uncoloured grey **core** is what remains
(`ReductionBands` in `Waveform.swift`, drawn by `BandPainter` in `WaveformView.swift`):

- **Depth = share of amplitude removed** (linear): core = output ÷ input peak, so −12 dB
  colours 75 % of the stroke, −2 dB 21 %, −0.5 dB 6 %. (The live waveform's stroke *height*
  stays dB-scaled so quiet speech is visible; the bands are fractions of that stroke.)
- The core uses the **measured output peak**, clamped to the input peak, but never paints more
  than the modules report (core ≥ 10^(−Σr/20)), so limiter gain or a louder sample elsewhere
  in the column is not attributed to a cleanup module.
- The rim is **split among modules in proportion to their dB reduction**, in **chain order from
  the outside in** (de-hum outermost, then de-plosive, de-click, de-smack, de-ess innermost):
  the first module removed the outermost part, and a fixed order keeps each colour in a
  predictable place instead of reshuffling column by column.
- Reductions under 0.5 dB are meter noise and not drawn; a module that cut ≥ 1 dB gets at least
  a 1 pt band, so a small cut on a quiet column stays visible. No reduction → plain grey stroke.
- One `Path` per colour per frame (cheap at 60 columns/s). VoiceOver reads "Recently turned
  down: De-smack up to 18 dB, De-ess up to 8 dB". Under Reduce Motion the popover polls the
  waveform 10×/s instead of 30×/s, so it advances in chunks rather than scrolling smoothly.
- Reduction rows are matched to colours **by module key**, never by position, so a future
  engine that sends a subset or another order can't paint the wrong colour.

## Module settings (human request 2026-11-14)

"Select how much to apply of each module, stored so I can tweak it — like advanced settings."

- **Popover:** each module row has a second line with its *primary* parameter (from the Rust
  catalog, `ModuleInfo.primaryParam`): de-hum *Amount* (%), de-plosive *Max cut* (dB), de-click
  *Sensitivity* (%; a click repair has no depth, so it is labelled for what it does), de-smack
  and de-ess *Range* (dB). Small slider, the parameter name before it and the formatted value
  after it; disabled while the module is off. VoiceOver: "De-smack range", value "12 dB".
- **Settings** — the hEARo window's **Settings** tab (`SettingsTabView`, see *The hEARo
  window*; opened from the popover's "Settings…" link or the ⋯ / right-click menu → "Settings…"): a grouped
  form with one section per module in chain order — swatch, on/off checkbox (the limiter says
  "always on"), every parameter, the module description, and **Reset to defaults** (disabled
  when already default). Discrete parameters (`choices`, e.g. de-hum mains Auto/50/60 Hz) are
  a segmented picker, whole-number steps (harmonics) a stepper, everything else a slider with
  the value beside it. **Reset All…** (with confirmation) at the bottom. Resets restore values,
  not on/off.
- **Values** read by unit (`ParamFormat`): "24 dB", "−40 dB" (real minus), "50%" (locale
  percent), "150 Hz", "7.5 kHz", "60 ms", "0.5 ms", "4:1", choices by label; decimals follow
  the user's locale.
- **Live + remembered:** every change goes to the engine at once (`set_param`, live within one
  audio buffer) and the engine's clamped/snapped value is shown. Settings are saved as JSON (the
  Rust `settings_to_json` shape, ADR 0015) to `~/Library/Application Support/hEARo/settings.json`,
  atomically, 0.4 s after the last change (a slider drag writes once; quitting — menu, logout or
  restart — saves a pending change at once). At launch the file is applied before processing starts, and
  again on every start. Loading is tolerant: unknown keys ignored, missing ones default, values
  clamped, a malformed entry skipped on its own; a file that isn't JSON at all is moved to `settings.corrupt.json` and defaults are used.
  Older installs' per-module on/off flags (UserDefaults) are migrated on first launch.
  Global bypass is never saved. Code: `ModuleSettingsModel.swift`, `SettingsViews.swift`.

## Copy

- Permission prompt (`NSAudioCaptureUsageDescription`): explains local-only processing and that
  only user-saved snapshots — and, if turned on in Settings, clips around removed smacks — are
  stored.
- Tone: friendly, plain, a little heroic ("Saved — thanks, hero").

## Labeling hotkey (decided with the human, 2026-11-13)

- **"Missed it" = a single `§` press** (no modifiers, no Enter, works from any app) instead of
  ⌃⌥⌘M from HANDOVER §13.5. Smacks are short; the press must be instant. "False hit" keeps a
  modified shortcut (configurable). Both stay user-configurable in Settings → Hotkeys.
- Implemented with Carbon `RegisterEventHotKey` (no Accessibility permission). While hEARo runs,
  `§` is captured system-wide.
- **Confirmation is visual and silent** (a sound would end up in the processed audio/snapshots):
  - CLI (now): bold line in the terminal + a macOS notification.
  - App (M3): the menu bar ear briefly **pulses/fills with a checkmark badge** ("got it!"),
    plus a small HUD "Saved — thanks, hero"; under Reduce Motion a static badge for ~1 s.
- Snapshots keep 8 s ending at the press; the smack is usually 0.5–2 s before it
  (`event_hint_s` in the sidecar), which narrows labeling and training targets.

## Auto-saved de-smack catches (human request 2026-11-14)

"When [de-smack is] active and triggering, can't we just add those to the dataset automatically
… labelled as auto-caught … unverified, but easier to double check and verify." Then: in
Settings, default off; flash the ear orange (the smack colour) when one is saved; adjustable
daily cap (default 250) and auto-delete (default 31 days); catches close together share a clip;
the detection pre-marked with start and end; clear keyboard shortcuts to verify or mark a false
positive — "which is an important dataset also". Privacy is fine for them (local only).

- **Settings tab**, section *Auto-save de-smack catches*: a toggle (off by default), *Daily
  limit* (1–2000, default 250; a rolling 24 h window, so it behaves the same across restarts and
  time zones; steps of 10), *Delete unreviewed auto-saved clips after* (1–365 days, default
  31), and *Saved in the last 24 hours: N of <limit>*. Clips you reviewed are never auto-deleted; neither are `§`
  snapshots. A changed retention deletes nothing for an hour (time to fix a typo). Stored with the app's other preferences (UserDefaults); Rust clamps the values.
- **Capture:** an 8 s clip ending 3 s after the last catch in a burst; catches within ~4.5 s of
  the first share one clip. Saved as label `auto` with the catches as `proposals` (same format
  as `hearo dataset propose`: start and end, class `lip_smack`, module, dB removed).
- **Icon:** `ear.badge.checkmark` in the de-smack colour for ~1.2 s (a `§` snapshot keeps the
  accent colour, so the two are distinguishable at a glance).
- **Review tab:** the list shows *Auto* clips. **A** = accept the detection (it *is* a smack →
  a region), **X** = false positive (stored in the sidecar's `rejected` list: de-smack's
  mistakes, the hard negatives). Both act on the proposal you last showed, else the first
  undecided one; pressing the other key flips a decision. The proposal rows carry the same two
  buttons with their shortcuts in the label ("Accept (A)", "False positive (X)"), and a hint
  line names the flow: *A accept · X false positive · Return reviewed & next*. Rejected
  proposals are drawn grey and crossed out.
- **Permission text** now names the opt-in auto-save as the second exception to "nothing is
  recorded".

## Snapshot review & annotation window (M6) — requirements from the human, 2026-11-13

Goal: mark the **exact** start/end of the annoying sound in a captured clip, fast.

- **Recent snapshots list** (newest first): time, source app, label (missed / false hit),
  annotated or not. Opening one shows its waveform immediately.
- **Waveform view** of the 8 s clip (plus a spectrogram toggle — smacks are easier to see
  as bright vertical lines), with the `event_hint_s` window (0.5–2.5 s before the press)
  shaded so the eye lands in the right place.
- **Playback**: click to place the playhead; **Space plays/pauses the clip under the mouse**
  (hover-to-target, no click needed); a playhead line follows audio (and, when zoomed in, the view pages along with it); playing a selection loops
  it; ⌥Space plays the *processed* version for A/B.
- **Selecting**: drag across the waveform to create a region. **Drag inside a region to move
  it** (keeps its length); drag its edges to resize; snap-free, sample-accurate edges shown in
  ms. Multiple regions per clip; Delete removes the hovered/selected region.
- **Zoom**: pinch / ⌘-scroll to zoom around the pointer, two-finger scroll to pan; double-click
  a region to zoom to it (smacks are 5–30 ms — they need zoom to mark precisely).
- **Class per region**: lip smack (default), mouth click, plosive, breath, **filled pause**
  ("uhm", "eh", Swedish "öh"), other — keyboard 1–6. Saved into the sidecar's `regions` as `{start_s, end_s, class}`.
- Keyboard flow: ↑/↓ next/previous clip, Return = mark clip as done, ⌫ on a clip = delete
  snapshot (with undo). *Implemented 2026-11-14 as ⌘⌫ (plain ⌫ deletes the selected region),
  the trash button in the header, or right-click a clip in the list → Move to Trash. "Undo" is
  the Finder Trash; the clip is also recorded in `deleted.txt` so importing another Mac's
  export doesn't bring it back.* Everything works without the mouse except dragging.
- Privacy note in the window footer: clips may contain other people's voices and never leave
  this Mac unless exported.

### Implemented (2026-11-13)

The hEARo window's **Review** tab (originally its own `Window("Review snapshots")`; merged
2026-11-14, see *The hEARo window*), opened from the popover's ⋯ menu; never opened at launch. Code: `HearoKit/Snapshots.swift` (list, sidecar round-trip, WAV decode, peaks,
`Annotation` edit operations, `Viewport`), `ReviewModel.swift`, `ClipPlayer.swift`,
`ReviewView.swift`, `ReviewTimeline.swift` (lanes, gestures, axis), `Spectrogram.swift` (STFT,
log scale, colour map), `SpectrogramCache.swift`; tests in `hEARoTests/SnapshotReviewTests.swift`
and `hEARoTests/SpectrogramTests.swift`.

- List (newest first): date/time, label, ✓ reviewed, "n marked", a sparkle icon when the clip
  has detector `proposals`. Re-read when the window appears and when the app becomes active.
- Waveform: mono mix, min/max per pixel column, normalised to the clip peak (gain ≤ 20×);
  `event_hint_s` shaded ("likely here"; old files: `[seconds−2.5, seconds−0.3]`), press
  marker dashed, proposals as dashed outlines in their class colour, regions as translucent
  fills (lip smack red, mouth click orange, plosive purple, breath teal, filled pause blue,
  other gray), playhead line, time axis in s/ms. Zoom down to 2 ms across the view.
- Proposals are also listed under the waveform with **Show**, **Accept (A)** and **False
  positive (X)** (accept = a region with the proposal's span and class; false positive = its
  span in the sidecar's `rejected`; each is a no-op twice and flips the other). A / X act on the
  shown proposal, else the first undecided one (marked ▸ in the list); the list header counts
  what is left to verify (2026-11-14, see *Auto-saved de-smack catches*).
- Region edges snap to the sample grid; the selection readout shows start–end (length) in ms.
- Saving: every finished edit (drag release, class, delete, accept, reviewed) merges
  `regions` (`{start_s, end_s, class}`, sorted by start) and `reviewed` into the sidecar,
  re-reading the file first and keeping every other key. No Save button.
- Playback: AVAudioEngine plays the original stereo WAV (engine runs only while playing).
  A selected region loops sample-accurately; otherwise playback runs from the playhead.
  Space over a list row opens that clip and plays it; over the rest of the window it
  plays/pauses the open clip.

#### Spectrogram (2026-11-14)

- **Layout:** the timeline is a stack of lanes sharing one `Viewport`, playhead and time axis:
  waveform on top (40 %), spectrogram below (60 %). Drag/click/zoom work identically on
  both lanes (the gestures live on the stack, `TimelineEditor`). The hint, press marker,
  regions and proposals are drawn on every lane by `TimelineMarks`; on the spectrogram,
  regions are a light tint with class-coloured bars along the top and bottom edges and
  proposals are dashed brackets, so a marked click stays visible instead of hiding under an
  edge line. The playhead is a single overlay above all lanes (the only thing redrawn at
  display rate while playing).
- **Toggle:** ⇧⌘S or the split-rectangle button in the header; remembered
  (`@AppStorage("review.showSpectrogram")`, on by default; ⇧⌘S works even with no clip
  open). Show/hide animates (the lane fades in, the waveform lane resizes) unless Reduce
  Motion is on. A resolution menu in the selection bar (remembered too):
  - *Time detail* — 10 ms window (512-pt FFT at 48 kHz), hop 1.3 ms: sharpest clicks/smacks.
  - *Balanced* (default) — 21 ms (1024-pt), hop 2.7 ms.
  - *Frequency detail* — 85 ms (4096-pt), hop 10.7 ms: separates 50/60 Hz hum and voice
    harmonics.
- **Maths:** mono mix, Hann window, `vDSP_fft_zrip`, power in dB re a full-scale sine;
  frames are centred on their sample (zero-padded at the clip edges). Log frequency axis
  30 Hz – min(20 kHz, Nyquist) with ticks 50 · 100 · 200 · 500 · 1k · 2k · 5k · 10k · 20k (labels
  skipped when crowded); 320 pixel rows, high rows take the loudest bin in their band (narrow
  lines survive), low rows interpolate between bins. Colour: magma (perceptually uniform,
  dark→pale yellow), 80 dB range with the top at the clip's loudest bin (never below −120 dB,
  so silence renders as the floor). The image is dark in both appearances; frequency labels
  are white on a 65 % black chip (≥ 4.5:1 even over the palest yellow). DC/Nyquist bins
  are scaled like the rest (zrip packs them at twice the amplitude).
- **Performance:** computed off the main thread per clip and resolution
  (`SpectrogramCache`: the last 6 overviews and, separately, the last 3 deep-zoom details,
  so zooming around never evicts the overview on screen; computation is cancelled when the
  clip or resolution changes; everything is dropped when the window closes).
  8 s of 48 kHz audio (STFT + image) takes ≈ 14 ms in release and ≈ 0.3 s in a debug build
  (`SpectrogramTests.eightSecondClipIsFast`). The view draws the same cached `CGImage` into
  the rect its time extent maps to (clipped by the lane), so zoom/pan stays smooth.
  **Deep zoom:** when overview columns would be more than 2 device pixels wide (and the
  window is short enough for a finer hop to help: under fftSize/32 samples per pixel), a
  sharper render of the visible stretch (hop = largest power of two ≤ samples per pixel,
  aligned to 256-frame blocks so panning doesn't recompute) replaces it ~80 ms after the
  gesture settles. Meanwhile the previous detail is shown if it covers the view, else the
  overview as nearest-neighbour blocks, honest about its time resolution.
- **Accessibility:** the spectrogram lane is its own element ("Spectrogram", value describes
  the frequency range and what clicks, hum and sibilance look like, or "Computing"); the
  resolution menu is labelled "Spectrogram resolution".
- **Extending:** the processing inspector sits beside `detail` in `ReviewView` (the timeline
  takes whatever width it is given); new lanes go in `TimelineEditor.lanes`, and overlay
  layers (like the per-module colour bands in `WaveformLane`) are drawn per lane between
  `TimelineMarks.under` and `.over`, all from the shared `model.viewport`.
- The STFT window still smears a click over ±½ window in time (≈ ±10 ms at *Balanced*):
  mark exact edges on the waveform, or switch to *Time detail*.

#### Processing inspector and A/B (human request 2026-11-14)

"A right panel to select/deselect the modules on the snapshot together with the advanced
settings to see how the audio is with/without (so have the bypass also)."

- **Panel:** a 320 pt trailing column beside the timeline (`ReviewInspector.swift`), shown by
  default; the sidebar button at the end of the clip header or **⌥⌘I** hides/shows it
  (remembered, `@AppStorage("review.showInspector")`; slides unless Reduce Motion is on).
  The window's minimum width grows to 1100 pt while it is shown. Contents, top down:
  **Bypass** switch ("Hearing the original" / "Hearing the processed clip"), render status
  ("Rendering…", "Rendered in 42 ms · 11 events", or the error), **Use my live settings** and
  **Apply these to live** (disabled while identical), then every module as in the
  Settings tab — swatch, on/off, every parameter, Reset — in a compact layout (name above
  a full-width slider).
- **Trial settings:** the panel works on a copy that starts from the live settings (and is
  refreshed from live when the window opens, unless you were mid-experiment). Edits never
  touch the live sound until *Apply these to live*, which applies them to the engine and saves
  them like any other change.
- **Rendering:** every edit or newly opened clip re-renders through the Rust chain
  (`render_clip`, `ClipRenderer`) off the main thread, 150 ms after the last request. The FFI
  render can't be interrupted, so **at most one runs**: requests that arrive meanwhile queue
  as one follow-up render with the newest settings, and a result that is no longer the newest
  request is discarded (holding ↓ through the clip list or dragging a slider never piles up
  renders). The processed WAV is decoded on the same background task; the main thread only
  copies samples into the player's buffer. Renders go to temporary WAVs in
  `$TMPDIR/hEARo-review/` (only the newest is kept; all are deleted when the window closes).
  8 s of audio renders in ≈ 80 ms in release.
- **A/B:** **B** (or the switch) flips between the original and the processed clip. The player
  holds both buffers, which line up sample for sample, and switches **at the same position**,
  keeping the span and looping (a looping region resumes mid-loop, then repeats the whole
  region; `ClipPlayer.plan`). A render that finishes while playing is swapped in the same way.
  B ignores ⌘/⌥/⌃ so it never clashes with menu shortcuts and works wherever focus is; Space,
  arrows, Return and ⌫ go to a focused inspector control (Full Keyboard Access) instead of the
  timeline.
- **Waveform overlay:** the waveform lane colours each pixel column outside-in from the render
  (`RenderResult.bands`, 500 columns/s = 2 ms; the loudest input column in the pixel, its
  output peak, and each module's deepest cut) exactly like the live waveform, and marks the
  render's events as short ticks along the top edge in the module's colour. Bypass hides both,
  so the picture always matches what you hear. The spectrogram lane is unchanged (module ticks
  would collide with the region bars drawn there). VoiceOver: the waveform's value adds
  "removed: De-ess 11, …".

Deviations / not yet done:

- **Return marks the clip reviewed and opens the next one** (fast flow); the "Reviewed"
  toggle in the header undoes it.
- **Reviewed = exhaustive, said in the UI (2026-11-15, ADR 0017):** a caption under the
  Reviewed toggle reads *Mark every mouth noise first* (then *Every mouth noise marked*); its
  tooltip and VoiceOver hint explain why — unmarked audio becomes clean training data — and
  name Return (`ReviewedHint`). A caption rather than an alert or a check on Return: the fast
  flow stays one key, and the promise is visible where the decision is made. The caption is
  hidden from VoiceOver, which hears the full hint on the toggle instead. It keeps the width
  of the longer text in both states, so the buttons beside it (Move to Trash) don't move when
  the toggle flips.
- Region classes are stored snake_case (`lip_smack`, `mouth_click`, `plosive`, `breath`,
  `filled_pause`, `other`), matching the `hearo dataset` proposal spelling.
- Processed A/B is **B** (a toggle at the same position), not ⌥Space as first sketched.
- Not yet: ⌫ on a clip to delete the snapshot (with undo), source app in the list
  (sidecars don't record it; `source` is "cli"/"app"), Delete on a *hovered* (unselected)
  region, unknown keys inside individual region objects (regions are rewritten as the three
  keys above).
