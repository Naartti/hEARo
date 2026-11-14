//! # hearo-ffi
//!
//! The Swift app's control plane (ADR 0002). Coarse, non-real-time calls only: start/stop,
//! toggles, a status record polled by the UI, snapshots. Nothing here runs on the audio thread;
//! the engine's lock-free [`Controls`](hearo_engine::Controls) carry changes to it.

use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant, SystemTime};

use hearo_dsp::modules;
use hearo_engine::ca::permission::{self, Permission as CaPermission};
use hearo_engine::ca::procs;
use hearo_engine::ca::watch::OutputWatcher;
use hearo_engine::snapshots::{self, Context, Label};
use hearo_engine::supervisor::{self, Counters, Settings, SleepDetector, Supervisor};
use hearo_engine::{DEFAULT_EXCLUDES, EngineConfig, EngineError, LiveBackend, LiveEngine, StatsSnapshot};

uniffi::setup_scaffolding!();

mod render;
mod settings;

pub use render::{ModuleReduction, RenderEvent, RenderReport, render_clip};
use settings::ModuleState;
pub use settings::{
    EngineSettings, ModuleInfo, ModuleSetting, ParamChoiceInfo, ParamInfo, ParamValue, default_settings,
    module_catalog, settings_from_json, settings_to_json,
};

/// System Audio Recording permission for this app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Permission {
    /// Granted.
    Authorized,
    /// Denied in System Settings.
    Denied,
    /// macOS will ask when audio first flows.
    NotDetermined,
    /// Couldn't check.
    Unknown,
}

impl From<CaPermission> for Permission {
    fn from(p: CaPermission) -> Self {
        match p {
            CaPermission::Authorized => Self::Authorized,
            CaPermission::Denied => Self::Denied,
            CaPermission::NotDetermined => Self::NotDetermined,
            CaPermission::Unknown => Self::Unknown,
        }
    }
}

/// One module as the UI shows it.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ModuleStatus {
    /// Stable key (`dehum`, ...).
    pub key: String,
    /// Display name.
    pub name: String,
    /// Enabled (target state).
    pub enabled: bool,
    /// Triggers this session.
    pub triggers: u64,
    /// Current gain reduction in dB.
    pub gain_reduction_db: f32,
}

/// Everything the popover shows, polled ~10–60×/s while visible.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct EngineStatus {
    /// Engine started.
    pub running: bool,
    /// Running but no processed app is playing (the tap is idle).
    pub idle: bool,
    /// Global bypass (A/B).
    pub bypassed: bool,
    /// System Audio Recording permission.
    pub permission: Permission,
    /// Processed apps currently playing audio.
    pub processing_apps: u32,
    /// Latency hEARo adds (buffer + lookahead), ms.
    pub added_latency_ms: f64,
    /// Output device's own latency, ms.
    pub device_latency_ms: f64,
    /// Worst IOProc cycle since the last poll as % of the buffer duration.
    pub cpu_percent: f64,
    /// Input/output peak since the last poll, dBFS (−inf as −200).
    pub input_peak_db: f32,
    /// Output peak, dBFS.
    pub output_peak_db: f32,
    /// Output device name.
    pub output_device: String,
    /// Overruns / faults / panics since start.
    pub overruns: u64,
    /// Non-finite samples caught by the chain.
    pub faults: u64,
    /// Caught panics (the engine then stays in passthrough).
    pub panics: u64,
    /// Snapshots saved this session.
    pub snapshots_saved: u32,
    /// Times the engine followed a new output device (or sample rate) this session.
    pub device_switches: u32,
    /// Why hEARo isn't processing although switched on (e.g. the new output device can't be
    /// used); empty when fine. Audio then plays normally, unprocessed.
    pub problem: String,
    /// Modules in chain order.
    pub modules: Vec<ModuleStatus>,
}

/// New live-waveform columns since the last call (60 per second, oldest first).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct WaveformChunk {
    /// Pass this as `since` next time.
    pub next: u64,
    /// Peak of the original sound per column, 0..=1.
    pub peaks: Vec<f32>,
    /// Whether a cleanup module removed something in that column (> 1 dB; draw it red).
    pub removed: Vec<bool>,
    /// Peak of the processed output per column, 0..=1 (same alignment as `peaks`).
    pub output_peaks: Vec<f32>,
    /// Gain reduction per column for each cleanup module, chain order (de-hum, de-plosive,
    /// de-click, de-smack, de-ess); dB ≥ 0 in 0.25 dB steps. Colour them outside-in.
    pub reductions: Vec<ModuleReduction>,
}

