//! Auto-saving de-smack catches (ADR 0018): which catches become which clip, and when.
//!
//! Pure policy, no I/O and no clocks of its own: the control thread feeds it the chain's events
//! ([`AutoCapture::observe`]), asks what is due once the capture ring has the audio after a
//! burst ([`AutoCapture::due`]), cuts that window from the capture ring, saves it
//! ([`crate::snapshots::save_auto`]) and reports back ([`AutoCapture::saved`]).
//!
//! A clip is [`SNAPSHOT_SECS`] long and ends [`POST_ROLL_S`] after the last catch of a burst;
//! catches within [`GROUP_SPAN_S`] of the burst's first one share it. The daily limit is a rolling
//! 24 h window over save times (seeded from the files on disk), so restarts and time-zone
//! changes don't reset it.

use std::collections::VecDeque;

use hearo_dsp::{Event, ModuleId};

use crate::snapshots::SNAPSHOT_SECS;

/// Default daily limit.
pub const DEFAULT_DAILY_CAP: u32 = 250;
/// Largest daily limit the settings accept.
pub const MAX_DAILY_CAP: u32 = 2000;
/// Default age (days) after which untouched auto clips are deleted.
pub const DEFAULT_RETENTION_DAYS: u32 = 31;
/// Longest retention the settings accept.
pub const MAX_RETENTION_DAYS: u32 = 365;
/// Audio kept after the last catch of a burst (what follows a smack matters for labelling).
pub const POST_ROLL_S: f64 = 3.0;
/// Catches starting within this long of a burst's first catch share its clip (it still has
/// ≥ 0.3 s before the first catch: 8 s − 3 s post-roll − 4.5 s − a ≤ 0.2 s catch).
pub const GROUP_SPAN_S: f64 = 4.5;
/// Bursts waiting for their post-roll (bounded: a flood beyond this is dropped).
const MAX_PENDING: usize = 16;
const DAY_S: u64 = 86_400;

/// The user's auto-save settings (persisted by the app; clamp before use).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoCaptureSettings {
    /// Off by default.
    pub enabled: bool,
    /// Clips per rolling 24 h.
    pub daily_cap: u32,
    /// Untouched auto clips older than this many days are deleted.
    pub retention_days: u32,
}

impl Default for AutoCaptureSettings {
    fn default() -> Self {
        Self { enabled: false, daily_cap: DEFAULT_DAILY_CAP, retention_days: DEFAULT_RETENTION_DAYS }
    }
}

impl AutoCaptureSettings {
    /// Limits within their legal ranges (≥ 1).
    pub fn clamped(self) -> Self {
        Self {
            enabled: self.enabled,
            daily_cap: self.daily_cap.clamp(1, MAX_DAILY_CAP),
            retention_days: self.retention_days.clamp(1, MAX_RETENTION_DAYS),
        }
    }
}

/// A clip to cut from the capture ring: frames `start..start + frames` (the chain's input clock)
/// and the catches inside it.
#[derive(Debug, Clone, PartialEq)]
pub struct DueClip {
    /// First frame.
    pub start: u64,
    /// Length in frames.
    pub frames: usize,
    /// The de-smack events in this clip.
    pub catches: Vec<Event>,
}

impl DueClip {
    /// The catches as proposals relative to the clip start: (start s, end s, dB removed).
    pub fn proposals(&self, sample_rate: f64) -> Vec<crate::snapshots::Catch> {
        self.catches
            .iter()
            .map(|e| {
                let start_s = e.start_sample.saturating_sub(self.start) as f64 / sample_rate;
                crate::snapshots::Catch {
                    start_s,
                    end_s: start_s + e.duration_samples as f64 / sample_rate,
                    db_removed: e.energy_removed_db,
                }
            })
            .collect()
    }
}

#[derive(Debug)]
struct Burst {
    first: u64,
    last_end: u64,
    catches: Vec<Event>,
}

/// Groups catches into clips and enforces the daily limit.
#[derive(Debug, Default)]
pub struct AutoCapture {
    pending: VecDeque<Burst>,
    /// Unix seconds of saves in the last 24 h (oldest first).
    saves: VecDeque<u64>,
    /// Clips not saved because the daily limit was reached (this session).
    pub skipped_by_cap: u64,
}

