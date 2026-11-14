//! De-hum: DC blocker + automatic 50/60 Hz mains-hum detection with frequency tracking + a gated
//! notch comb (HANDOVER §7.1; reworked after dsp-reviewer findings, see docs/PROGRESS.md).
//!
//! ```text
//! in ─▶ DC blocker (2 Hz) ─┬─▶ notch k·f̂0 (2 Hz wide), each mixed in by its own depth d_k ─▶ out
//!                         └─▶ mid/side ─▶ LP 250 Hz ─▶ ↓32 ─▶ detector banks (k = 1..3, f0 ∈ {50, 60})
//!                                                       └─▶ complex demodulators → f̂0 (tracks drift)
//! ```
//!
//! *Detection.* Hum is a stationary tone: over a long window its harmonics carry far more power
//! than frequencies right next to them, while speech moves its pitch. Per harmonic k = 1..3 we
//! compare 0.3 Hz-wide band power at k·f0 against ±12 % side bands (mean channel power, so hum
//! that differs between channels still counts). **At least two harmonics must each pass** the
//! threshold — a single held bass note (one tonal line) can't engage the notches. Detection runs
//! on a 32× decimated signal, which makes it ~30× cheaper.
//!
//! *Tracking.* Mains frequency drifts (49.9–50.1 Hz); at the 5th harmonic that's ±0.5 Hz — enough
//! to slide out of a 2 Hz notch. Complex demodulators at k·f0 measure the phase drift of the
//! strongest harmonic and the notches are retuned (state kept) to k·f̂0.
//!
//! *Repair.* Up to 8 cascaded notches, each mixed in by its own smoothed depth, so engaging,
//! changing the Harmonics parameter, or retuning never clicks. The depth target is the `amount`
//! parameter (default 1 = full notch; the simple UI's "how much"): a hum harmonic is reduced by
//! −20·log10(1 − amount) dB, which is also what the gain-reduction meter reports (capped by the
//! tonal score: never more than the hum stood out).

use crate::filters::{Biquad, one_pole_coeff};
use crate::module::{ParamChoice, ParamDesc, ParamUnit};
use crate::telemetry::{Event, EventKind};
use crate::{AudioBlock, Module, ModuleId, TelemetryWriter};

/// Maximum harmonics notched (including the fundamental).
pub const MAX_HARMONICS: usize = 8;
const DET_K: usize = 3;
const CANDIDATES: [f64; 2] = [50.0, 60.0];
const DECIM: usize = 32;
const NOTCH_BW_HZ: f64 = 2.0;
const DETECT_BW_HZ: f64 = 0.3;
const SIDE_OFFSET: f64 = 0.12;
const DETECT_TAU_S: f64 = 1.5;
const DEPTH_TAU_S: f32 = 0.25;
const DC_CUTOFF_HZ: f32 = 2.0;
const HYSTERESIS_DB: f32 = 4.0;
const MIN_HUM_DB: f32 = -80.0;
const MIN_TONAL_HARMONICS: usize = 2;
/// Hum must be present this long before the notches engage.
const PERSIST_S: f32 = 0.5;
const TRACK_TAU_S: f64 = 0.4;
/// Per-update smoothing of the frequency estimate (updates every 100 ms → τ ≈ 2 s; mains drifts slowly).
const TRACK_SMOOTH: f64 = 0.05;
const TRACK_EVERY_S: f64 = 0.1;
const MAX_DRIFT_HZ: f64 = 0.6;
const RETUNE_HZ: f64 = 0.002;
/// Input clamp (far above any real signal; keeps every state finite).
const X_CLAMP: f32 = 1e4;

const MAINS_CHOICES: &[ParamChoice] = &[
    ParamChoice { value: 0.0, label: "Auto" },
    ParamChoice { value: 50.0, label: "50 Hz" },
    ParamChoice { value: 60.0, label: "60 Hz" },
];

