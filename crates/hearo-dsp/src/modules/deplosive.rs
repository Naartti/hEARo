//! De-plosive: a dynamic low cut that engages only during "p"/"b" pops (HANDOVER §7.4).
//!
//! ```text
//! in ─┬───────────────────────────────────────────────┬─▶ (1 − d)·dry + d·HP ─▶ out
//!     ├─▶ high-pass 12 dB/oct @ cutoff (always running)┘          ▲
//!     ├─▶ LP 150 Hz ─▶ LF power ─┐                                 │ d (cut amount)
//!     └─▶ HP 300 Hz ─▶ HF power ─┴─▶ R = LF / HF ─▶ detector ─▶ ramp
//!                    slow LF / slow HF = R_bg ──────┘
//! ```
//!
//! A plosive pop is wind hitting the microphone: a burst of energy below ~150 Hz that arrives
//! within a few milliseconds and has almost nothing above it. So the detector does not look at
//! LF *level* alone (a bass line or a low voice is loud down there all the time) but mainly at
//! the spectral *balance* `R = LF power / power above 300 Hz`, on fast (1 ms attack, 12 ms
//! release) envelopes. It engages when all of these hold:
//! 1. **Balance jumps:** `R` is ≥ `threshold` dB above its slow background `R_bg` (the
//!    ratio of 0.4 s LF and HF means). Bass-heavy music and low voices have a high `R_bg`
//!    already, so their notes don't jump; a pop on speech moves `R` by 20–40 dB.
//! 2. **LF rises:** the LF level is ≥ 10 dB above its own 0.4 s mean. Without this, a syllable
//!    *fading out* (HF dies before LF) or a pause under a steady drone looks like a jump in `R`.
//! 3. **LF is a real part of the sound:** `R` ≥ −10 dB and LF power above −50 dBFS. A voice
//!    moving to a darker vowel shifts the balance too, but its fundamental stays well below
//!    its formants.
//!
//! The backgrounds learn only while nothing is being cut, so a pop can't raise its own reference.
//! The cut stays on while the burst lasts (≥ 15 ms hold, hysteresis 6 dB) and releases over
//! `release` ms. Anything still "bursting" after 150 ms is not a plosive (a new bass note or
//! drone starting from noise): the cut is released and the backgrounds adopt the new balance,
//! so the same sound no longer counts as a jump.
//!
//! **Repair** is a crossfade between the untouched input and a 12 dB/oct high-pass at `cutoff`
//! that runs continuously (so it has no start-up transient). The mix `d` ramps up over 1 ms and
//! down over `release` ms, and its maximum is set by `max cut` (`d_max = 1 − 10^(−cut/20)`).
//! When idle (`d = 0`) the output is the input, bit for bit. Zero lookahead: a pop rises over
//! 5–15 ms, so a 1 ms detector catches it early and only its first few milliseconds pass uncut
//! (project latency budget: de-smack uses 40 of the 45 ms).

use crate::filters::{Biquad, one_pole_coeff};
use crate::module::{ParamDesc, ParamUnit};
use crate::telemetry::{Event, EventKind};
use crate::{AudioBlock, Module, ModuleId, TelemetryWriter};

/// Detection band: plosive energy sits below this (spec §7.4).
const DET_LF_HZ: f64 = 150.0;
/// The "broadband" reference: everything above this.
const DET_HF_HZ: f64 = 300.0;
const ENV_ATTACK_S: f32 = 0.001;
/// Bridges the zero crossings of a 30–80 Hz pop (a power envelope dips each half cycle).
const ENV_RELEASE_S: f32 = 0.012;
/// Background balance (both bands): long enough that a ≤ 100 ms pop barely moves it.
const BG_TAU_S: f32 = 0.4;
/// Power floor added to both sides of every ratio (−100 dBFS): digital silence has no balance.
const EPS: f32 = 1e-10;
/// LF power that counts as audible (−50 dBFS).
const ABS_FLOOR: f32 = 1e-5;
/// The LF level itself must also jump this far above its own 0.4 s mean (a rise, not a decay).
const LF_RISE_LIN: f32 = 10.0; // +10 dB
/// …and LF must be a real part of the sound: `R` ≥ −10 dB.
const R_MIN_LIN: f32 = 0.1; // −10 dB
/// Release hysteresis below the trigger levels.
const HYST_LIN: f32 = 0.251_188_64; // −6 dB
const ATTACK_MS: f32 = 1.0;
/// The cut is a 12 dB/oct Butterworth high-pass. A 24 dB/oct one measured only 0.4 dB more
/// reduction (the uncut onset dominates) but has 180° of phase at the corner, so the dry/cut
/// crossfade would pass through a full notch there during every release; 12 dB/oct (90°) dips
/// at most 4.8 dB.
const CUT_Q: f64 = core::f64::consts::FRAC_1_SQRT_2;
const HOLD_MS: f32 = 15.0;
/// Longer than any plosive: a "burst" lasting this long is a new sustained sound.
const MAX_LEN_MS: f32 = 150.0;

