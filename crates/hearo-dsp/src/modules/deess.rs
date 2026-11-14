//! De-ess: a split-band dynamic EQ that turns down harsh sibilance ("s", "sh", "ts").
//!
//! ```text
//! x ─┬─────────────▶(+)── u ──▶(−)──┬──────────────────────────────▶ y (out)
//!    │               ▲              ▲  └─▶ BP ─▶ ×(1/√g − 1) ─┘   (delay-free feedback)
//!    └─▶ BP ─ b ─▶ ×(√g − 1)
//!               └─▶ BP ─▶ power, linked over channels ─▶ 1 ms RMS ─▶ soft-knee gain computer
//!                   (sidechain ≈ 5–9.6 kHz at defaults)  │  (threshold, 5:1, range) ─▶ attack/release ─▶ g
//!                                                        └─▶ 2 ms shape env ─▶ onset gate ──┘ (cut only while armed)
//! ```
//!
//! All band-passes are the same RBJ constant-0 dB-peak band-pass at `freq`/`Q`. The bell is
//! built from two halves with centre gain √g each: a feed-forward cut `x + (√g − 1)·bp(x)`
//! (which narrows as it deepens) and a feedback cut `1 / (1 + (1/√g − 1)·bp)` (which widens).
//! Together: `H = √g·(√g + jy) / (1 + j√g·y)` with `y = Q·(f/f₀ − f₀/f)` — exactly `g` at the
//! centre and half the dB cut at the band-pass −3 dB edges, i.e. a constant-bandwidth
//! peaking EQ (the shape of an RBJ peaking cut) whose depth can move every sample without
//! recomputing coefficients. A single feed-forward stage alone removed only ~5 dB of a
//! wide "s" at 12 dB centre depth (measured), because its skirts are too narrow.
//!
//! **Why a dynamic bell and not a Linkwitz-Riley crossover.**
//! - At rest (`g = 1`) the output *is* the input, bit for bit: no crossover phase shift on
//!   speech or music without sibilance. An LR4 split sums to an all-pass, not to a wire, so it
//!   would phase-shift the 5–9 kHz region of everything, all the time.
//! - The gain only scales separately filtered signals, so a fast attack never moves filter
//!   coefficients (no zipper noise, no ringing). That matters with zero lookahead.
//! - Cheap: three f64 biquads per channel (bell halves + one sidechain sharpening stage).
//!
//! The trade-off: a bell is softer at the band edges than a brick-wall band, so `freq`/`Q`
//! should cover the offending band. Default Q 1.0 → half-depth points at ≈ 4.3–11.3 kHz.
//!
//! **Onset gate (why the band level alone isn't enough).** With an absolute threshold the
//! de-esser fired on every bright drum snap (≈ 240/min on `music_like`, −7 dB in 5–9 kHz) and on
//! glottal-pulse HF in speech. What separates an "s" from those, with no lookahead, is how it
//! *starts*: a snap, a pluck's pick noise or a glottal pulse reaches its peak within ~1–2 ms and
//! then only decays (or, for a held bright tone or ringing string, stays flat); a fricative
//! keeps building for 10–100 ms. So once the sidechain crosses the bottom of the knee, a 2 ms
//! band-power envelope is watched: its maximum during the first 4 ms is the reference, and the
//! cut is allowed ("armed") only when the envelope climbs **3 dB above that reference** within
//! 60 ms. Armed, the 1 ms detector drives the gain exactly as before, until the band falls
//! below the knee. A sound that never climbs again is rejected until it fades below the knee
//! or rises 6 dB out of its trough (a new onset on top of it). 4 ms is about where the 2 ms
//! envelope of an instant-rise snap peaks (τ ≈ 8 ms decay); 3 dB is a little over 2 σ of that
//! envelope's noise on band-limited noise. All in linear power (no per-sample transcendental).
//!
//! Tried and rejected (measured): a band-vs-broadband dominance gate (snaps dominate the band
//! more than an "s" over voice); a "sustain" gate that rejects onsets whose envelope *drops*
//! from its peak within 10–15 ms (`music_like`'s snaps decay with τ ≈ 8 ms, too slowly to tell
//! apart in time, and glottal pulses make an "s" over a vowel look decaying); a long-term
//! relative threshold (snaps are as far above music's mean HF as an "s" is above speech's).
//!
//! **Zero lookahead** (project latency budget: de-smack already uses 40 ms of the 45 ms
//! ceiling). The gate arms ≈ 5–7 ms after the band crosses the knee, then the 1 ms attack
//! takes over. A slow "s" (raised-cosine, 120 ms) is still well before its peak (golden test:
//! ≥ 6 dB on the burst peak); an "s" that reaches full level in 10 ms loses its first peak
//! (≥ 3 dB, median ≈ 7 dB) but its body is cut fully (burst energy ≥ 6 dB). Limits of the gate:
//! an "s" must rise 3 dB above what was already in the band when it started (a quiet "s" under
//! louder bright-voice harmonics is left alone), and an affricate whose plosive burst is as loud
//! as its "s" ("ts", "ch") is not armed.
//!
//! **Real-time:** per sample, three biquads per channel, one `sqrt` and one division. Only
//! while the gate is armed does it take a `log10`/`exp` (gain computer); otherwise it
//! compares linear powers against precomputed thresholds. `freq`/`Q` changes glide in the
//! log domain at a 32-sample control rate (aligned to the absolute sample count, so output
//! is identical for any block size); threshold/range changes are smoothed by attack/release.
//!
//! **Telemetry:** gain reduction (block maximum, dB); one [`EventKind::Sibilance`] event per
//! burst (≥ 10 ms of more than 2 dB reduction) with start (chain-input clock), duration (until
//! the *target* gain is back above −0.5 dB, so the release tail isn't counted) and in-band
//! energy removed. `detail = [band centre Hz, sidechain band level dB (block max), threshold dB,
//! gate state at block end (0 idle, 1 watching an onset, 2 armed, 3 rejected)]`.