const PARAMS: [ParamDesc; 4] = [
    ParamDesc {
        key: "mains",
        name: "Mains",
        unit: ParamUnit::Hz,
        min: 0.0,
        max: 60.0,
        default: 0.0,
        step: 0.0,
        choices: MAINS_CHOICES,
    },
    ParamDesc {
        key: "harmonics",
        name: "Harmonics",
        unit: ParamUnit::None,
        min: 1.0,
        max: 8.0,
        default: 8.0,
        step: 1.0,
        choices: &[],
    },
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
    // How deep the notches go once hum is found: 1 = full notch (default), 0 = detect only.
    ParamDesc {
        key: "amount",
        name: "Amount",
        unit: ParamUnit::Percent,
        min: 0.0,
        max: 1.0,
        default: 1.0,
        step: 0.0,
        choices: &[],
    },
];

/// One candidate fundamental: band-pass detectors for mid and side, plus demodulators.
#[derive(Debug, Clone, Copy, Default)]
struct Bank {
    /// [signal: mid/side][harmonic][centre, low side, high side]
    filters: [[[Biquad; 3]; DET_K]; 2],
    /// Smoothed power summed over mid + side: [harmonic][centre, low, high].
    power: [[f64; 3]; DET_K],
    /// Complex demodulator per harmonic, for mid and side: rotating phasor + low-passed product.
    rot: [(f64, f64); DET_K],
    step: [(f64, f64); DET_K],
    z: [[(f64, f64); DET_K]; 2],
    last_phase: [[f64; DET_K]; 2],
    f_est: f64,
}

impl Bank {
    fn new(fd: f64, f0: f64) -> Self {
        let mut b = Bank { f_est: f0, ..Default::default() };
        for sig in &mut b.filters {
            for (k, row) in sig.iter_mut().enumerate() {
                let fc = f0 * (k + 1) as f64;
                for (j, f) in [fc, fc * (1.0 - SIDE_OFFSET), fc * (1.0 + SIDE_OFFSET)].into_iter().enumerate() {
                    row[j] = Biquad::bandpass(fd, f, f / DETECT_BW_HZ);
                }
            }
        }
        for k in 0..DET_K {
            let w = -2.0 * core::f64::consts::PI * f0 * (k + 1) as f64 / fd;
            b.step[k] = (w.cos(), w.sin());
            b.rot[k] = (1.0, 0.0);
        }
        b
    }

    #[inline]
    fn tick(&mut self, mid: f64, side: f64, a: f64, az: f64) {
        for k in 0..DET_K {
            for j in 0..3 {
                let m = self.filters[0][k][j].tick(mid);
                let s = self.filters[1][k][j].tick(side);
                self.power[k][j] += a * (m * m + s * s - self.power[k][j]);
            }
            // Demodulate mid and side at k·f0: z += az·(x·e^{−jωn} − z).
            let (c, s) = self.rot[k];
            for (z, x) in self.z.iter_mut().zip([mid, side]) {
                z[k].0 += az * (x * c - z[k].0);
                z[k].1 += az * (x * s - z[k].1);
            }
            let (sc, ss) = self.step[k];
            self.rot[k] = (c * sc - s * ss, c * ss + s * sc);
        }
    }

    /// Renormalise the phasors (they drift by rounding over millions of steps).
    fn renorm(&mut self) {
        for r in &mut self.rot {
            let m = (r.0 * r.0 + r.1 * r.1).sqrt();
            if m > 0.0 {
                *r = (r.0 / m, r.1 / m);
            }
        }
    }

    /// Per-harmonic score (dB) and the hum level estimate (dBFS sine amplitude).
    fn scores(&self) -> ([f32; DET_K], f32) {
        let mut s = [0.0f32; DET_K];
        let mut tonal = 0.0;
        for (k, p) in self.power.iter().enumerate() {
            let side = 0.5 * (p[1] + p[2]);
            s[k] = (10.0 * (p[0].max(1e-30) / side.max(1e-30)).log10()) as f32;
            tonal += p[0];
        }
        // power = mid² + side² = mean channel power; a sine of amplitude A has power A²/2.
        (s, (10.0 * (2.0 * tonal).max(1e-30).log10()) as f32)
    }

