//! The engine supervisor (ADR 0014): keeps the live engine running through everyday
//! disruptions — a new output device or sample rate, a failed start, an IOProc that stops
//! delivering audio, system sleep — and fails to passthrough when it can't.
//!
//! It is pure control logic over an [`AudioBackend`] (the OS-facing part: build/tear down tap +
//! aggregate + IOProc, read the default output, count callbacks, ask whether tapped apps are
//! playing) so every policy here is unit-tested with [`fake::FakeBackend`] and an injected
//! clock (`now` is always a parameter). The real backend is `LiveBackend` (macOS).
//!
//! Fail-safe by construction: whenever no engine exists, no tap exists, and macOS plays every
//! app normally (unprocessed, never silent). Dropping an engine tears it down; the supervisor
//! always drops the old engine before starting a new one.

use std::collections::BTreeMap;
use std::time::{Duration, Instant, SystemTime};

use crate::Controls;

/// Device changes come in bursts (default output, device list, sample rate); act once settled.
pub const SETTLE: Duration = Duration::from_millis(250);
/// How often the supervising thread ticks without a notification (output check + watchdog).
pub const TICK: Duration = Duration::from_millis(500);
/// First wait after a failed start; doubles per failure up to [`RETRY_MAX`].
pub const RETRY_MIN: Duration = Duration::from_secs(3);
/// Longest wait between start attempts.
pub const RETRY_MAX: Duration = Duration::from_secs(30);
/// Tapped apps report output but the IOProc gets no callbacks (or only digital silence) for
/// this long → rebuild. The tap is `MutedWhenTapped`, so a dead IOProc means the user hears
/// silence: this must be short.
pub const STALL: Duration = Duration::from_secs(2);
/// Watchdog rebuilds for "no callbacks" in one episode before giving up (tearing down so macOS
/// plays audio unprocessed). Stall needed before rebuild *n* is `STALL × n`.
pub const DEAD_REBUILDS: u32 = 2;
/// "Only digital silence" can be legitimate (a call where nobody talks), so it never gives up;
/// rebuilds back off `STALL × 2ⁿ` up to this. A rebuild while everything is silent is inaudible.
pub const SILENT_BACKOFF_MAX: Duration = Duration::from_secs(60);
/// Wall clock ran ahead of the monotonic clock (which stops during sleep) by this much → the
/// system slept.
pub const SLEEP_GAP: Duration = Duration::from_secs(3);
/// The supervising thread didn't run for this long (monotonic) → treat like a wake too.
pub const LONG_GAP: Duration = Duration::from_secs(30);

/// Shown in `EngineStatus.problem` after the watchdog gave up.
pub const GAVE_UP: &str = "hEARo stopped processing: the system audio tap stopped delivering sound while apps \
                           were playing, and rebuilding it didn't help. Audio plays normally, unprocessed. \
                           Turn hEARo off and on (or switch output) to try again.";

/// IOProc activity counters of one engine (monotonic since that engine started).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Activity {
    /// IOProc callbacks.
    pub callbacks: u64,
    /// Callbacks whose tap input was digital silence.
    pub silent: u64,
}

/// The OS-facing operations the supervisor needs. Dropping an `Engine` must stop its IOProc
/// and destroy its aggregate and tap.
pub trait AudioBackend {
    /// A running tap + aggregate + IOProc.
    type Engine;
    /// Why a start failed.
    type Error: std::fmt::Display;
    /// Build tap + aggregate on the current default output and start the IOProc.
    fn start(&mut self) -> Result<Self::Engine, Self::Error>;
    /// The default output (device or nominal rate) is no longer the one `engine` plays to.
    fn output_changed(&mut self, engine: &Self::Engine) -> bool;
    /// The current default output (UID, nominal rate); `None` if it can't be read.
    fn default_output(&mut self) -> Option<(String, f64)>;
    /// The engine's lock-free controls (to re-apply bypass/toggles/params after a rebuild).
    fn controls<'a>(&self, engine: &'a Self::Engine) -> &'a Controls;
    /// Callback counters (must not reset anything the UI reads).
    fn activity(&self, engine: &Self::Engine) -> Activity;
    /// Processes the tap should capture are reporting audio output right now. `false` when
    /// unknown (e.g. permission prompt pending): the watchdog only acts on a clear signal.
    fn tapped_output_running(&mut self, engine: &Self::Engine) -> bool;
}

/// What the user set; re-applied to every new engine.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    /// Global bypass (A/B).
    pub bypass: bool,
    /// Per chain slot.
    pub enabled: Vec<bool>,
    /// (slot, param) → value.
    pub params: BTreeMap<(usize, usize), f32>,
}

impl Settings {
    /// All `slots` enabled, no bypass, module default params.
    pub fn all_enabled(slots: usize) -> Self {
        Self { enabled: vec![true; slots], ..Self::default() }
    }