impl AutoCapture {
    /// `previous_saves`: unix times of auto clips already on disk (any order).
    pub fn new(mut previous_saves: Vec<u64>) -> Self {
        previous_saves.sort_unstable();
        Self { saves: previous_saves.into(), ..Self::default() }
    }

    /// Take note of new chain events (only de-smack catches matter).
    pub fn observe(&mut self, events: &[Event], sample_rate: f64) {
        let span = (GROUP_SPAN_S * sample_rate) as u64;
        for e in events.iter().filter(|e| e.module == ModuleId::DeSmack) {
            let end = e.start_sample + u64::from(e.duration_samples);
            if let Some(b) = self.pending.back_mut().filter(|b| e.start_sample <= b.first + span) {
                b.last_end = b.last_end.max(end);
                b.catches.push(*e);
            } else if self.pending.len() < MAX_PENDING {
                self.pending.push_back(Burst { first: e.start_sample, last_end: end, catches: vec![*e] });
            }
        }
    }

    /// The next clip whose post-roll has been captured (`captured` = frames in the capture
    /// ring), unless the daily limit is reached — then it's dropped and counted.
    pub fn due(&mut self, captured: u64, sample_rate: f64, daily_cap: u32, now_unix: u64) -> Option<DueClip> {
        let post = (POST_ROLL_S * sample_rate) as u64;
        let frames = (SNAPSHOT_SECS * sample_rate) as usize;
        while self.pending.front().is_some_and(|b| b.last_end + post <= captured) {
            let b = self.pending.pop_front()?;
            if self.saved_last_24h(now_unix) >= daily_cap {
                self.skipped_by_cap += 1;
                continue;
            }
            let end = b.last_end + post;
            let start = end.saturating_sub(frames as u64);
            // Shorter than 8 s right after a start/rebuild: only what the ring has.
            return Some(DueClip { start, frames: (end - start) as usize, catches: b.catches });
        }
        None
    }

    /// A clip was written at `now_unix`.
    pub fn saved(&mut self, now_unix: u64) {
        self.saves.push_back(now_unix);
    }

    /// Clips saved in the 24 h before `now_unix`.
    pub fn saved_last_24h(&mut self, now_unix: u64) -> u32 {
        while self.saves.front().is_some_and(|&t| t + DAY_S <= now_unix) {
            self.saves.pop_front();
        }
        self.saves.len() as u32
    }