use crate::block::MAX_CHANNELS;
use crate::filters::{Biquad, one_pole_coeff};
use crate::module::{ParamDesc, ParamUnit};
use crate::telemetry::{Event, EventKind};
use crate::{AudioBlock, Module, ModuleId, TelemetryWriter};

/// Fixed compression ratio above the knee (de-essers run high ratios; `range` caps the depth).
const RATIO: f32 = 5.0;
/// Soft-knee width in dB.
const KNEE_DB: f32 = 6.0;
/// RMS window of the sidechain detector (smooths the noisy power of band-limited sibilance).
const RMS_TAU_S: f32 = 0.001;
/// Control rate for freq/Q glides (samples) and glide time constant.
const CTRL: u64 = 32;
const GLIDE_TAU_S: f32 = 0.02;
/// Event hysteresis: a burst starts when the (smoothed) gain passes −2 dB and ends when the
/// *target* gain has stayed above −0.5 dB for [`EVENT_HOLD_MS`] (so the reported length is the
/// sibilant's, not the release tail's).
const EVENT_ON_LIN: f32 = 0.794_328_2; // −2 dB
const EVENT_OFF_LIN: f32 = 0.944_060_9; // −0.5 dB
const EVENT_HOLD_MS: f32 = 10.0;
/// Onset gate (see the module docs): time constant of the shape envelope, the "attack window"
/// after the band crosses the knee whose peak becomes the reference, how far above that peak the
/// band must climb to count as a sibilant, how long it may take, and how far a rejected sound
/// must climb out of its trough to count as a new onset.
const SHAPE_TAU_S: f32 = 0.002;
const ONSET_WINDOW_MS: f32 = 4.0;
const RISE_LIN: f32 = 1.995_262_3; // +3 dB (power)
const RISE_WAIT_MS: f32 = 60.0;
const REONSET_LIN: f32 = 3.981_071_7; // +6 dB (power)
/// Bursts shorter than this aren't reported (still processed).
const MIN_EVENT_MS: f32 = 10.0;
/// A burst longer than this is reported in pieces (an event must end some time).
const MAX_EVENT_MS: f32 = 2000.0;
/// Gain this close to 1 snaps to exactly 1 (bit-exact passthrough at rest).
const SNAP: f32 = 1e-6;
/// Clamp detector input so absurd (finite) input can't overflow into inf/NaN.
const X_CLAMP: f32 = 1e4;
/// Highest band centre as a fraction of the sample rate (RBJ stays well-behaved).
const MAX_FREQ_FRAC: f32 = 0.45;