    fn apply(&self, c: &Controls) {
        c.set_bypass(self.bypass);
        for (i, on) in self.enabled.iter().enumerate() {
            c.set_enabled(i, *on);
        }
        for (&(slot, p), &v) in &self.params {
            c.set_param(slot, p, v);
        }
    }
}

/// What happened this session (diagnostics, tests).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    /// Successful starts (each new engine).
    pub starts: u32,
    /// Failed start attempts.
    pub start_failures: u32,
    /// Rebuilds because the output device or rate changed.
    pub device_switches: u32,
    /// Rebuilds by the watchdog (stalled IOProc while apps play).
    pub watchdog_rebuilds: u32,
    /// Rebuilds after a system wake.
    pub wake_rebuilds: u32,
    /// Times the watchdog gave up (→ passthrough + problem).
    pub give_ups: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StallKind {
    /// No callbacks at all.
    Dead,
    /// Callbacks, but only digital silence.
    Silent,
}

#[derive(Debug, Clone, Copy)]
struct Stall {
    since: Instant,
    kind: StallKind,
}

/// Keeps one engine running while the user wants it. Call [`tick`](Self::tick) every
/// [`TICK`] and right after a (settled) device notification, and [`woke`](Self::woke) after
/// system sleep.
///
/// A device notification by itself doesn't end a retry wait or a give-up: building and
/// destroying our own aggregate changes the device list too, which would defeat both. Only a
/// different default output (UID or rate) does — that's checked on every tick.
#[derive(Debug)]
pub struct Supervisor<B: AudioBackend> {
    backend: B,
    engine: Option<B::Engine>,
    settings: Settings,
    want_running: bool,
    gave_up: bool,
    problem: Option<String>,
    retry_at: Option<Instant>,
    retry_delay: Duration,
    /// Default output when the last start failed or the watchdog gave up.
    failed_on: Option<(String, f64)>,
    last: Activity,
    stall: Option<Stall>,
    dead_rebuilds: u32,
    silent_rebuilds: u32,
    counters: Counters,
}

impl<B: AudioBackend> Supervisor<B> {
    /// Not running until [`start`](Self::start).
    pub fn new(backend: B, settings: Settings) -> Self {
        Self {
            backend,
            engine: None,
            settings,
            want_running: false,
            gave_up: false,
            problem: None,
            retry_at: None,
            retry_delay: RETRY_MIN,
            failed_on: None,
            last: Activity::default(),
            stall: None,
            dead_rebuilds: 0,
            silent_rebuilds: 0,
            counters: Counters::default(),
        }
    }

    /// The user switched processing on. Starts now; on failure keeps retrying (with backoff)
    /// and returns the error. Also the way out of a give-up.
    pub fn start(&mut self, now: Instant) -> Result<(), B::Error> {
        self.want_running = true;
        self.gave_up = false;
        if self.engine.is_some() {
            return Ok(());
        }
        self.retry_delay = RETRY_MIN;
        self.reset_watchdog();
        self.try_start(now)
    }

    /// The user switched processing off: tear down; macOS plays every app normally.
    pub fn stop(&mut self) {
        self.want_running = false;
        self.gave_up = false;
        self.problem = None;
        self.retry_at = None;
        self.engine = None;
        self.reset_watchdog();
    }

    /// The system woke from sleep: whatever survived it, rebuild tap + aggregate now.
    pub fn woke(&mut self, now: Instant) {
        if !self.want_running {
            return;
        }
        self.counters.wake_rebuilds += 1;
        self.gave_up = false;
        self.engine = None; // tear down first
        self.retry_delay = RETRY_MIN;
        self.reset_watchdog();
        let _ = self.try_start(now); // failure → problem + retry with backoff
    }

    /// Follow the output, retry failed starts, run the watchdog.
    pub fn tick(&mut self, now: Instant) {
        if !self.want_running {
            return;
        }
        match &self.engine {
            Some(e) if self.backend.output_changed(e) => {
                self.engine = None; // the old tap and aggregate must be gone before new ones
                self.counters.device_switches += 1;
                self.retry_delay = RETRY_MIN;
                self.reset_watchdog();
                let _ = self.try_start(now);
            }
            Some(_) => self.watchdog(now),
            None if self.backend.default_output() != self.failed_on => {
                // A different output (AirPods connected, …): try it at once, even after a give-up.
                self.gave_up = false;
                self.retry_delay = RETRY_MIN;
                self.reset_watchdog();
                let _ = self.try_start(now);
            }
            None if self.gave_up || self.retry_at.is_some_and(|t| now < t) => {}
            None => {
                let _ = self.try_start(now);
            }
        }
    }

    fn try_start(&mut self, now: Instant) -> Result<(), B::Error> {
        debug_assert!(self.engine.is_none(), "old engine must be torn down first");
        match self.backend.start() {
            Ok(engine) => {
                self.settings.apply(self.backend.controls(&engine));
                self.engine = Some(engine);
                self.problem = None;
                self.retry_at = None;
                self.retry_delay = RETRY_MIN;
                self.last = Activity::default();
                self.counters.starts += 1;
                Ok(())
            }
            Err(e) => {
                self.failed_on = self.backend.default_output();
                self.problem = Some(e.to_string());
                self.retry_at = Some(now + self.retry_delay);
                self.retry_delay = (self.retry_delay * 2).min(RETRY_MAX);
                self.counters.start_failures += 1;
                Err(e)
            }
        }
    }