const PARAMS: [ParamDesc; 4] = [
    ParamDesc {
        key: "threshold",
        name: "Threshold",
        unit: ParamUnit::Db,
        min: 6.0,
        max: 30.0,
        default: 12.0,
        step: 0.0,
        choices: &[],
    },
    ParamDesc {
        key: "cutoff",
        name: "Cutoff",
        unit: ParamUnit::Hz,
        min: 80.0,
        max: 250.0,
        default: 150.0,
        step: 0.0,
        choices: &[],
    },
    ParamDesc {
        key: "max_cut",
        name: "Max cut",
        unit: ParamUnit::Db,
        min: 3.0,
        max: 36.0,
        default: 24.0,
        step: 0.0,
        choices: &[],
    },
    ParamDesc {
        key: "release",
        name: "Release",
        unit: ParamUnit::Ms,
        min: 10.0,
        max: 300.0,
        default: 60.0,
        step: 0.0,
        choices: &[],
    },
];

/// Per-channel filter state.
#[derive(Debug, Clone, Copy, Default)]
struct Chan {
    lf: [Biquad; 2],
    hf: Biquad,
    cut: Biquad,
}

/// The de-plosive module.
#[derive(Debug)]
pub struct DePlosive {
    threshold_db: f32,
    cutoff_hz: f32,
    max_cut_db: f32,
    release_ms: f32,
    /// Derived from the params.
    threshold_lin: f32,
    d_max: f32,
    release_step: f32,
    /// Cutoff waiting to be applied (only while idle, so a running cut never jumps).
    pending_cutoff: bool,
    sr: f32,
    attack_step: f32,
    hold: u64,
    max_len: u64,
    a_att: f32,
    a_rel: f32,
    a_bg: f32,
    chans: Vec<Chan>,
    e_lf: f32,
    e_hf: f32,
    bg_lf: f32,
    bg_hf: f32,
    /// Cut engaged (detector side); `d` follows with ramps.
    active: bool,
    active_since: u64,
    /// Current mix towards the high-passed path, 0..=d_max.
    d: f32,
    pos: u64,
    /// Running event (while `d > 0`): chain-clock start, input/output energy.
    ev_start: u64,
    ev_len: u32,
    ev_in: f64,
    ev_out: f64,
}

impl Default for DePlosive {
    fn default() -> Self {
        Self::new()
    }
}

fn db_to_pow(db: f32) -> f32 {
    10f32.powf(db / 10.0)
}

impl DePlosive {
    /// De-plosive with default parameters.
    pub fn new() -> Self {
        let mut m = Self {
            threshold_db: PARAMS[0].default,
            cutoff_hz: PARAMS[1].default,
            max_cut_db: PARAMS[2].default,
            release_ms: PARAMS[3].default,
            threshold_lin: 1.0,
            d_max: 0.0,
            release_step: 1.0,
            pending_cutoff: false,
            sr: 48_000.0,
            attack_step: 1.0,
            hold: 0,
            max_len: 1,
            a_att: 1.0,
            a_rel: 1.0,
            a_bg: 1.0,
            chans: Vec::new(),
            e_lf: 0.0,
            e_hf: 0.0,
            bg_lf: 0.0,
            bg_hf: 0.0,
            active: false,
            active_since: 0,
            d: 0.0,
            pos: 0,
            ev_start: 0,
            ev_len: 0,
            ev_in: 0.0,
            ev_out: 0.0,
        };
        m.update_derived();
        m
    }

