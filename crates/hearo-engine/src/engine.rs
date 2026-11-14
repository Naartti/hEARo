//! [`LiveEngine`]: builds tap → private aggregate → IOProc, runs the chain, tears it all down.

#![allow(unsafe_code)]

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::Arc;

use hearo_dsp::{Chain, Module};
use objc2_core_audio::{
    AudioDeviceCreateIOProcID, AudioDeviceDestroyIOProcID, AudioDeviceIOProcID, AudioDeviceStart, AudioDeviceStop,
    kAudioDevicePropertyIOProcStreamUsage, kAudioDevicePropertyStreams,
};

use crate::ca::aggregate::Aggregate;
use crate::ca::device::{self, DeviceInfo};
use crate::ca::permission::{self, Permission};
use crate::ca::procs::{self, ProcessInfo};
use crate::ca::tap::Tap;
use crate::ca::{CaError, INPUT, ObjectId, check, get_vec, set_raw};
use crate::rt::{RtState, io_proc};
use crate::{CaptureRing, Controls, Stats, is_excluded};

/// How to start the engine.
pub struct EngineConfig {
    /// Bundle-ID prefixes to leave untouched (hEARo itself is always excluded).
    pub exclude: Vec<String>,
    /// The processing modules, in chain order.
    pub modules: Vec<Box<dyn Module>>,
    /// Largest IO buffer we prepare for.
    pub max_frames: usize,
    /// Seconds of unprocessed input kept for snapshots (0 = off).
    pub capture_seconds: f64,
    /// Requested IO buffer in frames *at 48 kHz* (scaled to the device rate; 0 = device default). Smaller = less latency, a few µs
    /// more overhead per second. ADR 0011/0012: 64 keeps the full chain (≈ 43 ms lookahead) under the 45 ms ceiling.
    pub buffer_frames: u32,
}

impl std::fmt::Debug for EngineConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineConfig")
            .field("exclude", &self.exclude)
            .field("modules", &self.modules.iter().map(|m| m.id()).collect::<Vec<_>>())
            .finish()
    }
}

/// What the engine set up (for status lines and diagnostics).
#[derive(Debug, Clone)]
pub struct RunInfo {
    /// The real output device.
    pub output: DeviceInfo,
    /// The aggregate device as opened.
    pub aggregate: DeviceInfo,
    /// Processes excluded from the tap (always includes hEARo itself if Core Audio knows it).
    pub excluded: Vec<ProcessInfo>,
    /// Chain latency in samples (lookahead).
    pub chain_latency: usize,
    /// System Audio Recording permission at start.
    pub permission: Permission,
}

/// `frames_at_48k` scaled to `sample_rate`, rounded to a power of two (min 32).
pub fn scaled_buffer_frames(frames_at_48k: u32, sample_rate: f64) -> u32 {
    let f = (frames_at_48k as f64 * sample_rate / 48_000.0).max(32.0);
    (f.log2().round().exp2() as u32).max(32)
}

impl RunInfo {
    /// Latency hEARo adds on top of the device's own: one IO buffer + chain lookahead.
    pub fn added_latency_ms(&self) -> f64 {
        let sr = self.aggregate.sample_rate.max(1.0);
        (self.aggregate.buffer_frames as f64 + self.chain_latency as f64) * 1000.0 / sr
    }

    /// Output device latency (outside hEARo's budget, e.g. Bluetooth).
    pub fn device_latency_ms(&self) -> f64 {
        self.output.output_latency_frames as f64 * 1000.0 / self.output.sample_rate.max(1.0)
    }
}

/// The running engine. Dropping it stops audio and removes the tap: macOS then plays every
/// app normally again (MutedWhenTapped).
#[derive(Debug)]
pub struct LiveEngine {
    info: RunInfo,
    controls: Arc<Controls>,
    stats: Arc<Stats>,
    state: *mut RtState,
    proc_id: AudioDeviceIOProcID,
    // Drop order matters: aggregate before tap.
    aggregate: Option<Aggregate>,
    tap: Option<Tap>,
    module_count: usize,
    capture: Option<Arc<CaptureRing>>,
    waveform: Arc<crate::waveform::WaveformRing>,
    /// Process objects the tap excludes (for the watchdog's "is a tapped app playing?").
    tap_excluded: Vec<ObjectId>,
}