    fn reset_watchdog(&mut self) {
        self.stall = None;
        self.dead_rebuilds = 0;
        self.silent_rebuilds = 0;
    }

    fn watchdog(&mut self, now: Instant) {
        let Some(engine) = &self.engine else { return };
        let a = self.backend.activity(engine);
        let calls = a.callbacks.saturating_sub(self.last.callbacks);
        let silent = a.silent.saturating_sub(self.last.silent);
        self.last = a;
        // Sound flowed, or nothing is playing (idle: the IOProc only runs while a tapped app
        // plays, ADR 0010) → healthy.
        if calls > silent || !self.backend.tapped_output_running(engine) {
            self.reset_watchdog();
            return;
        }
        let kind = if calls == 0 { StallKind::Dead } else { StallKind::Silent };
        let stall = match self.stall {
            Some(s) if s.kind == kind => s,
            _ => *self.stall.insert(Stall { since: now, kind }),
        };
        let needed = match kind {
            StallKind::Dead => STALL * self.dead_rebuilds.max(1),
            StallKind::Silent => (STALL * 2u32.saturating_pow(self.silent_rebuilds.min(16))).min(SILENT_BACKOFF_MAX),
        };
        if now.saturating_duration_since(stall.since) < needed {
            return;
        }
        if kind == StallKind::Dead && self.dead_rebuilds >= DEAD_REBUILDS {
            self.engine = None;
            self.gave_up = true;
            self.failed_on = self.backend.default_output();
            self.problem = Some(GAVE_UP.into());
            self.counters.give_ups += 1;
            self.reset_watchdog();
            return;
        }
        match kind {
            StallKind::Dead => self.dead_rebuilds += 1,
            StallKind::Silent => self.silent_rebuilds += 1,
        }
        self.counters.watchdog_rebuilds += 1;
        self.engine = None;
        self.stall = Some(Stall { since: now, kind });
        let _ = self.try_start(now);
    }

    /// The running engine, if any.
    pub fn engine(&self) -> Option<&B::Engine> {
        self.engine.as_ref()
    }

    /// The backend.
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// The backend, mutably (tests script the fake through this).
    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    /// The user wants processing on.
    pub fn wants_running(&self) -> bool {
        self.want_running
    }

    /// Why hEARo isn't processing although switched on (audio then plays unprocessed).
    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// Session counters.
    pub fn counters(&self) -> Counters {
        self.counters
    }

    /// Current settings.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Global bypass; remembered across rebuilds.
    pub fn set_bypass(&mut self, on: bool) {
        self.settings.bypass = on;
        if let Some(e) = &self.engine {
            self.backend.controls(e).set_bypass(on);
        }
    }

    /// Replace enabled flags and parameters (bypass is kept); applied now and on every rebuild.
    pub fn set_modules(&mut self, enabled: Vec<bool>, params: BTreeMap<(usize, usize), f32>) {
        self.settings.enabled = enabled;
        self.settings.params = params;
        if let Some(e) = &self.engine {
            self.settings.apply(self.backend.controls(e));
        }
    }

    /// Enable/disable chain slot `slot`; remembered across rebuilds.
    pub fn set_enabled(&mut self, slot: usize, on: bool) {
        if let Some(v) = self.settings.enabled.get_mut(slot) {
            *v = on;
        }
        if let Some(e) = &self.engine {
            self.backend.controls(e).set_enabled(slot, on);
        }
    }

    /// Set a module parameter; remembered across rebuilds.
    pub fn set_param(&mut self, slot: usize, param: usize, value: f32) {
        self.settings.params.insert((slot, param), value);
        if let Some(e) = &self.engine {
            self.backend.controls(e).set_param(slot, param, value);
        }
    }
}

/// Detects system sleep from the supervising thread's point of view: Rust's `Instant` on macOS
/// (`CLOCK_UPTIME_RAW`) stops while the system sleeps, the wall clock doesn't.
#[derive(Debug, Default)]
pub struct SleepDetector {
    last: Option<(Instant, SystemTime)>,
}

impl SleepDetector {
    /// Record a tick; `true` if the system slept (or this thread was frozen for
    /// [`LONG_GAP`]) since the previous one. A wall clock set backwards is ignored.
    pub fn observe(&mut self, mono: Instant, wall: SystemTime) -> bool {
        let Some((m0, w0)) = self.last.replace((mono, wall)) else { return false };
        let dm = mono.saturating_duration_since(m0);
        let Ok(dw) = wall.duration_since(w0) else { return false };
        dw.saturating_sub(dm) >= SLEEP_GAP || dm >= LONG_GAP
    }
}

pub mod fake;

#[cfg(test)]
mod tests;