    /// Update the frequency estimate from the phase drift of the *cleanest* harmonic (highest
    /// tonal score — usually the fundamental, where speech has little energy) over `dt`.
    fn track(&mut self, f_nom: f64, dt: f64) {
        let (scores, _) = self.scores();
        let k = (0..DET_K).max_by(|a, b| scores[*a].total_cmp(&scores[*b])).unwrap_or(0);
        // Mid or side, whichever carries more of that harmonic (out-of-phase hum lives in side).
        let sig = usize::from(self.z[1][k].0.hypot(self.z[1][k].1) > self.z[0][k].0.hypot(self.z[0][k].1));
        let z = self.z[sig][k];
        let phase = z.1.atan2(z.0);
        let mut d = phase - self.last_phase[sig][k];
        while d > core::f64::consts::PI {
            d -= 2.0 * core::f64::consts::PI;
        }
        while d < -core::f64::consts::PI {
            d += 2.0 * core::f64::consts::PI;
        }
        for (zs, ps) in self.z.iter().zip(self.last_phase.iter_mut()) {
            for (z, p) in zs.iter().zip(ps.iter_mut()) {
                *p = z.1.atan2(z.0);
            }
        }
        // e^{−jωn} demodulation: a tone at f_nom + δ rotates by +2πδ per second.
        let offset = d / (2.0 * core::f64::consts::PI * dt) / (k + 1) as f64;
        if !offset.is_finite() {
            return; // poisoned detector: keep the last good estimate
        }
        let offset = offset.clamp(-MAX_DRIFT_HZ, MAX_DRIFT_HZ);
        self.f_est += TRACK_SMOOTH * (f_nom + offset - self.f_est);
    }

    fn reset(&mut self, f0: f64) {
        self.filters.iter_mut().flatten().flatten().for_each(Biquad::reset);
        self.power = [[0.0; 3]; DET_K];
        self.rot = [(1.0, 0.0); DET_K];
        self.z = [[(0.0, 0.0); DET_K]; 2];
        self.last_phase = [[0.0; DET_K]; 2];
        self.f_est = f0;
    }
}

/// The de-hum module.
#[derive(Debug)]
pub struct DeHum {
    mains: f32,
    harmonics: usize,
    threshold_db: f32,
    amount: f32,
    sr: f32,
    fd: f64,
    banks: [Bank; 2],
    lp: [[Biquad; 2]; 2],
    decim_count: usize,
    det_a: f64,
    det_az: f64,
    track_count: usize,
    track_every: usize,
    depth_a: f32,
    dc_r: f32,
    dc: Vec<(f32, f32)>,
    notches: Vec<[Biquad; MAX_HARMONICS]>,
    notch_f0: f64,
    notch_nominal: usize,
    depth: [f32; MAX_HARMONICS],
    engaged: bool,
    present_since: f32,
    hum_db: f32,
    score_db: f32,
}

impl Default for DeHum {
    fn default() -> Self {
        Self::new()
    }
}

impl DeHum {
    /// De-hum with default parameters (auto 50/60 Hz, 8 harmonics, 12 dB per-harmonic threshold).
    pub fn new() -> Self {
        Self {
            mains: PARAMS[0].default,
            harmonics: PARAMS[1].default as usize,
            threshold_db: PARAMS[2].default,
            amount: PARAMS[3].default,
            sr: 48_000.0,
            fd: 1500.0,
            banks: [Bank::default(); 2],
            lp: [[Biquad::default(); 2]; 2],
            decim_count: 0,
            det_a: 0.0,
            det_az: 0.0,
            track_count: 0,
            track_every: 1,
            depth_a: 0.0,
            dc_r: 0.0,
            dc: Vec::new(),
            notches: Vec::new(),
            notch_f0: CANDIDATES[0],
            notch_nominal: 0,
            depth: [0.0; MAX_HARMONICS],
            engaged: false,
            present_since: 0.0,
            hum_db: -200.0,
            score_db: 0.0,
        }
    }

    fn notch_for(&self, k: usize, f0: f64) -> Biquad {
        let sr = self.sr as f64;
        let f = f0 * (k + 1) as f64;
        // Above 0.45·fs (or for a bad f0) pass through — never all-zero coefficients, which mute.
        if f.is_finite() && f > 0.0 && f < 0.45 * sr { Biquad::notch(sr, f, f / NOTCH_BW_HZ) } else { Biquad::IDENTITY }
    }