/// Setup error.
#[derive(Debug)]
pub enum EngineError {
    /// A Core Audio call failed.
    CoreAudio(CaError),
    /// System Audio Recording permission was denied for the app running hEARo.
    PermissionDenied,
    /// Something else (bad config).
    Other(String),
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CoreAudio(e) => write!(f, "{e}"),
            Self::PermissionDenied => f.write_str(
                "System Audio Recording permission is denied. Enable it for the app running hEARo \
                 (Terminal/iTerm/VS Code for the CLI, hEARo for the app) in System Settings → \
                 Privacy & Security → Screen & System Audio Recording, then restart that app",
            ),
            Self::Other(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for EngineError {}

impl From<CaError> for EngineError {
    fn from(e: CaError) -> Self {
        Self::CoreAudio(e)
    }
}

impl LiveEngine {
    /// Build the tap and aggregate device on the current default output and start processing.
    pub fn start(config: EngineConfig) -> Result<Self, EngineError> {
        let permission = permission::audio_capture();
        if permission == Permission::Denied {
            return Err(EngineError::PermissionDenied);
        }
        let output = device::info(device::default_output()?)?;
        if output.output_channels == 0 {
            return Err(EngineError::Other(format!("output device '{}' has no output channels", output.name)));
        }

        let me = std::process::id() as i32;
        let all = procs::list()?;
        let excluded: Vec<ProcessInfo> =
            all.into_iter().filter(|p| p.pid == me || is_excluded(&p.bundle_id, &config.exclude)).collect();
        let mut exclude_ids: Vec<ObjectId> = excluded.iter().map(|p| p.object).collect();
        if let Some(me_obj) = procs::object_for_pid(me)?
            && !exclude_ids.contains(&me_obj)
        {
            exclude_ids.push(me_obj);
        }

        let tap = Tap::global_excluding(&exclude_ids, "hEARo")?;
        let tap_excluded = exclude_ids;
        let uid = format!("dev.hearo.aggregate.{me}.{}", tap.id());
        let aggregate = Aggregate::create("hEARo", &uid, &output.uid, tap.uid())?;
        if config.buffer_frames > 0 {
            // `buffer_frames` is specified at 48 kHz; scale so the buffer is the same *duration* at
            // any rate (64 @ 48 kHz → 128 @ 96 kHz), keeping latency and per-callback CPU headroom.
            let frames = scaled_buffer_frames(config.buffer_frames, output.sample_rate);
            // Best effort: some devices refuse small buffers; then we keep their default.
            let _ = crate::ca::set(
                aggregate.id(),
                objc2_core_audio::kAudioDevicePropertyBufferFrameSize,
                crate::ca::GLOBAL,
                &frames,
            );
        }
        let agg_info = device::info(aggregate.id())?;

        let max_frames = config.max_frames.max(agg_info.buffer_frames as usize).max(64);
        let module_count = config.modules.len();
        let mut chain = Chain::new(config.modules);
        chain.prepare(agg_info.sample_rate as f32, max_frames, 2);
        let chain_latency = chain.latency_samples();

        let controls = Arc::new(Controls::default());
        controls.set_bypass(chain.is_bypassed());
        for i in 0..chain.len() {
            controls.set_enabled(i, chain.is_enabled(i));
        }
        let stats = Arc::new(Stats::default());
        // Clamp: a huge value would try to allocate an enormous ring.
        let capture_secs =
            if config.capture_seconds.is_finite() { config.capture_seconds.clamp(0.0, 120.0) } else { 0.0 };
        let capture =
            (capture_secs > 0.0).then(|| Arc::new(CaptureRing::new((capture_secs * agg_info.sample_rate) as usize)));
        let waveform = Arc::new(crate::waveform::WaveformRing::default());
        let state = Box::into_raw(Box::new(
            RtState::with_capture(
                chain,
                max_frames,
                agg_info.sample_rate,
                controls.clone(),
                stats.clone(),
                capture.clone(),
            )
            .with_waveform(waveform.clone()),
        ));

        let mut proc_id: AudioDeviceIOProcID = None;
        // SAFETY: valid device, C callback, client pointer kept alive until Drop.
        let status = unsafe {
            AudioDeviceCreateIOProcID(aggregate.id(), Some(io_proc), state as *mut c_void, NonNull::from(&mut proc_id))
        };
        if let Err(e) = check("AudioDeviceCreateIOProcID", status) {
            // SAFETY: never handed to Core Audio.
            drop(unsafe { Box::from_raw(state) });
            return Err(e.into());
        }

        let engine = Self {
            info: RunInfo { output, aggregate: agg_info, excluded, chain_latency, permission },
            controls,
            stats,
            state,
            proc_id,
            aggregate: Some(aggregate),
            tap: Some(tap),
            module_count,
            capture,
            waveform,
            tap_excluded,
        };
        engine.use_only_tap_input()?;
        // SAFETY: valid device and proc id.
        check("AudioDeviceStart", unsafe { AudioDeviceStart(engine.aggregate_id(), engine.proc_id) })?;
        // Note: the IOProc only runs while some tapped process is playing audio; zero
        // callbacks right after start is normal ("idle"), not an error (ADR 0010).
        Ok(engine)
    }

    fn aggregate_id(&self) -> ObjectId {
        self.aggregate.as_ref().map_or(0, Aggregate::id)
    }

    /// Switch off every aggregate input stream except the tap (the last one) for our IOProc,
    /// so e.g. an AirPods microphone is never opened (which would force the call profile).
    fn use_only_tap_input(&self) -> Result<(), CaError> {
        let streams: Vec<ObjectId> = get_vec(self.aggregate_id(), kAudioDevicePropertyStreams, INPUT)?;
        let n = streams.len();
        if n <= 1 {
            return Ok(());
        }
        // AudioHardwareIOProcStreamUsage { void* mIOProc; UInt32 mNumberStreams; UInt32 mStreamIsOn[n]; }
        let mut buf = vec![0u32; (3 + n).next_multiple_of(2)];
        let proc_ptr = self.proc_id.map_or(0usize, |f| f as usize) as u64;
        buf[0] = proc_ptr as u32;
        buf[1] = (proc_ptr >> 32) as u32;
        buf[2] = n as u32;
        for i in 0..n {
            buf[3 + i] = u32::from(i == n - 1);
        }
        set_raw(self.aggregate_id(), kAudioDevicePropertyIOProcStreamUsage, INPUT, buf.as_ptr().cast(), (3 + n) * 4)
    }

    /// Whether the system's default output (device or sample rate) is no longer the one this
    /// engine is bound to. Then the engine must be rebuilt: the aggregate plays to the old device.
    /// Errors (device vanishing mid-query) count as changed.
    pub fn output_changed(&self) -> bool {
        device::default_output_key()
            .map_or(true, |(uid, rate)| uid != self.info.output.uid || rate != self.info.output.sample_rate)
    }

    /// Whether some process the tap captures is sending audio to an output right now, i.e.
    /// whether the IOProc *should* be getting callbacks (it only runs while a tapped app plays,
    /// ADR 0010). `false` when unknown (permission prompt possibly pending, process list
    /// unreadable) so the watchdog only acts on a clear signal.
    pub fn tapped_output_running(&self) -> bool {
        // Process list first: it's false whenever nothing plays, sparing the TCC query.
        procs::others_running_output(std::process::id() as i32, &self.tap_excluded).unwrap_or(false)
            && permission::audio_capture() != Permission::NotDetermined
    }

    /// Setup details.
    pub fn info(&self) -> &RunInfo {
        &self.info
    }

    /// Controls (bypass, module toggles, parameters).
    pub fn controls(&self) -> &Arc<Controls> {
        &self.controls
    }

    /// Live statistics.
    pub fn stats(&self) -> &Arc<Stats> {
        &self.stats
    }

    /// The last `seconds` of unprocessed input as (left, right), if capture is on.
    pub fn capture_last(&self, seconds: f64) -> Option<(Vec<f32>, Vec<f32>)> {
        let ring = self.capture.as_ref()?;
        let frames = (seconds * self.info.aggregate.sample_rate) as usize;
        (0..3).find_map(|_| ring.snapshot(frames))
    }

    /// Live waveform columns (see [`crate::waveform`]).
    pub fn waveform(&self) -> &Arc<crate::waveform::WaveformRing> {
        &self.waveform
    }

    /// Number of modules in the chain.
    pub fn module_count(&self) -> usize {
        self.module_count
    }
}

// SAFETY: `state` is owned by the engine and only dereferenced by the Core Audio IOProc; the
// control side never touches it except to free it in Drop after stopping the IOProc. Every
// other field is Send. Moving the engine between control threads is therefore sound.
unsafe impl Send for LiveEngine {}

impl LiveEngine {
    /// Save the last [`SNAPSHOT_SECS`](crate::snapshots::SNAPSHOT_SECS) of unprocessed input.
    pub fn save_snapshot(
        &self,
        dir: &std::path::Path,
        label: crate::snapshots::Label,
        mut ctx: crate::snapshots::Context,
    ) -> Result<std::path::PathBuf, Box<dyn std::error::Error + Send + Sync>> {
        let (l, r) = self
            .capture_last(crate::snapshots::SNAPSHOT_SECS)
            .ok_or("capture unavailable (off, or the writer kept lapping the window)")?;
        if ctx.output_device.is_empty() {
            ctx.output_device = self.info.output.name.clone();
        }
        crate::snapshots::save(dir, &l, &r, self.info.aggregate.sample_rate, label, &ctx)
    }
}

impl Drop for LiveEngine {
    fn drop(&mut self) {
        let dev = self.aggregate_id();
        // SAFETY: stop IO, then remove the IOProc; only then is RtState unreachable.
        let stopped = unsafe { AudioDeviceStop(dev, self.proc_id) } == 0;
        let destroyed = unsafe { AudioDeviceDestroyIOProcID(dev, self.proc_id) } == 0;
        // Destroying the aggregate also guarantees the IOProc can never run again.
        self.aggregate.take();
        self.tap.take();
        if stopped || destroyed {
            // SAFETY: the IOProc is stopped/removed and the device is gone.
            drop(unsafe { Box::from_raw(self.state) });
        }
        // else: leak RtState rather than risk a use-after-free on the audio thread.
    }
}