const PARAMS: [ParamDesc; 6] = [
    ParamDesc {
        key: "threshold",
        name: "Threshold",
        unit: ParamUnit::Db,
        min: -60.0,
        max: 0.0,
        default: -40.0,
        step: 0.0,
        choices: &[],
    },
    ParamDesc {
        key: "range",
        name: "Range",
        unit: ParamUnit::Db,
        min: 0.0,
        max: 24.0,
        default: 12.0,
        step: 0.0,
        choices: &[],
    },
    ParamDesc {
        key: "freq",
        name: "Frequency",
        unit: ParamUnit::Hz,
        min: 2000.0,
        max: 12_000.0,
        default: 7000.0,
        step: 0.0,
        choices: &[],
    },
    ParamDesc { key: "q", name: "Q", unit: ParamUnit::None, min: 0.5, max: 6.0, default: 1.0, step: 0.0, choices: &[] },
    ParamDesc {
        key: "attack",
        name: "Attack",
        unit: ParamUnit::Ms,
        min: 0.1,
        max: 20.0,
        default: 1.0,
        step: 0.0,
        choices: &[],
    },
    ParamDesc {
        key: "release",
        name: "Release",
        unit: ParamUnit::Ms,
        min: 10.0,
        max: 500.0,
        default: 60.0,
        step: 0.0,
        choices: &[],
    },
];

fn db_to_pow(db: f32) -> f32 {
    10f32.powf(db / 10.0)
}

/// Onset gate state. Reduction is only allowed while `Armed`.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Gate {
    /// Band below the knee.
    Idle,
    /// Band crossed the knee at `start`. `peak` = highest shape envelope during the first
    /// [`ONSET_WINDOW_MS`]; after that the band must climb [`RISE_LIN`] above it.
    Pending { start: u64, peak: f32 },
    /// Still rising after its onset: the gain computer drives the cut until the band drops
    /// below the knee.
    Armed,
    /// Peaked within the onset window and never rose again (snap, pluck, glottal pulse, held
    /// bright tone). Ignored until the band drops below the knee or climbs [`REONSET_LIN`] out
    /// of `trough`.
    Rejected { trough: f32 },
}

impl Gate {
    fn code(self) -> f32 {
        match self {
            Gate::Idle => 0.0,
            Gate::Pending { .. } => 1.0,
            Gate::Armed => 2.0,
            Gate::Rejected { .. } => 3.0,
        }
    }
}

/// One sibilance burst being tracked for telemetry.
#[derive(Debug, Clone, Copy, Default)]
struct Burst {
    active: bool,
    /// Module-input sample where it started.
    start: u64,
    /// Σ band power and Σ band power after the cut (≈ g²·p at the bell centre).
    sum_in: f64,
    sum_out: f64,
    /// Last sample whose target gain was below −0.5 dB (the burst's end so far).
    last: u64,
}

/// The de-ess module.
#[derive(Debug)]
pub struct DeEss {
    values: [f32; 6],
    sr: f32,
    channels: usize,
    /// Per channel: [0] feed-forward bell band, [1] feedback bell band, [2] second sidechain stage.
    bp: [[Biquad; 3]; MAX_CHANNELS],
    /// Current (gliding) and target log2 frequency / log2 Q.
    lf: f32,
    lq: f32,
    lf_target: f32,
    lq_target: f32,
    a_glide: f32,
    a_rms: f32,
    a_att: f32,
    a_rel: f32,
    /// Band power below which no reduction happens (bottom of the knee), linear power.
    knee_lo_pow: f32,
    /// Smallest linear gain (−range dB).
    min_gain: f32,
    env: f32,
    /// Shape envelope (2 ms band power) for the sustain gate.
    shape: f32,
    a_shape: f32,
    gate: Gate,
    /// Onset-gate times in samples.
    onset_window: u64,
    rise_wait: u64,
    g: f32,
    pos: u64,
    min_event: u64,
    max_event: u64,
    event_hold: u64,
    burst: Burst,
}