    /// The engine was rebuilt: its sample clock restarted, so pending positions are meaningless.
    pub fn reset(&mut self) {
        self.pending.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hearo_dsp::EventKind;

    const SR: f64 = 48_000.0;

    fn catch(at_s: f64) -> Event {
        Event {
            module: ModuleId::DeSmack,
            kind: EventKind::Smack,
            start_sample: (at_s * SR) as u64,
            duration_samples: 960, // 20 ms
            energy_removed_db: 18.0,
        }
    }

    fn at(s: f64) -> u64 {
        (s * SR) as u64
    }

    #[test]
    fn waits_for_the_post_roll_then_cuts_8_s_ending_3_s_after_the_catch() {
        let mut ac = AutoCapture::new(vec![]);
        ac.observe(&[catch(10.0)], SR);
        assert_eq!(ac.due(at(12.9), SR, 250, 0), None, "post-roll not captured yet");
        let clip = ac.due(at(13.1), SR, 250, 0).expect("due");
        assert_eq!(clip.frames, at(8.0) as usize);
        assert_eq!(clip.start, at(10.02 + 3.0) - at(8.0));
        let p = clip.proposals(SR);
        assert_eq!(p.len(), 1);
        assert!((p[0].start_s - 4.98).abs() < 1e-3 && (p[0].end_s - 5.0).abs() < 1e-3, "{p:?}");
        assert_eq!(ac.due(at(20.0), SR, 250, 0), None, "a clip is handed out once");
    }

    #[test]
    fn close_catches_share_a_clip_far_ones_get_their_own() {
        let mut ac = AutoCapture::new(vec![]);
        ac.observe(&[catch(10.0), catch(11.0)], SR);
        ac.observe(&[catch(14.4)], SR); // within 4.5 s of the first
        ac.observe(&[catch(14.6)], SR); // beyond: a new burst
        let first = ac.due(at(30.0), SR, 250, 0).unwrap();
        assert_eq!(first.catches.len(), 3);
        let p = first.proposals(SR);
        assert!(p.iter().all(|c| c.start_s >= 0.3 && c.end_s <= 8.0 - 2.9), "all inside with margins: {p:?}");
        assert_eq!(ac.due(at(30.0), SR, 250, 0).unwrap().catches.len(), 1);
    }

    #[test]
    fn ignores_other_modules() {
        let mut ac = AutoCapture::new(vec![]);
        let mut e = catch(1.0);
        e.module = ModuleId::DeEss;
        ac.observe(&[e], SR);
        assert_eq!(ac.due(at(60.0), SR, 250, 0), None);
    }

    #[test]
    fn daily_cap_is_a_rolling_24_h_window_seeded_from_disk() {
        let now = 1_000_000;
        // Two saves already on disk in the last day, one older.
        let mut ac = AutoCapture::new(vec![now - 100, now - 90_000, now - 50]);
        assert_eq!(ac.saved_last_24h(now), 2);
        ac.observe(&[catch(1.0)], SR);
        assert_eq!(ac.due(at(10.0), SR, 2, now), None, "cap reached: dropped");
        assert_eq!(ac.skipped_by_cap, 1);
        // A day later there is room again.
        ac.observe(&[catch(20.0)], SR);
        assert!(ac.due(at(30.0), SR, 2, now + DAY_S).is_some());
        ac.saved(now + DAY_S);
        assert_eq!(ac.saved_last_24h(now + DAY_S), 1);
    }

    /// Right after a start or a rebuild the ring holds less than 8 s: the clip is shorter
    /// instead of being cut from frames that don't exist (review: it used to vanish).
    #[test]
    fn a_catch_in_the_first_seconds_gets_a_shorter_clip() {
        let mut ac = AutoCapture::new(vec![]);
        ac.observe(&[catch(1.0)], SR);
        let clip = ac.due(at(4.1), SR, 250, 0).expect("due");
        assert_eq!(clip.start, 0);
        assert_eq!(clip.frames as u64, at(1.02) + at(3.0));
        assert!((clip.proposals(SR)[0].start_s - 1.0).abs() < 1e-3);
    }

    #[test]
    fn cap_boundary_is_exactly_24_h() {
        let now = 1_000_000;
        let mut ac = AutoCapture::new(vec![now - DAY_S, now - DAY_S + 1]);
        assert_eq!(ac.saved_last_24h(now), 1, "a save exactly 24 h old no longer counts");
    }

    #[test]
    fn reset_forgets_pending_bursts() {
        let mut ac = AutoCapture::new(vec![]);
        ac.observe(&[catch(1.0)], SR);
        ac.reset();
        assert_eq!(ac.due(at(60.0), SR, 250, 0), None);
    }

    #[test]
    fn a_flood_of_bursts_is_bounded() {
        let mut ac = AutoCapture::new(vec![]);
        let evs: Vec<Event> = (0..100).map(|i| catch(5.0 * i as f64)).collect();
        ac.observe(&evs, SR);
        let mut n = 0;
        while ac.due(u64::MAX / 2, SR, 10_000, 0).is_some() {
            n += 1;
        }
        assert_eq!(n, MAX_PENDING);
    }

    #[test]
    fn settings_clamp_into_range() {
        let s = AutoCaptureSettings { enabled: true, daily_cap: 0, retention_days: 10_000 }.clamped();
        assert_eq!((s.daily_cap, s.retention_days), (1, MAX_RETENTION_DAYS));
        assert_eq!(
            AutoCaptureSettings::default(),
            AutoCaptureSettings { enabled: false, daily_cap: 250, retention_days: 31 }
        );
    }
}