    fn samples(&self, ms: f32) -> u64 {
        (self.sr * ms / 1000.0).round() as u64
    }

    fn update_derived(&mut self) {
        self.threshold_lin = db_to_pow(self.threshold_db);
        self.d_max = 1.0 - crate::db_to_amp(-self.max_cut_db);
        // Release ramps from full cut to none in `release` ms.
        self.release_step = self.d_max / (self.samples(self.release_ms).max(1) as f32);
        self.attack_step = self.d_max / (self.samples(ATTACK_MS).max(1) as f32);
    }

    fn apply_cutoff(&mut self) {
        let sr = self.sr as f64;
        let f = self.cutoff_hz as f64;
        for c in &mut self.chans {
            // Keep the running state: only the coefficients change (applied while idle).
            let old = c.cut;
            c.cut = Biquad::highpass(sr, f, CUT_Q);
            c.cut.copy_state_from(&old);
        }
        self.pending_cutoff = false;
    }

    /// Feed one frame's band powers to the detector; returns whether the cut should be on.
    #[inline]
    fn detect(&mut self, p_lf: f32, p_hf: f32) -> bool {
        let i = self.pos;
        let a = if p_lf > self.e_lf { self.a_att } else { self.a_rel };
        self.e_lf += a * (p_lf - self.e_lf);
        let a = if p_hf > self.e_hf { self.a_att } else { self.a_rel };
        self.e_hf += a * (p_hf - self.e_hf);

        // Compare R = e_lf/e_hf with R_bg·k without dividing: e_lf·(bg_hf+ε) vs k·(bg_lf+ε)·(e_hf+ε).
        let lhs = self.e_lf * (self.bg_hf + EPS);
        let bg_r = (self.bg_lf + EPS) * (self.e_hf + EPS);
        let lf_rise = self.e_lf > LF_RISE_LIN * (self.bg_lf + EPS);
        let audible = self.e_lf > ABS_FLOOR && self.e_lf > R_MIN_LIN * (self.e_hf + EPS);
        let jump = lhs > self.threshold_lin * bg_r;

        if self.active {
            let age = i - self.active_since;
            if age > self.max_len {
                // Sustained: not a plosive. Release and adopt the new balance, so the same sound
                // no longer counts as a jump.
                self.active = false;
                self.bg_lf = self.e_lf;
                self.bg_hf = self.e_hf;
            } else if age >= self.hold {
                let holding = lhs > self.threshold_lin * HYST_LIN * bg_r
                    && self.e_lf > ABS_FLOOR * HYST_LIN
                    && self.e_lf > LF_RISE_LIN * HYST_LIN * (self.bg_lf + EPS);
                if !holding {
                    self.active = false;
                }
            }
        } else if jump && lf_rise && audible {
            self.active = true;
            self.active_since = i;
        }

        // Backgrounds learn only while nothing is being cut (a pop must not lift its own reference).
        if !self.active && self.d == 0.0 {
            self.bg_lf += self.a_bg * (self.e_lf - self.bg_lf);
            self.bg_hf += self.a_bg * (self.e_hf - self.bg_hf);
        }
        self.pos = i + 1;
        self.active
    }

    /// Close the running event (if any) and report it.
    fn finish_event(&mut self, telemetry: &mut TelemetryWriter) {
        let removed = if self.ev_out > 0.0 { 10.0 * (self.ev_in / self.ev_out).log10() } else { 0.0 };
        telemetry.push_event(Event {
            module: ModuleId::DePlosive,
            kind: EventKind::Plosive,
            start_sample: self.ev_start,
            duration_samples: self.ev_len,
            energy_removed_db: removed.max(0.0) as f32,
        });
    }
}