/// Errors surfaced to Swift.
#[derive(Debug, Clone, PartialEq, uniffi::Error)]
pub enum HearoError {
    /// System Audio Recording is denied.
    PermissionDenied,
    /// The engine isn't running.
    NotRunning,
    /// Core Audio or setup failed.
    Engine {
        /// Details.
        message: String,
    },
    /// Writing a snapshot failed.
    Io {
        /// Details.
        message: String,
    },
}

impl std::fmt::Display for HearoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PermissionDenied => f.write_str("System Audio Recording permission is denied"),
            Self::NotRunning => f.write_str("hEARo isn't running"),
            Self::Engine { message } | Self::Io { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for HearoError {}

struct Inner {
    /// Owns the running engine (if any) and keeps it alive through device changes, stalls and
    /// sleep (ADR 0014). It re-applies bypass, enabled flags and params to every new engine.
    sup: Supervisor<LiveBackend>,
    /// `Counters::starts` when `last_stats` was taken (a rebuilt engine restarts its stats).
    stats_engine: u32,
    /// Module keys, enabled flags and parameter values as the UI sees them (ADR 0015); every
    /// change is mirrored into `sup`.
    modules: ModuleState,
    snapshots_saved: u32,
    last_stats: StatsSnapshot,
    playing_apps: (Instant, u32),
}

impl Inner {
    fn engine(&self) -> Option<&LiveEngine> {
        self.sup.engine()
    }
}

/// The engine as Swift sees it. Thread-safe; every call is short.
#[derive(uniffi::Object)]
pub struct HearoEngine {
    inner: Mutex<Inner>,
}

impl std::fmt::Debug for HearoEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HearoEngine").finish_non_exhaustive()
    }
}

fn peak_db(x: f32) -> f32 {
    if x > 0.0 { hearo_dsp::amp_to_db(x) } else { -200.0 }
}

impl HearoEngine {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A poisoned lock only means a previous call panicked; the data is still usable.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Supervisor counters (tests, diagnostics; not exported to Swift).
    pub fn counters(&self) -> Counters {
        self.lock().sup.counters()
    }
}

fn live_backend(module_keys: Vec<String>) -> LiveBackend {
    LiveBackend::new(move || EngineConfig {
        exclude: DEFAULT_EXCLUDES.iter().map(|s| s.to_string()).collect(),
        modules: module_keys.iter().filter_map(|k| modules::create(k)).collect(),
        max_frames: 4096,
        capture_seconds: 20.0,
        buffer_frames: 64,
    })
}

fn to_hearo_error(e: EngineError) -> HearoError {
    match e {
        EngineError::PermissionDenied => HearoError::PermissionDenied,
        other => HearoError::Engine { message: other.to_string() },
    }
}

/// Background thread driving the supervisor: wakes on device-change notifications (then waits
/// for the burst to settle) or every `TICK`, detects system sleep, and ticks (follow output,
/// retry, watchdog). Ends once the engine is dropped.
fn spawn_supervisor(engine: Weak<HearoEngine>) {
    let spawned = std::thread::Builder::new().name("hearo-supervisor".into()).spawn(move || {
        let watcher = OutputWatcher::new().ok();
        let mut seen = watcher.as_ref().map_or(0, OutputWatcher::generation);
        let mut sleep = SleepDetector::default();
        loop {
            match &watcher {
                Some(w) => {
                    if w.wait(seen, supervisor::TICK) != seen {
                        std::thread::sleep(supervisor::SETTLE); // act once the burst settled
                        seen = w.generation();
                    }
                }
                None => std::thread::sleep(supervisor::TICK),
            }
            let woke = sleep.observe(Instant::now(), SystemTime::now());
            let Some(e) = engine.upgrade() else { break };
            let mut inner = e.lock();
            let now = Instant::now();
            if woke {
                inner.sup.woke(now);
            }
            inner.sup.tick(now);
        }
    });
    // Without the thread hEARo still works; it just won't follow device changes or recover.
    drop(spawned);
}