impl Default for DeEss {
    fn default() -> Self {
        Self::new()
    }
}

impl DeEss {
    /// De-ess with default parameters.
    pub fn new() -> Self {
        let mut m = Self {
            values: [0.0; 6],
            sr: 48_000.0,
            channels: 1,
            bp: [[Biquad::default(); 3]; MAX_CHANNELS],
            lf: 0.0,
            lq: 0.0,
            lf_target: 0.0,
            lq_target: 0.0,
            a_glide: 0.0,
            a_rms: 0.0,
            a_att: 0.0,
            a_rel: 0.0,
            knee_lo_pow: 0.0,
            min_gain: 1.0,
            env: 0.0,
            shape: 0.0,
            a_shape: 0.0,
            gate: Gate::Idle,
            onset_window: 1,
            rise_wait: 1,
            g: 1.0,
            pos: 0,
            min_event: 1,
            max_event: 1,
            event_hold: 1,
            burst: Burst::default(),
        };
        for (i, p) in PARAMS.iter().enumerate() {
            m.set_param(i, p.default);
        }
        m.lf = m.lf_target;
        m.lq = m.lq_target;
        m
    }

    fn threshold(&self) -> f32 {
        self.values[0]
    }

    fn update_dynamics(&mut self) {
        self.knee_lo_pow = db_to_pow(self.values[0] - 0.5 * KNEE_DB);
        self.min_gain = crate::db_to_amp(-self.values[1]);
    }

    fn update_times(&mut self) {
        self.a_att = one_pole_coeff(self.sr, self.values[4] / 1000.0);
        self.a_rel = one_pole_coeff(self.sr, self.values[5] / 1000.0);
    }

    /// Band centre in Hz for the current (gliding) state, kept below Nyquist.
    fn centre_hz(&self) -> f32 {
        self.lf.exp2().min(MAX_FREQ_FRAC * self.sr)
    }

    fn design(&self) -> Biquad {
        Biquad::bandpass(self.sr as f64, self.centre_hz() as f64, self.lq.exp2() as f64)
    }

    /// Move freq/Q one control step towards their targets and retune (only while gliding).
    #[inline]
    fn glide(&mut self) {
        let (df, dq) = (self.lf_target - self.lf, self.lq_target - self.lq);
        if df == 0.0 && dq == 0.0 {
            return;
        }
        if df.abs() < 1e-4 && dq.abs() < 1e-4 {
            self.lf = self.lf_target;
            self.lq = self.lq_target;
        } else {
            self.lf += self.a_glide * df;
            self.lq += self.a_glide * dq;
        }
        let d = self.design();
        for f in self.bp[..self.channels].iter_mut().flatten() {
            f.retune(&d);
        }
    }

    /// Target linear gain for sidechain power `env` (soft knee, ratio, capped at `range`).
    #[inline]
    fn target_gain(&self, env: f32) -> f32 {
        if env <= self.knee_lo_pow {
            return 1.0;
        }
        let over = 10.0 * env.log10() - self.threshold();
        let slope = 1.0 - 1.0 / RATIO;
        let gr = if over < 0.5 * KNEE_DB {
            let k = over + 0.5 * KNEE_DB;
            slope * k * k / (2.0 * KNEE_DB)
        } else {
            slope * over
        };
        crate::db_to_amp(-gr).max(self.min_gain)
    }