impl Module for DePlosive {
    fn id(&self) -> ModuleId {
        ModuleId::DePlosive
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize, channels: usize) {
        self.sr = sample_rate;
        let sr = sample_rate as f64;
        let [q1, q2] = Biquad::BUTTERWORTH4_Q;
        let chan = Chan {
            lf: [Biquad::lowpass(sr, DET_LF_HZ, q1), Biquad::lowpass(sr, DET_LF_HZ, q2)],
            hf: Biquad::highpass(sr, DET_HF_HZ, core::f64::consts::FRAC_1_SQRT_2),
            cut: Biquad::default(),
        };
        self.chans = vec![chan; channels.clamp(1, crate::MAX_CHANNELS)];
        self.apply_cutoff();
        self.a_att = one_pole_coeff(sample_rate, ENV_ATTACK_S);
        self.a_rel = one_pole_coeff(sample_rate, ENV_RELEASE_S);
        self.a_bg = one_pole_coeff(sample_rate, BG_TAU_S);
        self.hold = self.samples(HOLD_MS);
        self.max_len = self.samples(MAX_LEN_MS);
        self.update_derived();
        self.reset();
    }

    fn reset(&mut self) {
        for c in &mut self.chans {
            c.lf.iter_mut().for_each(Biquad::reset);
            c.hf.reset();
            c.cut.reset();
        }
        if self.pending_cutoff {
            self.apply_cutoff();
        }
        self.e_lf = 0.0;
        self.e_hf = 0.0;
        self.bg_lf = 0.0;
        self.bg_hf = 0.0;
        self.active = false;
        self.active_since = 0;
        self.d = 0.0;
        self.pos = 0;
        self.ev_len = 0;
        self.ev_in = 0.0;
        self.ev_out = 0.0;
    }

    fn process(&mut self, block: &mut AudioBlock<'_>, telemetry: &mut TelemetryWriter) {
        let n = block.frames();
        let ch = block.channels().min(self.chans.len());
        let chain_start = telemetry.block_start();
        let mut max_d = 0.0f32;
        for i in 0..n {
            let (mut p_lf, mut p_hf) = (0.0f32, 0.0f32);
            let mut hp = [0.0f32; crate::MAX_CHANNELS];
            for (c, h) in hp.iter_mut().enumerate().take(ch) {
                // Clamp so absurd input can't overflow the power envelopes, and keep NaN/inf out
                // of the filter states (the sample itself passes through untouched while idle).
                let v = block.channel(c)[i];
                let x = if v.is_finite() { v.clamp(-1e4, 1e4) as f64 } else { 0.0 };
                let st = &mut self.chans[c];
                let lf = st.lf[0].tick(x);
                let lf = st.lf[1].tick(lf) as f32;
                let hf = st.hf.tick(x) as f32;
                *h = st.cut.tick(x) as f32;
                p_lf += lf * lf;
                p_hf += hf * hf;
            }
            // Mean channel power, so the absolute floors don't shift with the channel count.
            let inv = 1.0 / ch.max(1) as f32;
            let on = self.detect(p_lf * inv, p_hf * inv);
            let was_idle = self.d == 0.0;
            let target = if on { self.d_max } else { 0.0 };
            self.d = if self.d < target {
                (self.d + self.attack_step).min(target)
            } else {
                // Release, or max cut lowered mid-event: glide down at the release rate.
                (self.d - self.release_step).max(target)
            };
            if self.d > 0.0 {
                if was_idle {
                    self.ev_start = chain_start + i as u64;
                    self.ev_len = 0;
                    self.ev_in = 0.0;
                    self.ev_out = 0.0;
                }
                let d = self.d;
                for (c, &h) in hp.iter().enumerate().take(ch) {
                    let s = &mut block.channel_mut(c)[i];
                    let x = *s;
                    let y = x + d * (h - x);
                    self.ev_in += (x as f64) * (x as f64);
                    self.ev_out += (y as f64) * (y as f64);
                    *s = y;
                }
                self.ev_len = self.ev_len.saturating_add(1);
                max_d = max_d.max(d);
            } else if !was_idle {
                self.finish_event(telemetry);
                if self.pending_cutoff {
                    self.apply_cutoff();
                }
            }
        }
        // Block maximum of the cut at low frequencies (where the high-pass removes everything).
        telemetry.set_gain_reduction(ModuleId::DePlosive, -crate::amp_to_db(1.0 - max_d));
    }