#[uniffi::export]
impl HearoEngine {
    /// An engine with these module keys in chain order (unknown keys are ignored).
    #[uniffi::constructor]
    pub fn new(modules: Vec<String>) -> Arc<Self> {
        let module_keys: Vec<String> = modules.into_iter().filter(|k| modules::create(k).is_some()).collect();
        let modules = ModuleState::new(module_keys.clone());
        let mut sup = Supervisor::new(live_backend(module_keys), Settings::all_enabled(modules.keys.len()));
        let (enabled, params) = modules.for_supervisor();
        sup.set_modules(enabled, params);
        let engine = Arc::new(Self {
            inner: Mutex::new(Inner {
                sup,
                stats_engine: 0,
                modules,
                snapshots_saved: 0,
                last_stats: StatsSnapshot::default(),
                playing_apps: (Instant::now() - Duration::from_secs(10), 0),
            }),
        });
        spawn_supervisor(Arc::downgrade(&engine));
        engine
    }

    /// Build the tap + aggregate on the current output and start processing. hEARo then
    /// follows the system output (AirPods, Sonos, …), recovers from stalls and sleep, and
    /// retries failures until `stop`.
    pub fn start(&self) -> Result<(), HearoError> {
        self.lock().sup.start(Instant::now()).map_err(to_hearo_error)
    }

    /// Stop processing; macOS plays every app normally again.
    pub fn stop(&self) {
        self.lock().sup.stop();
    }

    /// Global bypass (A/B). Remembered across restarts of the engine.
    pub fn set_bypass(&self, on: bool) {
        self.lock().sup.set_bypass(on);
    }

    /// Enable/disable a module by key (always-on modules stay on). Remembered across rebuilds.
    pub fn set_module_enabled(&self, key: String, on: bool) {
        let mut inner = self.lock();
        if let Some((i, on)) = inner.modules.set_enabled(&key, on) {
            inner.sup.set_enabled(i, on);
        }
    }

    /// Current status (call from the UI's poll timer).
    pub fn status(&self) -> EngineStatus {
        let mut inner = self.lock();
        let permission = permission::audio_capture().into();
        let meters = |id| inner.engine().map_or((0, 0.0), |e| e.stats().module(id));
        let modules: Vec<ModuleStatus> = inner
            .modules
            .keys
            .iter()
            .zip(&inner.modules.enabled)
            .map(|(k, on)| {
                let id = modules::create(k).map(|m| m.id());
                let (triggers, gain_reduction_db) = id.map_or((0, 0.0), meters);
                ModuleStatus {
                    key: k.clone(),
                    name: id.map_or_else(|| k.clone(), |id| id.display_name().to_string()),
                    enabled: *on,
                    triggers,
                    gain_reduction_db,
                }
            })
            .collect();
        // Apps playing: refreshed at most once a second (enumerating processes isn't free).
        if inner.playing_apps.0.elapsed() > Duration::from_secs(1) {
            let excluded = |b: &str| hearo_engine::is_excluded(b, DEFAULT_EXCLUDES);
            let n = procs::list()
                .map(|l| {
                    l.iter()
                        .filter(|p| p.running_output && !excluded(&p.bundle_id) && p.pid != std::process::id() as i32)
                        .count()
                })
                .unwrap_or(0);
            inner.playing_apps = (Instant::now(), n as u32);
        }
        let processing_apps = inner.playing_apps.1;
        let Some(engine) = inner.engine() else {
            return EngineStatus {
                running: false,
                idle: false,
                bypassed: inner.sup.settings().bypass,
                permission,
                processing_apps,
                added_latency_ms: 0.0,
                device_latency_ms: 0.0,
                cpu_percent: 0.0,
                input_peak_db: -200.0,
                output_peak_db: -200.0,
                output_device: String::new(),
                overruns: 0,
                faults: 0,
                panics: 0,
                snapshots_saved: inner.snapshots_saved,
                device_switches: inner.sup.counters().device_switches,
                problem: inner.sup.problem().unwrap_or_default().to_string(),
                modules,
            };
        };
        let s = engine.stats().snapshot();
        let info = engine.info();
        // A rebuilt engine counts from zero again.
        let starts = inner.sup.counters().starts;
        let last_callbacks = if inner.stats_engine == starts { inner.last_stats.callbacks } else { 0 };
        let idle = s.callbacks == last_callbacks;
        let budget_ns = s.last_frames as f64 / info.aggregate.sample_rate.max(1.0) * 1e9;
        let status = EngineStatus {
            running: true,
            idle,
            bypassed: inner.sup.settings().bypass,
            permission,
            processing_apps,
            added_latency_ms: info.added_latency_ms(),
            device_latency_ms: info.device_latency_ms(),
            cpu_percent: if budget_ns > 0.0 { s.max_cycle_ns as f64 / budget_ns * 100.0 } else { 0.0 },
            input_peak_db: peak_db(s.in_peak),
            output_peak_db: peak_db(s.out_peak),
            output_device: info.output.name.clone(),
            overruns: s.overruns,
            faults: s.faults,
            panics: s.panics,
            snapshots_saved: inner.snapshots_saved,
            device_switches: inner.sup.counters().device_switches,
            problem: inner.sup.problem().unwrap_or_default().to_string(),
            modules,
        };
        inner.last_stats = s;
        inner.stats_engine = starts;
        status
    }