    /// Advance the onset gate by one sample (uses `self.env`, `self.shape`, `self.pos`).
    #[inline]
    fn update_gate(&mut self) {
        let (env, shape, pos, knee) = (self.env, self.shape, self.pos, self.knee_lo_pow);
        self.gate = match self.gate {
            Gate::Idle if env > knee => Gate::Pending { start: pos, peak: shape },
            Gate::Idle => Gate::Idle,
            _ if env <= knee && shape <= knee => Gate::Idle,
            Gate::Pending { start, peak } => {
                let age = pos - start;
                if age < self.onset_window {
                    Gate::Pending { start, peak: peak.max(shape) }
                } else if shape > peak * RISE_LIN {
                    Gate::Armed
                } else if age >= self.rise_wait {
                    Gate::Rejected { trough: shape }
                } else {
                    Gate::Pending { start, peak }
                }
            }
            Gate::Armed => Gate::Armed,
            Gate::Rejected { trough } if shape > trough * REONSET_LIN => Gate::Pending { start: pos, peak: shape },
            Gate::Rejected { trough } => Gate::Rejected { trough: trough.min(shape) },
        };
    }

    fn report(&mut self, telemetry: &mut TelemetryWriter, block_in_start: u64, chain_start: u64) {
        let b = self.burst;
        let dur = b.last + 1 - b.start;
        if dur >= self.min_event {
            telemetry.push_event(Event {
                module: ModuleId::DeEss,
                kind: EventKind::Sibilance,
                // Module-input clock → chain-input clock (offset of this block).
                start_sample: (chain_start as i64 + b.start as i64 - block_in_start as i64).max(0) as u64,
                duration_samples: dur.min(u32::MAX as u64) as u32,
                energy_removed_db: (10.0 * (b.sum_in / b.sum_out.max(1e-30)).log10()) as f32,
            });
        }
    }
}

impl Module for DeEss {
    fn id(&self) -> ModuleId {
        ModuleId::DeEss
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize, channels: usize) {
        self.sr = sample_rate;
        self.channels = channels.clamp(1, MAX_CHANNELS);
        self.a_glide = one_pole_coeff(sample_rate / CTRL as f32, GLIDE_TAU_S);
        self.a_rms = one_pole_coeff(sample_rate, RMS_TAU_S);
        self.a_shape = one_pole_coeff(sample_rate, SHAPE_TAU_S);
        let samples = |ms: f32| ((sample_rate * ms / 1000.0).round() as u64).max(1);
        self.min_event = samples(MIN_EVENT_MS);
        self.max_event = samples(MAX_EVENT_MS);
        self.event_hold = samples(EVENT_HOLD_MS);
        self.onset_window = samples(ONSET_WINDOW_MS);
        self.rise_wait = samples(RISE_WAIT_MS);
        self.update_times();
        self.update_dynamics();
        // A new format starts on target (no glide from the old settings).
        self.lf = self.lf_target;
        self.lq = self.lq_target;
        self.bp = [[self.design(); 3]; MAX_CHANNELS];
        self.reset();
    }

    fn reset(&mut self) {
        self.bp.iter_mut().flatten().for_each(Biquad::reset);
        self.env = 0.0;
        self.shape = 0.0;
        self.gate = Gate::Idle;
        self.g = 1.0;
        self.pos = 0;
        self.burst = Burst::default();
    }