    /// Retune all notches to `f0`, keeping filter state (no click).
    fn tune_notches(&mut self, f0: f64) {
        if !f0.is_finite() {
            return;
        }
        let designs: [Biquad; MAX_HARMONICS] = core::array::from_fn(|k| self.notch_for(k, f0));
        for chan in &mut self.notches {
            for (n, d) in chan.iter_mut().zip(&designs) {
                n.retune(d);
            }
        }
        self.notch_f0 = f0;
    }

    /// Decide which mains frequency (if any) is present; update tracking and retune.
    fn detect(&mut self, telemetry: &mut TelemetryWriter, frames: usize) {
        let (s50, l50) = self.banks[0].scores();
        let (s60, l60) = self.banks[1].scores();
        let thr = if self.engaged { self.threshold_db - HYSTERESIS_DB } else { self.threshold_db };
        let tonal = |s: &[f32; DET_K]| s.iter().filter(|&&v| v > thr).count();
        let total = |s: &[f32; DET_K]| s.iter().map(|v| v.max(0.0)).sum::<f32>();
        let pick = match self.mains {
            m if m >= 55.0 => 1,
            m if m >= 25.0 => 0,
            _ if (tonal(&s60), total(&s60)) > (tonal(&s50), total(&s50)) => 1,
            _ => 0,
        };
        let (s, level) = if pick == 0 { (s50, l50) } else { (s60, l60) };
        self.score_db = s.iter().copied().fold(f32::MIN, f32::max);
        self.hum_db = level;
        let present = tonal(&s) >= MIN_TONAL_HARMONICS && level > MIN_HUM_DB;
        let dt = frames as f32 / self.sr;
        self.present_since = if present { self.present_since + dt } else { 0.0 };
        let hum = present && (self.engaged || self.present_since >= PERSIST_S);
        if hum && !self.engaged {
            telemetry.push_event(Event {
                module: ModuleId::DeHum,
                kind: EventKind::Hum,
                start_sample: telemetry.block_start(),
                duration_samples: frames as u32,
                energy_removed_db: self.score_db,
            });
        }
        self.engaged = hum && pick == self.notch_nominal;
        let all_out = self.depth.iter().all(|d| *d < 1e-3);
        if pick != self.notch_nominal && all_out {
            // Switch mains standard only once the notches are fully out of the signal.
            self.notch_nominal = pick;
            self.tune_notches(self.banks[pick].f_est);
        } else if pick == self.notch_nominal {
            let f = self.banks[pick].f_est;
            if (f - self.notch_f0).abs() > RETUNE_HZ {
                self.tune_notches(f);
            }
        }
    }
}

impl Module for DeHum {
    fn id(&self) -> ModuleId {
        ModuleId::DeHum
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize, channels: usize) {
        self.sr = sample_rate;
        let sr = sample_rate as f64;
        self.fd = sr / DECIM as f64;
        self.banks = [Bank::new(self.fd, CANDIDATES[0]), Bank::new(self.fd, CANDIDATES[1])];
        self.lp = [[Biquad::lowpass(sr, 250.0, 0.707); 2]; 2];
        self.det_a = 1.0 - (-1.0 / (DETECT_TAU_S * self.fd)).exp();
        self.det_az = 1.0 - (-1.0 / (TRACK_TAU_S * self.fd)).exp();
        self.track_every = ((TRACK_EVERY_S * self.fd).round() as usize).max(1);
        self.depth_a = one_pole_coeff(sample_rate, DEPTH_TAU_S);
        self.dc_r = (-2.0 * core::f32::consts::PI * DC_CUTOFF_HZ / sample_rate).exp();
        self.dc = vec![(0.0, 0.0); channels];
        self.notches = vec![[Biquad::default(); MAX_HARMONICS]; channels];
        self.notch_nominal = 0;
        let designs: [Biquad; MAX_HARMONICS] = core::array::from_fn(|k| self.notch_for(k, CANDIDATES[0]));
        self.notches.fill(designs);
        self.notch_f0 = CANDIDATES[0];
        self.reset();
    }