    /// Waveform columns with index ≥ `since` (at most `max_columns`, the most recent ones).
    pub fn waveform(&self, since: u64, max_columns: u32) -> WaveformChunk {
        let inner = self.lock();
        let Some(engine) = inner.engine() else {
            return WaveformChunk {
                next: since,
                peaks: vec![],
                removed: vec![],
                output_peaks: vec![],
                reductions: reductions(&[]),
            };
        };
        let (next, cols) = engine.waveform().read_since(since, max_columns as usize);
        WaveformChunk {
            next,
            peaks: cols.iter().map(|c| c.peak).collect(),
            removed: cols.iter().map(|c| c.removed()).collect(),
            output_peaks: cols.iter().map(|c| c.out_peak).collect(),
            reductions: reductions(&cols),
        }
    }

    /// Set one module parameter (clamped and snapped to a legal value; unknown keys are
    /// ignored). Takes effect within one audio buffer, with the module's own smoothing, and is
    /// remembered across device-change rebuilds.
    pub fn set_param(&self, module_key: String, param_key: String, value: f32) {
        let mut inner = self.lock();
        if let Some((slot, p, v)) = inner.modules.set_param(&module_key, &param_key, value) {
            inner.sup.set_param(slot, p, v);
        }
    }

    /// Enabled flags and parameter values of every module in this engine (persist with
    /// `settings_to_json`).
    pub fn settings(&self) -> EngineSettings {
        self.lock().modules.settings()
    }

    /// Replace all module settings (unknown modules/params ignored, missing ones reset to
    /// defaults, values clamped). Applied live and kept across rebuilds. Bypass is unaffected.
    pub fn apply_settings(&self, settings: EngineSettings) {
        let mut inner = self.lock();
        inner.modules.apply(&settings);
        let (enabled, params) = inner.modules.for_supervisor();
        inner.sup.set_modules(enabled, params);
    }

    /// Save the last 8 s of unprocessed audio. `false_hit` = hEARo removed something it
    /// shouldn't have; otherwise "missed it". Returns the WAV path.
    pub fn save_snapshot(&self, false_hit: bool) -> Result<String, HearoError> {
        let mut inner = self.lock();
        let ctx = Context { modules: inner.modules.keys.clone(), source: "app".into(), ..Context::default() };
        let engine = inner.engine().ok_or(HearoError::NotRunning)?;
        let dir = snapshots::default_dir().ok_or(HearoError::Io { message: "HOME not set".into() })?;
        let label = if false_hit { Label::FalseHit } else { Label::Missed };
        let path = engine.save_snapshot(&dir, label, ctx).map_err(|e| HearoError::Io { message: e.to_string() })?;
        inner.snapshots_saved += 1;
        Ok(path.display().to_string())
    }
}

/// One row per cleanup module (chain order) from live columns.
fn reductions(cols: &[hearo_engine::waveform::Column]) -> Vec<ModuleReduction> {
    hearo_engine::waveform::GR_MODULES
        .iter()
        .enumerate()
        .map(|(i, id)| ModuleReduction {
            key: id.key().into(),
            gain_reduction_db: cols.iter().map(|c| c.gain_reduction_db[i]).collect(),
        })
        .collect()
}