    fn process(&mut self, block: &mut AudioBlock<'_>, telemetry: &mut TelemetryWriter) {
        let n = block.frames();
        let ch = block.channels().min(self.channels);
        if ch == 0 {
            return; // nothing to do (and 1/0 would poison the envelope)
        }
        let inv_ch = 1.0 / ch as f32;
        let block_in_start = self.pos;
        let chain_start = telemetry.block_start();
        let mut min_g = 1.0f32;
        let mut max_env = 0.0f32;
        for i in 0..n {
            if self.pos.is_multiple_of(CTRL) {
                self.glide();
            }
            // Band-pass every channel; linked detector on the mean (sharpened) band power.
            let mut band = [0.0f64; MAX_CHANNELS];
            let mut p = 0.0f32;
            for (c, b) in band.iter_mut().enumerate().take(ch) {
                let x = block.channel(c)[i].clamp(-X_CLAMP, X_CLAMP);
                let [ff, _, side] = &mut self.bp[c];
                *b = ff.tick(x as f64);
                let s = side.tick(*b) as f32;
                p += s * s;
            }
            p *= inv_ch;
            self.env += self.a_rms * (p - self.env);
            self.shape += self.a_shape * (p - self.shape);
            max_env = max_env.max(self.env);
            self.update_gate();

            let target = if self.gate == Gate::Armed { self.target_gain(self.env) } else { 1.0 };
            let a = if target < self.g { self.a_att } else { self.a_rel };
            self.g += a * (target - self.g);
            if target == 1.0 && self.g > 1.0 - SNAP {
                self.g = 1.0;
            }
            let g = self.g;
            // Bell with centre gain g = two √g halves: feed-forward x + (√g − 1)·bp(x), then the
            // same band in a delay-free feedback loop, 1 / (1 + (1/√g − 1)·bp).
            let s = g.sqrt() as f64;
            let (k, fb) = (s - 1.0, 1.0 / s - 1.0);
            for (c, b) in band.iter().enumerate().take(ch) {
                let x = block.channel(c)[i] as f64;
                let y = self.bp[c][1].tick_feedback(x + k * b, fb);
                if g < 1.0 {
                    block.channel_mut(c)[i] = y as f32;
                }
            }
            min_g = min_g.min(g);

            // Burst tracking for telemetry.
            if self.burst.active {
                self.burst.sum_in += p as f64;
                self.burst.sum_out += (g * g * p) as f64;
                if target < EVENT_OFF_LIN {
                    self.burst.last = self.pos;
                }
                if self.pos - self.burst.last >= self.event_hold || self.pos - self.burst.start >= self.max_event {
                    self.report(telemetry, block_in_start, chain_start);
                    self.burst.active = false;
                }
            } else if g < EVENT_ON_LIN && target < EVENT_OFF_LIN {
                self.burst = Burst {
                    active: true,
                    start: self.pos,
                    sum_in: p as f64,
                    sum_out: (g * g * p) as f64,
                    last: self.pos,
                };
            }
            self.pos += 1;
        }
        // Block maximum, so a short burst still shows on a meter polled every few blocks.
        telemetry.set_gain_reduction(ModuleId::DeEss, -crate::amp_to_db(min_g));
        let centre = self.centre_hz();
        let threshold = self.threshold();
        let m = telemetry.meters_mut(ModuleId::DeEss);
        m.detail = [centre, 10.0 * max_env.max(1e-20).log10(), threshold, self.gate.code()];
    }

    fn params(&self) -> &[ParamDesc] {
        &PARAMS
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let Some(p) = PARAMS.get(index) else { return };
        let v = p.clamp(value);
        self.values[index] = v;
        match index {
            0 | 1 => self.update_dynamics(),
            2 => self.lf_target = v.log2(),
            3 => self.lq_target = v.log2(),
            _ => self.update_times(),
        }
    }

    fn primary_param(&self) -> Option<usize> {
        Some(1) // range
    }

