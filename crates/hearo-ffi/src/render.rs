//! Offline render for the snapshot review window: run a WAV through the same chain the live
//! engine uses (same modules, settings, block size) and report what each module removed.
//! Shares `hearo_dsp::offline::render` with `hearo process`.

use std::time::Instant;

use hearo_dsp::{Chain, MAX_CHANNELS, Module, ModuleId, modules};
use hearo_testkit::wav;

use crate::HearoError;
use crate::settings::{EngineSettings, ModuleState, user_module_keys};

/// Per-column gain reduction of one cleanup module.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ModuleReduction {
    /// Module key (`dehum`, `deplosive`, `declick`, `desmack`, `deess`).
    pub key: String,
    /// Largest gain reduction in each column, dB (≥ 0). Same length as the peaks.
    pub gain_reduction_db: Vec<f32>,
}

/// Something a module removed in a rendered clip.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct RenderEvent {
    /// Module key.
    pub module: String,
    /// `click`, `smack`, `plosive`, `sibilance`, `hum` or `limit`.
    pub kind: String,
    /// Start in the clip, seconds.
    pub start_s: f64,
    /// Length, seconds.
    pub duration_s: f64,
    /// Energy removed, dB (positive = reduction).
    pub reduction_db: f32,
}

/// Result of [`render_clip`].
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct RenderReport {
    /// The processed WAV that was written (32-bit float, same rate/channels/length).
    pub output_path: String,
    /// Sample rate, Hz.
    pub sample_rate: u32,
    /// Channels.
    pub channels: u32,
    /// Frames per channel (input = output).
    pub frames: u64,
    /// Columns per second actually used.
    pub columns_per_second: f32,
    /// Peak |sample| of the original per column (0..; not clamped).
    pub input_peaks: Vec<f32>,
    /// Peak |sample| of the processed output per column.
    pub output_peaks: Vec<f32>,
    /// One row per cleanup module, chain order (de-hum, de-plosive, de-click, de-smack, de-ess);
    /// draw them outside-in. All 0 under bypass or when the module is off.
    pub reductions: Vec<ModuleReduction>,
    /// Events, in the order modules reported them.
    pub events: Vec<RenderEvent>,
    /// Chain latency that was compensated, samples.
    pub latency_samples: u32,
    /// Non-finite samples the chain caught (should be 0).
    pub faults: u64,
    /// Wall-clock time the processing took, ms.
    pub render_ms: f64,
}

/// The live engine's IO buffer: 64 frames at 48 kHz, scaled to a power of two with the rate
/// (de-click's per-callback repair limit depends on it, so offline matches live).
fn live_block(sample_rate: f64) -> usize {
    let f = (64.0 * sample_rate / 48_000.0).max(32.0);
    (f.log2().round().exp2() as usize).max(32)
}

fn io(e: impl std::fmt::Display) -> HearoError {
    HearoError::Io { message: e.to_string() }
}

/// Process the WAV at `wav_path` with `settings` (normalised like `apply_settings`: unknown keys
/// ignored, missing modules/params at defaults) through every user-facing module, write the
/// result to `out_wav_path` and report per-column peaks and per-module gain reduction at
/// `columns_per_second` (clamped to 1…1000; non-finite → 60). `bypass` renders the global-bypass
/// path: the output is then bit-identical to the input. The output lines up sample for sample
/// with the input (chain latency compensated). Thread-safe, no shared state; an 8 s clip takes
/// a few tens of ms.
#[uniffi::export]
pub fn render_clip(
    wav_path: String,
    out_wav_path: String,
    settings: EngineSettings,
    bypass: bool,
    columns_per_second: f32,
) -> Result<RenderReport, HearoError> {
    let input = wav::read(&wav_path).map_err(io)?;
    let channels = input.channels.len();
    if channels == 0 || channels > MAX_CHANNELS {
        return Err(io(format!("unsupported channel count {channels}")));
    }
    let sr = input.sample_rate as f64;
    let cps = if columns_per_second.is_finite() { columns_per_second.clamp(1.0, 1000.0) } else { 60.0 };

    let mut state = ModuleState::new(user_module_keys());
    state.apply(&settings);
    let mods: Vec<Box<dyn Module>> = state.keys.iter().filter_map(|k| modules::create(k)).collect();
    let mut chain = Chain::new(mods);
    // Toggles and bypass before `prepare`, which settles them (no fades at the clip start).
    for (i, on) in state.enabled.iter().enumerate() {
        chain.set_enabled(i, *on);
    }
    chain.set_bypass(bypass);
    let block = live_block(sr);
    chain.prepare(input.sample_rate as f32, block, channels);
    for (i, params) in state.params.iter().enumerate() {
        for (p, v) in params.iter().enumerate() {
            chain.set_param(i, p, *v);
        }
    }

    let started = Instant::now();
    let r = hearo_dsp::offline::render(&mut chain, &input.channels, block, Some(cps as f64));
    let render_ms = started.elapsed().as_secs_f64() * 1000.0;
    let out = wav::Wav { sample_rate: input.sample_rate, channels: r.channels };
    wav::write(&out_wav_path, &out).map_err(io)?;

    let cols = r.columns.unwrap_or_else(unreachable_columns);
    Ok(RenderReport {
        output_path: out_wav_path,
        sample_rate: input.sample_rate,
        channels: channels as u32,
        frames: out.frames() as u64,
        columns_per_second: cols.per_second as f32,
        input_peaks: cols.input_peak,
        output_peaks: cols.output_peak,
        reductions: ModuleId::CLEANUP
            .iter()
            .zip(cols.gain_reduction_db)
            .map(|(id, db)| ModuleReduction { key: id.key().into(), gain_reduction_db: db })
            .collect(),
        events: r
            .events
            .iter()
            .map(|e| RenderEvent {
                module: e.module.key().into(),
                kind: format!("{:?}", e.kind).to_lowercase(),
                start_s: e.start_sample as f64 / sr,
                duration_s: e.duration_samples as f64 / sr,
                reduction_db: e.energy_removed_db,
            })
            .collect(),
        latency_samples: chain.latency_samples() as u32,
        faults: chain.faults(),
        render_ms,
    })
}