/// Module keys the engine knows (for settings UIs).
#[uniffi::export]
pub fn available_modules() -> Vec<String> {
    modules::KEYS.iter().map(|s| s.to_string()).collect()
}

/// System Audio Recording permission (never prompts).
#[uniffi::export]
pub fn current_permission() -> Permission {
    permission::audio_capture().into()
}

/// Result of importing a snapshot archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct ImportReport {
    /// Snapshots copied in.
    pub added: u32,
    /// Already present.
    pub skipped: u32,
    /// Name clash, imported under a new name.
    pub renamed: u32,
}

fn snapshot_dir() -> Result<std::path::PathBuf, HearoError> {
    snapshots::default_dir().ok_or(HearoError::Io { message: "HOME not set".into() })
}

/// Number of saved snapshots on this Mac.
#[uniffi::export]
pub fn snapshot_count() -> u32 {
    snapshot_dir().map_or(0, |d| snapshots::list(&d).len() as u32)
}

/// The snapshot folder (for "Show in Finder").
#[uniffi::export]
pub fn snapshot_folder() -> String {
    snapshot_dir().map(|d| d.display().to_string()).unwrap_or_default()
}

/// Zip all snapshots into `path`. Returns how many.
#[uniffi::export]
pub fn export_snapshots(path: String) -> Result<u32, HearoError> {
    let dir = snapshot_dir()?;
    snapshots::export(&dir, std::path::Path::new(&path))
        .map(|n| n as u32)
        .map_err(|e| HearoError::Io { message: e.to_string() })
}

/// Merge a snapshot zip from another Mac. Never overwrites.
#[uniffi::export]
pub fn import_snapshots(path: String) -> Result<ImportReport, HearoError> {
    let dir = snapshot_dir()?;
    snapshots::import(std::path::Path::new(&path), &dir)
        .map(|r| ImportReport { added: r.added, skipped: r.skipped, renamed: r.renamed })
        .map_err(|e| HearoError::Io { message: e.to_string() })
}

/// Waveform columns per second.
#[uniffi::export]
pub fn waveform_columns_per_second() -> f32 {
    hearo_engine::waveform::COLUMNS_PER_SECOND
}

/// hEARo version.
#[uniffi::export]
pub fn hearo_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_without_starting_is_safe() {
        let e = HearoEngine::new(vec!["gain".into(), "nope".into()]);
        let s = e.status();
        assert!(!s.running);
        assert_eq!(s.modules.len(), 1, "unknown keys are dropped");
        e.set_module_enabled("gain".into(), false);
        assert!(!e.status().modules[0].enabled);
        e.set_bypass(true);
        assert!(e.status().bypassed);
        assert_eq!(e.save_snapshot(false), Err(HearoError::NotRunning));
        let w = e.waveform(7, 100);
        assert_eq!((w.next, w.peaks.len()), (7, 0), "no engine → nothing, same cursor");
        assert_eq!(w.reductions.len(), 5, "one (empty) row per cleanup module");
        e.stop();
    }

    #[test]
    fn params_and_settings_without_starting() {
        let e = HearoEngine::new(vec!["desmack".into(), "deess".into(), "limiter".into()]);
        e.set_param("desmack".into(), "range".into(), 99.0);
        e.set_param("desmack".into(), "nope".into(), 1.0);
        e.set_param("nope".into(), "range".into(), 1.0);
        e.set_module_enabled("limiter".into(), false);
        let s = e.settings();
        assert_eq!(s.modules.len(), 3);
        assert_eq!(s.modules[0].params.iter().find(|p| p.key == "range").map(|p| p.value), Some(48.0), "clamped");
        assert!(s.modules[2].enabled, "the limiter is always on");
        // Round trip through JSON, then a reset to defaults.
        let restored = settings_from_json(settings_to_json(s.clone())).unwrap();
        e.apply_settings(EngineSettings::default());
        assert_ne!(e.settings(), s);
        e.apply_settings(restored);
        assert_eq!(e.settings(), s);
        e.stop();
    }
}