    fn param(&self, index: usize) -> f32 {
        self.values.get(index).copied().unwrap_or(f32::NAN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_constants_match_their_db() {
        assert!((EVENT_ON_LIN - crate::db_to_amp(-2.0)).abs() < 1e-6);
        assert!((EVENT_OFF_LIN - crate::db_to_amp(-0.5)).abs() < 1e-6);
    }

    #[test]
    fn gain_computer_has_soft_knee_ratio_and_range() {
        let mut m = DeEss::new();
        m.prepare(48_000.0, 64, 1);
        let gr = |m: &DeEss, level_db: f32| -crate::amp_to_db(m.target_gain(db_to_pow(level_db)));
        let t = m.threshold();
        assert_eq!(gr(&m, t - 10.0), 0.0);
        // Knee centre: slope·(W/2)²/(2W) with slope = 1 − 1/ratio; above the knee: slope·over.
        let slope = 1.0 - 1.0 / RATIO;
        assert!((gr(&m, t) - slope * 9.0 / 12.0).abs() < 1e-3, "{}", gr(&m, t));
        assert!((gr(&m, t + 8.0) - slope * 8.0).abs() < 1e-3, "{}", gr(&m, t + 8.0));
        assert!((gr(&m, t + 40.0) - 12.0).abs() < 1e-3, "range caps the depth");
        m.set_param(1, 0.0);
        assert_eq!(gr(&m, t + 40.0), 0.0, "range 0 = off");
    }

    #[test]
    fn freq_and_q_glide_to_target() {
        let mut m = DeEss::new();
        m.prepare(48_000.0, 64, 1);
        m.set_param(2, 4000.0);
        m.set_param(3, 3.0);
        assert!((m.centre_hz() - 7000.0).abs() < 1.0, "no jump");
        let mut t = TelemetryWriter::new();
        let mut buf = vec![0.0f32; 48_000];
        m.process(&mut AudioBlock::from_planar(&mut buf, 1), &mut t);
        assert!((m.centre_hz() - 4000.0).abs() < 0.5, "{}", m.centre_hz());
        assert!((m.lq.exp2() - 3.0).abs() < 1e-3);
    }

    #[test]
    fn params_clamp_and_nan_maps_to_default() {
        let mut m = DeEss::new();
        m.set_param(2, 1e9);
        assert_eq!(m.param(2), 12_000.0);
        m.set_param(0, f32::NAN);
        assert_eq!(m.param(0), PARAMS[0].default);
        m.set_param(99, 1.0);
        assert!(m.param(99).is_nan());
    }

    #[test]
    fn centre_stays_below_nyquist() {
        let mut m = DeEss::new();
        m.set_param(2, 12_000.0);
        m.prepare(22_050.0, 64, 1);
        assert!(m.centre_hz() <= 0.45 * 22_050.0);
    }

    #[test]
    fn huge_input_stays_finite() {
        let mut m = DeEss::new();
        m.prepare(48_000.0, 64, 2);
        let mut t = TelemetryWriter::new();
        let mut buf: Vec<f32> = (0..128).map(|i| if i % 2 == 0 { 1e30 } else { -1e30 }).collect();
        m.process(&mut AudioBlock::from_planar(&mut buf, 2), &mut t);
        assert!(m.env.is_finite() && m.g.is_finite());
    }

    /// 7 kHz tone burst with amplitude envelope `env(t)` (60 ms at 48 kHz).
    fn tone_burst(env: impl Fn(f32) -> f32) -> Vec<f32> {
        (0..2880)
            .map(|i| {
                let t = i as f32 / 48_000.0;
                env(t) * (core::f32::consts::TAU * 7000.0 * t).sin()
            })
            .collect()
    }

    #[test]
    fn onset_gate_ignores_decaying_transients_and_arms_on_rising_ones() {
        let run = |x: &[f32]| {
            let mut m = DeEss::new();
            m.prepare(48_000.0, 4096, 1);
            let mut t = TelemetryWriter::new();
            let mut y = x.to_vec();
            m.process(&mut AudioBlock::from_planar(&mut y, 1), &mut t);
            (y, m.gate)
        };
        // A snap: loud at once, then only decays → never armed, output bit-exact.
        let snap = tone_burst(|t| 0.3 * (-t / 0.005).exp());
        let (y, _) = run(&snap);
        assert_eq!(y, snap, "a decaying transient was touched");
        // An "s": keeps rising for 30 ms → armed and cut.
        let s = tone_burst(|t| 0.3 * (t / 0.03).min(1.0));
        let (y, gate) = run(&s);
        assert_eq!(gate, Gate::Armed);
        let tail = |v: &[f32]| v[2400..].iter().map(|x| x * x).sum::<f32>();
        assert!(tail(&y) < 0.1 * tail(&s), "rising burst not reduced");
    }
}