    fn reset(&mut self) {
        for (b, f0) in self.banks.iter_mut().zip(CANDIDATES) {
            b.reset(f0);
        }
        self.lp.iter_mut().flatten().for_each(Biquad::reset);
        self.decim_count = 0;
        self.track_count = 0;
        self.dc.iter_mut().for_each(|s| *s = (0.0, 0.0));
        self.notches.iter_mut().flatten().for_each(Biquad::reset);
        self.depth = [0.0; MAX_HARMONICS];
        self.engaged = false;
        self.present_since = 0.0;
        self.hum_db = -200.0;
        self.score_db = 0.0;
        // Re-derive notch coefficients from the (reset) estimate so nothing stale survives.
        let f = self.banks[self.notch_nominal.min(1)].f_est;
        self.tune_notches(f);
    }

    fn process(&mut self, block: &mut AudioBlock<'_>, telemetry: &mut TelemetryWriter) {
        let n = block.frames();
        let ch = block.channels().min(self.dc.len());
        if ch == 0 {
            return;
        }

        // DC blocker (always on). Input clamped so absurd finite values can't overflow its state.
        for c in 0..ch {
            let (mut x1, mut y1) = self.dc[c];
            for x in block.channel_mut(c).iter_mut() {
                *x = x.clamp(-X_CLAMP, X_CLAMP);
                let y = *x - x1 + self.dc_r * y1;
                x1 = *x;
                y1 = y;
                *x = y;
            }
            self.dc[c] = (x1, y1);
        }

        // Detector: mid/side → low-pass → decimate → banks + demodulators.
        for i in 0..n {
            let (mid, side) = match ch {
                1 => (block.channel(0)[i] as f64, 0.0),
                _ => {
                    let (l, r) = (block.channel(0)[i] as f64, block.channel(1)[i] as f64);
                    (0.5 * (l + r), 0.5 * (l - r))
                }
            };
            let (mut m, mut s) = (mid, side);
            for f in &mut self.lp[0] {
                m = f.tick(m);
            }
            for f in &mut self.lp[1] {
                s = f.tick(s);
            }
            self.decim_count += 1;
            if self.decim_count == DECIM {
                self.decim_count = 0;
                for b in &mut self.banks {
                    b.tick(m, s, self.det_a, self.det_az);
                }
                self.track_count += 1;
                if self.track_count == self.track_every {
                    self.track_count = 0;
                    let dt = self.track_every as f64 / self.fd;
                    for (b, f0) in self.banks.iter_mut().zip(CANDIDATES) {
                        b.track(f0, dt);
                        b.renorm();
                    }
                }
            }
        }
        self.detect(telemetry, n);

        // Notch comb, each stage mixed in by its own smoothed depth.
        let targets: [f32; MAX_HARMONICS] =
            core::array::from_fn(|k| if self.engaged && k < self.harmonics { self.amount } else { 0.0 });
        let idle = self.depth.iter().zip(&targets).all(|(d, t)| *d < 1e-4 && *t == 0.0);
        if idle {
            self.depth = [0.0; MAX_HARMONICS];
            self.notches.iter_mut().flatten().for_each(Biquad::reset);
        } else {
            let a = self.depth_a;
            let start = self.depth;
            for c in 0..ch {
                let mut d = start;
                let notches = &mut self.notches[c];
                for x in block.channel_mut(c).iter_mut() {
                    let mut y = *x as f64;
                    for k in 0..MAX_HARMONICS {
                        d[k] += a * (targets[k] - d[k]);
                        if d[k] > 1e-5 {
                            let notched = notches[k].tick(y);
                            y += d[k] as f64 * (notched - y);
                        }
                    }
                    *x = (y as f32).clamp(-X_CLAMP, X_CLAMP);
                }
                self.depth = d;
            }
        }

        let mean_depth = self.depth[..self.harmonics].iter().sum::<f32>() / self.harmonics.max(1) as f32;
        let m = telemetry.meters_mut(ModuleId::DeHum);
        m.detail = [if self.engaged { self.notch_f0 as f32 } else { 0.0 }, self.hum_db, mean_depth, self.score_db];
        // The notch takes a mixed-in tone down by 1 / (1 − depth), and never by more than the hum
        // stood above its surroundings (the tonal score).
        m.gain_reduction_db = (-crate::amp_to_db(1.0 - mean_depth.min(1.0))).min(self.score_db.max(0.0)).max(0.0);
    }

