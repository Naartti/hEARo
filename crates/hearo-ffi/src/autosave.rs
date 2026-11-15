//! Auto-saving de-smack catches (ADR 0018): the settings Swift sets and the step the
//! `hearo-supervisor` thread runs every tick. The policy itself is `hearo_engine::autocapture`.

use std::time::{Duration, Instant, SystemTime};

use hearo_engine::autocapture::{self, AutoCapture, AutoCaptureSettings};
use hearo_engine::snapshots::{self, Context};

use crate::{HearoEngine, Inner};

/// How often untouched old auto clips are swept — and how long a *changed* retention waits
/// before it deletes anything, so a value typed by mistake can be corrected first.
const EXPIRY_EVERY: Duration = Duration::from_secs(3600);

/// The snapshot folder. Never the real one in unit tests: they must not read or sweep the
/// developer's dataset (review finding).
fn snapshot_dir() -> Option<std::path::PathBuf> {
    if cfg!(test) { None } else { snapshots::default_dir() }
}

/// The user's auto-save settings (the app persists them; hEARo clamps them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct AutoCaptureConfig {
    /// Save a clip around every de-smack catch. Off by default.
    pub enabled: bool,
    /// At most this many clips per rolling 24 hours.
    pub daily_cap: u32,
    /// Delete auto-saved clips you never touched after this many days.
    pub retention_days: u32,
}

impl From<AutoCaptureSettings> for AutoCaptureConfig {
    fn from(s: AutoCaptureSettings) -> Self {
        Self { enabled: s.enabled, daily_cap: s.daily_cap, retention_days: s.retention_days }
    }
}

impl From<AutoCaptureConfig> for AutoCaptureSettings {
    fn from(c: AutoCaptureConfig) -> Self {
        Self { enabled: c.enabled, daily_cap: c.daily_cap, retention_days: c.retention_days }
    }
}

/// Defaults and ranges for the settings UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct AutoCaptureLimits {
    /// Default daily limit.
    pub default_daily_cap: u32,
    /// Largest daily limit.
    pub max_daily_cap: u32,
    /// Default retention, days.
    pub default_retention_days: u32,
    /// Longest retention, days.
    pub max_retention_days: u32,
}

/// Defaults and ranges of [`AutoCaptureConfig`] (minimum 1 for both numbers).
#[uniffi::export]
pub fn auto_capture_limits() -> AutoCaptureLimits {
    AutoCaptureLimits {
        default_daily_cap: autocapture::DEFAULT_DAILY_CAP,
        max_daily_cap: autocapture::MAX_DAILY_CAP,
        default_retention_days: autocapture::DEFAULT_RETENTION_DAYS,
        max_retention_days: autocapture::MAX_RETENTION_DAYS,
    }
}

/// Auto-save state kept in [`Inner`].
#[derive(Debug)]
pub(crate) struct AutoSave {
    pub settings: AutoCaptureSettings,
    pub policy: AutoCapture,
    /// Engine generation (`Counters::starts`) the pending bursts belong to.
    generation: u32,
    /// Clips saved this session (the app flashes the icon when it grows).
    pub saved_session: u32,
    /// When the next expiry sweep may run; `None` until the app has handed over the user's
    /// settings (so a launch never sweeps with the default retention).
    next_sweep: Option<Instant>,
}

impl Default for AutoSave {
    fn default() -> Self {
        let previous = snapshot_dir().map(|d| snapshots::auto_save_times(&d)).unwrap_or_default();
        Self {
            settings: AutoCaptureSettings::default(),
            policy: AutoCapture::new(previous),
            generation: 0,
            saved_session: 0,
            next_sweep: None,
        }
    }
}

impl AutoSave {
    /// The app handed over settings: the first time (launch, stored values) sweeping may start
    /// now; after a change it waits [`EXPIRY_EVERY`].
    fn configured(&mut self, now: Instant) {
        self.next_sweep = Some(if self.next_sweep.is_none() { now } else { now + EXPIRY_EVERY });
    }