/// `render` always returns columns when asked; this keeps the code panic-free regardless.
fn unreachable_columns() -> hearo_dsp::offline::Columns {
    hearo_dsp::offline::Columns {
        per_second: 0.0,
        input_peak: vec![],
        output_peak: vec![],
        gain_reduction_db: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use hearo_testkit::signals;

    use super::*;
    use crate::settings::{ModuleSetting, default_settings};

    fn tmp(name: &str) -> String {
        std::env::temp_dir().join(format!("hearo-render-{}-{name}.wav", std::process::id())).display().to_string()
    }

    /// 8 s of speech with lip smacks in the pauses, stereo.
    fn clip(name: &str) -> (String, Vec<signals::Injected>) {
        let sr = 48_000.0;
        let (x, smacks) = signals::speech_with_pause_smacks(sr, 8.0, 41, 5, (-18.0, -12.0), -55.0);
        let path = tmp(name);
        wav::write(&path, &wav::Wav { sample_rate: 48_000, channels: vec![x.clone(), x] }).unwrap();
        (path, smacks)
    }

    #[test]
    fn bypass_render_is_bit_exact() {
        let (path, _) = clip("bypass-in");
        let out = tmp("bypass-out");
        let r = render_clip(path.clone(), out.clone(), default_settings(), true, 60.0).unwrap();
        assert_eq!(wav::read(&out).unwrap(), wav::read(&path).unwrap());
        assert!(r.reductions.iter().all(|m| m.gain_reduction_db.iter().all(|&g| g == 0.0)));
        assert_eq!(r.input_peaks, r.output_peaks);
        let _ = (std::fs::remove_file(path), std::fs::remove_file(out));
    }

    #[test]
    fn processed_render_is_aligned_fast_and_shows_desmack_where_smacks_are() {
        let (path, smacks) = clip("smack-in");
        let out = tmp("smack-out");
        let started = Instant::now();
        let r = render_clip(path.clone(), out.clone(), default_settings(), false, 100.0).unwrap();
        let wall = started.elapsed().as_secs_f64();
        let (inp, outp) = (wav::read(&path).unwrap(), wav::read(&out).unwrap());
        assert_eq!((outp.frames(), outp.channels.len(), outp.sample_rate), (inp.frames(), 2, 48_000));
        assert_eq!(r.frames, inp.frames() as u64);
        assert_eq!(r.faults, 0);
        assert!(wall < 1.0, "8 s clip took {wall:.2} s");
        assert_eq!(r.input_peaks.len(), 800);
        let keys: Vec<&str> = r.reductions.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(keys, ["dehum", "deplosive", "declick", "desmack", "deess"]);
        // Alignment: away from the smacks the output equals the input closely (speech mostly
        // passes), so the correlation peak is at lag 0.
        let corr = |lag: usize| -> f64 {
            inp.channels[0][..inp.frames() - lag]
                .iter()
                .zip(&outp.channels[0][lag..])
                .map(|(a, b)| (*a as f64) * (*b as f64))
                .sum()
        };
        assert!(corr(0) > corr(1) && corr(0) > corr(64), "output not aligned with input");
        // De-smack reduces in the columns of the injected smacks.
        assert!(smacks.len() >= 3, "test clip has smacks");
        let desmack = &r.reductions[3].gain_reduction_db;
        let hits = smacks
            .iter()
            .filter(|s| {
                let (a, b) = (s.start / 480, s.end() / 480);
                desmack[a.saturating_sub(1)..=(b + 1).min(799)].iter().any(|&g| g > 3.0)
            })
            .count();
        assert!(hits * 10 >= smacks.len() * 8, "de-smack GR in {hits} of {} smack columns", smacks.len());
        assert!(r.events.iter().any(|e| e.module == "desmack" && e.kind == "smack"));
        let _ = (std::fs::remove_file(path), std::fs::remove_file(out));
    }

    #[test]
    fn disabled_module_reports_nothing_and_bad_input_errors() {
        let (path, _) = clip("off-in");
        let out = tmp("off-out");
        let s =
            EngineSettings { modules: vec![ModuleSetting { key: "desmack".into(), enabled: false, params: vec![] }] };
        let r = render_clip(path.clone(), out.clone(), s, false, 60.0).unwrap();
        assert!(r.reductions[3].gain_reduction_db.iter().all(|&g| g == 0.0));
        assert!(!r.events.iter().any(|e| e.module == "desmack"));
        assert!(render_clip("/nonexistent.wav".into(), out.clone(), default_settings(), false, 60.0).is_err());
        let _ = (std::fs::remove_file(path), std::fs::remove_file(out));
    }
}