    fn params(&self) -> &[ParamDesc] {
        &PARAMS
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let Some(p) = PARAMS.get(index) else { return };
        // `snap`: mains is one of Auto / 50 / 60, harmonics a whole number (same rule as settings).
        let v = p.snap(value);
        match index {
            0 => self.mains = v,
            1 => self.harmonics = v as usize,
            2 => self.threshold_db = v,
            _ => self.amount = v,
        }
    }

    fn primary_param(&self) -> Option<usize> {
        Some(3) // amount
    }

    fn param(&self, index: usize) -> f32 {
        match index {
            0 => self.mains,
            1 => self.harmonics as f32,
            2 => self.threshold_db,
            3 => self.amount,
            _ => f32::NAN,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn survives_absurd_input_and_recovers_after_reset() {
        let sr = 48_000.0;
        let mut m = DeHum::new();
        m.prepare(sr, 64, 1);
        let mut t = TelemetryWriter::new();
        let mut buf: Vec<f32> = (0..64).map(|i| if i % 2 == 0 { 3e38 } else { -3e38 }).collect();
        for _ in 0..100 {
            m.process(&mut AudioBlock::from_planar(&mut buf, 1), &mut t);
        }
        assert!(buf.iter().all(|x| x.is_finite()), "output must stay finite");
        m.reset();
        assert!(m.banks.iter().all(|b| b.f_est.is_finite()) && m.notch_f0.is_finite());
        // Afterwards it still passes ordinary audio.
        let mut ok: Vec<f32> = (0..64).map(|i| (i as f32 * 0.1).sin() * 0.3).collect();
        m.process(&mut AudioBlock::from_planar(&mut ok, 1), &mut t);
        assert!(ok.iter().any(|x| x.abs() > 0.01), "silenced after recovery");
    }

    #[test]
    fn mains_param_is_stepped() {
        let mut m = DeHum::new();
        for (v, want) in [(10.0, 0.0), (48.0, 50.0), (57.0, 60.0), (60.0, 60.0)] {
            m.set_param(0, v);
            assert_eq!(m.param(0), want);
        }
    }

    #[test]
    fn tracks_a_drifting_fundamental() {
        let sr = 48_000.0;
        let mut m = DeHum::new();
        m.prepare(sr, 512, 1);
        let mut t = TelemetryWriter::new();
        let f = 50.12f32;
        let mut buf = vec![0.0f32; 512];
        let mut n = 0u64;
        for _ in 0..(sr as usize * 6 / 512) {
            for x in &mut buf {
                *x = 0.05 * (2.0 * core::f32::consts::PI * f * n as f32 / sr).sin();
                n += 1;
            }
            m.process(&mut AudioBlock::from_planar(&mut buf, 1), &mut t);
        }
        assert!((m.banks[0].f_est - f as f64).abs() < 0.02, "f̂0 {}", m.banks[0].f_est);
    }
}

#[cfg(test)]
mod probe {
    #![allow(clippy::print_stderr)]
    use super::*;
    use hearo_testkit::signals::{add_at, hum, speech_like};

    #[test]
    #[ignore = "diagnostic"]
    fn probe_tracking_and_depth() {
        let sr = 48_000.0;
        for (name, with_speech) in [("hum only", false), ("speech+hum", true)] {
            let mut x = if with_speech { speech_like(sr, 10.0, 11) } else { vec![0.0; 480_000] };
            let h = hum(sr, 50.0, 5, 10.0, 0.05);
            add_at(&mut x, &h, 0);
            let mut m = DeHum::new();
            m.prepare(sr, 512, 1);
            let mut t = TelemetryWriter::new();
            let mut out = x.clone();
            for (i, c) in out.chunks_mut(512).enumerate() {
                m.process(&mut AudioBlock::from_planar(c, 1), &mut t);
                if i % 94 == 0 {
                    eprintln!(
                        "{name} t={:.0}s f_est {:.4} notch {:.4} engaged {} depth {:?}",
                        i as f32 * 512.0 / sr,
                        m.banks[0].f_est,
                        m.notch_f0,
                        m.engaged,
                        &m.depth[..3]
                    );
                }
            }
            let tail = 240_000;
            let e = |v: &[f32]| v[tail..].iter().map(|x| (*x as f64).powi(2)).sum::<f64>();
            if !with_speech {
                eprintln!("{name}: reduction {:.1} dB", 10.0 * (e(&x) / e(&out)).log10());
            }
        }
    }
}