    fn params(&self) -> &[ParamDesc] {
        &PARAMS
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let Some(p) = PARAMS.get(index) else { return };
        let v = p.clamp(value);
        match index {
            0 => self.threshold_db = v,
            1 => {
                if v != self.cutoff_hz {
                    self.cutoff_hz = v;
                    if self.d == 0.0 {
                        self.apply_cutoff();
                    } else {
                        self.pending_cutoff = true;
                    }
                }
            }
            2 => self.max_cut_db = v,
            _ => self.release_ms = v,
        }
        self.update_derived();
    }

    fn primary_param(&self) -> Option<usize> {
        Some(2) // max_cut
    }

    fn param(&self, index: usize) -> f32 {
        match index {
            0 => self.threshold_db,
            1 => self.cutoff_hz,
            2 => self.max_cut_db,
            3 => self.release_ms,
            _ => f32::NAN,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_constants_match_their_db() {
        let close = |a: f32, b: f32| (a - b).abs() < 1e-5 * b.max(1.0);
        assert!(close(LF_RISE_LIN, db_to_pow(10.0)));
        assert!(close(HYST_LIN, db_to_pow(-6.0)));
    }

    #[test]
    fn params_update_derived_values() {
        let mut m = DePlosive::new();
        m.prepare(48_000.0, 64, 1);
        m.set_param(0, 20.0);
        m.set_param(2, 20.0);
        assert!((m.threshold_lin - 100.0).abs() < 1e-3);
        assert!((m.d_max - 0.9).abs() < 1e-6);
        m.set_param(1, 1e9);
        assert_eq!(m.param(1), 250.0);
        assert!(m.param(9).is_nan());
    }

    /// Changing max cut, release or cutoff in the middle of a pop never steps the mix.
    #[test]
    fn param_changes_mid_cut_glide() {
        use hearo_testkit::Rng;
        use hearo_testkit::signals::{add_at, pink, plosive};
        let sr = 48_000.0;
        let mut x = pink(&mut Rng::new(1), 57_600, 1e-3);
        add_at(&mut x, &plosive(sr, &mut Rng::new(2), 0.5), 48_000);
        let mut m = DePlosive::new();
        m.prepare(sr, 1, 1);
        let mut t = TelemetryWriter::new();
        let (mut prev, mut max_step, mut engaged) = (0.0f32, 0.0f32, false);
        for (i, v) in x.iter().enumerate() {
            if engaged && i % 50 == 0 {
                let k = (i / 50) % 3;
                m.set_param(2, [3.0, 36.0, 12.0][k]);
                m.set_param(1, [80.0, 250.0, 150.0][k]);
                m.set_param(3, [10.0, 300.0, 60.0][k]);
            }
            let mut s = [*v];
            m.process(&mut AudioBlock::from_planar(&mut s, 1), &mut t);
            engaged |= m.d > 0.5;
            max_step = max_step.max((m.d - prev).abs());
            prev = m.d;
        }
        assert!(engaged, "the pop must engage the cut");
        let limit = PARAMS[2].max; // steepest legal ramp: full 36 dB cut in 1 ms
        let limit = (1.0 - crate::db_to_amp(-limit)) / (sr * ATTACK_MS / 1000.0) + 1e-6;
        assert!(max_step <= limit, "mix stepped {max_step} in one sample (limit {limit})");
    }

    #[test]
    fn nan_input_does_not_poison_the_filters() {
        let mut m = DePlosive::new();
        m.prepare(48_000.0, 64, 1);
        let mut t = TelemetryWriter::new();
        let mut buf = [f32::NAN, f32::INFINITY, 0.1, 0.2];
        m.process(&mut AudioBlock::from_planar(&mut buf, 1), &mut t);
        assert!(m.e_lf.is_finite() && m.e_hf.is_finite());
        assert!(m.chans[0].cut.tick(0.0).is_finite());
        assert_eq!(&buf[2..], &[0.1, 0.2], "idle output is the input");
    }

    #[test]
    fn huge_input_does_not_poison_the_detector() {
        let mut m = DePlosive::new();
        m.prepare(48_000.0, 64, 1);
        let mut t = TelemetryWriter::new();
        let mut buf = vec![1e30f32; 64];
        m.process(&mut AudioBlock::from_planar(&mut buf, 1), &mut t);
        assert!(m.e_lf.is_finite() && m.e_hf.is_finite() && m.bg_lf.is_finite());
    }
}