    /// The retention to sweep with, if a sweep is due (then the next one is an hour away).
    fn sweep_due(&mut self, now: Instant) -> Option<u32> {
        let due = self.next_sweep.is_some_and(|t| now >= t);
        if due {
            self.next_sweep = Some(now + EXPIRY_EVERY);
        }
        due.then_some(self.settings.retention_days)
    }
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// A clip cut from the capture ring, written outside the engine lock.
struct Job {
    left: Vec<f32>,
    right: Vec<f32>,
    sample_rate: f64,
    catches: Vec<snapshots::Catch>,
    ctx: Context,
}

impl Inner {
    /// Drain the engine's events (always, so the ring never fills) and, if auto-save is on,
    /// cut the next due clip. Runs under the lock; the file is written by the caller.
    fn auto_collect(&mut self, now_unix: u64) -> Option<Job> {
        let starts = self.sup.counters().starts;
        let mut events = Vec::new();
        let (sr, captured) = {
            let engine = self.engine()?;
            engine.drain_events(&mut events);
            (engine.info().aggregate.sample_rate, engine.captured_frames())
        };
        if starts != self.auto.generation {
            // A rebuilt engine restarted its sample clock: older positions are meaningless.
            self.auto.policy.reset();
            self.auto.generation = starts;
        }
        if !self.auto.settings.enabled {
            self.auto.policy.reset();
            return None;
        }
        self.auto.policy.observe(&events, sr);
        let clip = self.auto.policy.due(captured, sr, self.auto.settings.daily_cap, now_unix)?;
        // None if the audio already left the ring (e.g. a long stall); the clip is dropped.
        let (left, right) = self.engine()?.capture_window(clip.start, clip.frames)?;
        let ctx = Context {
            modules: self.modules.keys.clone(),
            source: "app-auto".into(),
            output_device: self.engine().map(|e| e.info().output.name.clone()).unwrap_or_default(),
            settings: Some(self.modules.live_settings()),
            ..Context::default()
        };
        Some(Job { left, right, sample_rate: sr, catches: clip.proposals(sr), ctx })
    }
}

impl HearoEngine {
    /// One supervisor tick of auto-save: collect under the lock, write without it, then count.
    pub(crate) fn auto_step(&self) {
        let now = unix_now();
        let (job, sweep) = {
            let mut inner = self.lock();
            let job = inner.auto_collect(now);
            let due = inner.auto.sweep_due(Instant::now());
            (job, due)
        };
        let Some(dir) = snapshot_dir() else { return };
        if let Some(days) = sweep {
            snapshots::expire_auto(&dir, days, now);
        }
        if let Some(j) = job
            && snapshots::save_auto(&dir, &j.left, &j.right, j.sample_rate, &j.ctx, &j.catches).is_ok()
        {
            let mut inner = self.lock();
            inner.auto.policy.saved(now);
            inner.auto.saved_session += 1;
        }
    }

    pub(crate) fn auto_saved_last_24h(inner: &mut Inner) -> u32 {
        inner.auto.policy.saved_last_24h(unix_now())
    }
}

#[uniffi::export]
impl HearoEngine {
    /// Turn auto-saving of de-smack catches on/off and set its limits (clamped). Also sweeps
    /// untouched auto clips older than the (new) retention.
    pub fn set_auto_capture(&self, config: AutoCaptureConfig) {
        let mut inner = self.lock();
        inner.auto.settings = AutoCaptureSettings::from(config).clamped();
        inner.auto.configured(Instant::now());
    }

    /// The current (clamped) auto-save settings.
    pub fn auto_capture(&self) -> AutoCaptureConfig {
        self.lock().auto.settings.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No sweep before the app handed over the user's settings; the first hand-over sweeps at
    /// once, later changes (a mistyped retention) only after an hour, then hourly.
    #[test]
    fn expiry_waits_for_settings_and_for_an_hour_after_a_change() {
        let t0 = Instant::now();
        let mut a = AutoSave::default();
        assert_eq!(a.sweep_due(t0), None, "never with the default retention at launch");
        a.settings.retention_days = 90;
        a.configured(t0);
        assert_eq!(a.sweep_due(t0), Some(90));
        assert_eq!(a.sweep_due(t0 + Duration::from_secs(60)), None, "hourly");
        a.settings.retention_days = 1; // typed by mistake
        a.configured(t0 + Duration::from_secs(120));
        assert_eq!(a.sweep_due(t0 + Duration::from_secs(1800)), None, "a change waits an hour");
        assert_eq!(a.sweep_due(t0 + Duration::from_secs(120) + EXPIRY_EVERY), Some(1));
    }
}
