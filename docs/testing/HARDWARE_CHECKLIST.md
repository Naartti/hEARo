# Hardware checklist

Human-assisted; run with `just test-hw` from M2 on. Record results in the log at the bottom.

- [ ] Built-in speakers / wired headphones / AirPods: start, switch output mid-playback,
      AirPods sleep/wake, disconnect/reconnect.
- [ ] Teams call, Slack huddle (app and browser), YouTube in Chrome and Safari, Podcasts/Spotify
      podcast: processed, toggles audible, latency acceptable.
- [ ] Exclusions: Spotify, Music, Logic untouched.
- [ ] Fail-safe: `kill -9` → normal audio within 1 s.
- [ ] Permission denied → clear UI state, never silence.
- [ ] 2-hour soak: no zero-buffer stalls, or the watchdog recovers.
- [ ] `just test-hw` → `hw_watchdog`: a quiet 440 Hz tone plays for 7 s; the watchdog must not
      rebuild a healthy engine (0 rebuilds, no problem).
- [ ] Sleep/wake (ADR 0014): in the app, play a podcast/YouTube, close the lid (or Apple menu →
      Sleep) for ≥ 1 min, wake, press play: processed audio resumes within ~1 s, no silence,
      no problem shown. Repeat with AirPods as output (they may reconnect after wake).
- [ ] Watchdog false positives: a Teams/Zoom call where nobody talks for a minute and a paused
      YouTube tab — no audible glitches, hEARo stays on, no problem shown.

## Log

- 2026-11-14 — `just test-hw` (automated): engine follows the default output AirPods Pro
  (48 kHz) → MacBook Pro Speakers (44.1 kHz) → back, both rebuilds within 0.8 s, no problem
  reported. Still human: mid-playback switch by ear, AirPods sleep/wake, Sonos (AirPlay).
