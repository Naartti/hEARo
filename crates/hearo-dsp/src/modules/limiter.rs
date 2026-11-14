//! Safety limiter (HANDOVER §7.5): an always-on, stereo-linked **true-peak** lookahead limiter.
//!
//! ```text
//! in ──▶ delay D (≤ 0.5 ms) ───────────────────────────────────────────▶ × g ──▶ clamp ──▶ out
//!   └─▶ per channel: 4× interpolator (12 taps/phase) + near-Nyquist guard
//!       p[m] = max(|x[m]|, |x(m+¼)|, |x(m+½)|, |x(m+¾)|) + |HF(m)|   (max over channels)
//!         ─▶ sliding max over A + 2S (monotonic deque) ─▶ h = min(1, target / max)
//!         ─▶ release (one-pole towards 1, never above h) ─▶ moving average over A ─▶ g
//! ```
//!
//! * **True peak.** Between two samples a band-limited signal can swing higher than either
//!   sample (+3 dB for a sine at fs/4 with 45° phase). A Kaiser-windowed-sinc polyphase FIR
//!   estimates the signal at ¼, ½ and ¾ of each sample interval (4× oversampling, as in
//!   ITU-R BS.1770), and the gain computer limits those peaks, not just the samples.
//! * **Near-Nyquist guard.** With only 0.5 ms of future, no interpolator can know what content
//!   above ~0.42·fs does between samples (it depends on samples we haven't seen yet). A
//!   high-order difference measures how much such content there is and adds it to the peak
//!   estimate: conservative (more limiting) exactly where the estimate is blind, inert below
//!   ~0.3·fs. Cost: a hot tone at 0.42·fs is limited ≈ 5.4 dB more than needed.
//! * **Lookahead without overshoot.** `h[k]` is the smallest required gain anywhere in the
//!   next `A` samples (± `S` guard samples). Averaging `h` over `A` samples gives a gain that
//!   ramps down over `A` samples and is ≤ the required gain throughout `[peak − S, peak + S]`:
//!   every value in the average already "saw" that peak. The sliding max is a monotonic deque
//!   in a fixed ring: O(1) amortized, no allocation.
//! * **Release** is a one-pole recovery towards unity (`release` ms time constant) that can only
//!   *lower* the target, so it never breaks the guarantee.
//! * **Latency** `D = H + S + A − 1` with `H` = 6 (half the interpolator), `S` = 2, and `A` chosen
//!   so `D = ⌊0.5 ms · fs⌋`: 22/24/48 samples at 44.1/48/96 kHz (de-smack uses 40 of the 45 ms).
//! * **Backstop.** A final clamp to ±ceiling guarantees the *sample* peak even in rounding
//!   corner cases or right after a ceiling change; it does not engage on normal material.
//!
//! Below the ceiling `g` is exactly 1.0 and the output is a bit-exact delayed copy.

use crate::delay::MultiDelay;
use crate::module::{ParamDesc, ParamUnit};
use crate::telemetry::{Event, EventKind};
use crate::{AudioBlock, MAX_CHANNELS, Module, ModuleId, TelemetryWriter, db_to_amp};

/// Lookahead budget (project decision: de-smack takes 40 of the 45 ms latency ceiling).
pub const LOOKAHEAD_BUDGET_MS: f32 = 0.5;
/// Taps per polyphase branch of the 4× interpolator.
const TAPS: usize = 12;
/// Interpolator delay in input samples: the estimate between `m` and `m+1` needs `x[m+H]`.
const H: usize = TAPS / 2;
/// Guard samples: the required gain is held this many samples either side of a peak, because
/// the interpolated value between two samples also depends on the gain at their neighbours.
/// (Measured: S = 2 and S = 6 give the same true peaks; the error is in the estimate, not the
/// gain shape, so the longer attack wins.)
const S: usize = 2;
/// Input history per channel: the interpolator window plus one older sample for the
/// even-order HF detector (`x[m − 6 ..= m + 6]`).
const HIST: usize = TAPS + 1;
/// Weight of the near-Nyquist guard term (see [`hf_amplitude`]). Measured: 1.0 keeps every
/// golden signal ≥ 0.09 dB under the ceiling; 0.5 lets white-noise drum snaps through by 0.08 dB.
const HF_WEIGHT: f32 = 1.0;
/// 12th- and 11th-order binomial differences, normalized to unity gain at Nyquist.
const D12: [f32; 13] = binomial_diff::<13>();
const D11: [f32; 12] = binomial_diff::<12>();
/// Smallest attack (moving-average) length; only reached at very low sample rates.
const MIN_ATTACK: usize = 4;
/// Kaiser window β for the interpolator. Measured trade-off (12 taps, worst case over phase vs
/// an ideal 4× grid): β = 2 reads sines exactly up to 0.42·fs (≤ 0.002 dB low) and is 1.5 dB
/// low at 0.45·fs; β = 4 is already 1 dB low at 0.42·fs; β = 1 ripples (0.15 dB low at 0.35·fs).
const KAISER_BETA: f64 = 2.0;
/// Internal headroom under the ceiling for what the estimate still misses. Measured with a
/// 64-tap reference meter (`tests/golden_limiter.rs`): worst output −1.10 dBTP on pink noise,
/// i.e. ~0.1 dB of this is used.
const HEADROOM_DB: f32 = 0.2;
/// Telemetry: an engagement starts when gain reduction exceeds this …
const EVENT_ON_DB: f32 = 1.0;
/// … and ends when it falls below this (hysteresis: one event per engagement).
const EVENT_OFF_DB: f32 = 0.5;
/// A continuous engagement is reported at least this often, so long overloads stay visible.
const EVENT_SPLIT_S: f32 = 1.0;
/// Release target snaps to exactly 1.0 above this (the one-pole would only approach it).
const UNITY_SNAP: f32 = 1.0 - 1e-6;

const PARAMS: [ParamDesc; 2] = [
    ParamDesc {
        key: "ceiling",
        name: "Ceiling",
        unit: ParamUnit::Db,
        min: -6.0,
        max: 0.0,
        default: -1.0,
        step: 0.0,
        choices: &[],
    },
    ParamDesc {
        key: "release",
        name: "Release",
        unit: ParamUnit::Ms,
        min: 20.0,
        max: 1000.0,
        default: 100.0,
        step: 0.0,
        choices: &[],
    },
];

/// Fixed-capacity sliding-window maximum (monotonic deque in a ring). `push` is O(1) amortized.
#[derive(Debug, Default)]
struct SlidingMax {
    pos: Vec<u64>,
    val: Vec<f32>,
    head: usize,
    len: usize,
    window: u64,
}

impl SlidingMax {
    fn prepare(&mut self, window: usize) {
        self.pos = vec![0; window];
        self.val = vec![0.0; window];
        self.window = window as u64;
        self.reset();
    }

    fn reset(&mut self) {
        self.head = 0;
        self.len = 0;
    }

    /// Add sample `v` at position `n` (strictly increasing) and return the max over the last
    /// `window` positions.
    #[inline]
    fn push(&mut self, n: u64, v: f32) -> f32 {
        let cap = self.val.len();
        // Drop dominated values from the back.
        while self.len > 0 {
            let back = (self.head + self.len - 1) % cap;
            if self.val[back] <= v {
                self.len -= 1;
            } else {
                break;
            }
        }
        // Drop the front when it left the window (at most one per push).
        if self.len > 0 && self.pos[self.head] + self.window <= n {
            self.head = (self.head + 1) % cap;
            self.len -= 1;
        }
        // At most `window` positions are inside the window, so this never overflows `cap`.
        let back = (self.head + self.len) % cap;
        self.pos[back] = n;
        self.val[back] = v;
        self.len += 1;
        self.val[self.head]
    }
}

/// Alternating binomial coefficients `(−1)^k C(N−1, k) / 2^(N−1)`: an (N−1)th-order difference,
/// gain `|sin(π f / fs)|^(N−1)` — 1 at Nyquist, 0.016 at fs/4, 8e-5 at 0.15·fs.
const fn binomial_diff<const N: usize>() -> [f32; N] {
    let mut c = [0.0f32; N];
    let mut k = 0;
    let mut v: u64 = 1;
    let scale = (1u64 << (N - 1)) as f32;
    while k < N {
        let sign = if k % 2 == 0 { 1.0 } else { -1.0 };
        c[k] = sign * v as f32 / scale;
        v = v * (N - 1 - k) as u64 / (k as u64 + 1);
        k += 1;
    }
    c
}

/// Modified Bessel function of the first kind, order 0 (for the Kaiser window).
fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let q = x * x / 4.0;
    for k in 1..50 {
        term *= q / (k * k) as f64;
        sum += term;
        if term < 1e-12 * sum {
            break;
        }
    }
    sum
}

/// Polyphase kernels for the ¼, ½, ¾ positions: tap `t` multiplies `x[m − 5 + t]` and the
/// result estimates `x(m + phase/4)`. Kaiser-windowed sinc, each phase normalized to unity DC
/// gain.
fn interpolator() -> [[f32; TAPS]; 3] {
    let mut k = [[0.0f32; TAPS]; 3];
    let half = H as f64;
    for (p, row) in k.iter_mut().enumerate() {
        let frac = (p + 1) as f64 / 4.0;
        let mut taps = [0.0f64; TAPS];
        for (t, tap) in taps.iter_mut().enumerate() {
            let u = t as f64 - (H as f64 - 1.0) - frac; // distance from the estimated position
            let sinc = if u == 0.0 { 1.0 } else { (core::f64::consts::PI * u).sin() / (core::f64::consts::PI * u) };
            let r = (u / half).clamp(-1.0, 1.0);
            *tap = sinc * bessel_i0(KAISER_BETA * (1.0 - r * r).sqrt()) / bessel_i0(KAISER_BETA);
        }
        let dc: f64 = taps.iter().sum();
        for (dst, t) in row.iter_mut().zip(taps) {
            *dst = (t / dc) as f32;
        }
    }
    k
}

#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).fold(0.0, |acc, (x, y)| acc + x * y)
}

/// How much near-Nyquist content sits around `x[m]` … `x[m+1]` (`full` = `x[m−6..=m+6]`).
///
/// Near Nyquist the 12-tap interpolator can't know the signal between samples: the samples
/// beyond the 0.5 ms lookahead decide it (white-noise peaks were read up to 1.5 dB low).
/// Root-sum-square of a 12th-order difference centred on `m` and an 11th-order one centred on
/// `m + ½`. It is a heuristic level, not an exact amplitude: it reads ≤ 0.025 at fs/4, ≈ 1 at
/// 0.42·fs and up to ~1.4 just below Nyquist (both terms then see the same component). Adding
/// it to the estimate makes the peak reading conservative exactly where the interpolator is
/// blind, and costs nothing below ~0.3·fs.
#[inline]
fn hf_amplitude(full: &[f32]) -> f32 {
    let even = dot(&D12, full);
    let odd = dot(&D11, &full[1..]);
    (even * even + odd * odd).sqrt()
}

/// The safety limiter module.
#[derive(Debug)]
pub struct Limiter {
    ceiling_db: f32,
    release_ms: f32,
    /// Hard ceiling (linear) for the backstop clamp.
    ceiling: f32,
    /// Internal target = ceiling − headroom (linear).
    target: f32,
    sr: f32,
    kernels: [[f32; TAPS]; 3],
    /// Per-channel input history, written twice (`i` and `i + TAPS`) so the last `TAPS` samples
    /// are always one contiguous slice.
    hist: [[f32; 2 * HIST]; MAX_CHANNELS],
    hist_pos: usize,
    peaks: SlidingMax,
    /// Moving average of the release-smoothed target (ring + running sum).
    avg_ring: Vec<f32>,
    avg_pos: usize,
    avg_sum: f64,
    attack: usize,
    latency: usize,
    release_coeff: f32,
    /// Release-smoothed target gain (before the moving average).
    held: f32,
    delay: MultiDelay,
    /// Input samples seen since `reset`.
    pos: u64,
    /// Current engagement: (start, chain-input clock; lowest gain); `None` when idle.
    engaged: Option<(i64, f32)>,
    /// Event hysteresis thresholds as linear gains.
    event_on: f32,
    event_off: f32,
    split: u64,
}

impl Default for Limiter {
    fn default() -> Self {
        Self::new()
    }
}

impl Limiter {
    /// Limiter with default parameters (ceiling −1 dBTP, release 100 ms).
    pub fn new() -> Self {
        let mut l = Self {
            ceiling_db: PARAMS[0].default,
            release_ms: PARAMS[1].default,
            ceiling: 1.0,
            target: 1.0,
            sr: 48_000.0,
            kernels: interpolator(),
            hist: [[0.0; 2 * HIST]; MAX_CHANNELS],
            hist_pos: 0,
            peaks: SlidingMax::default(),
            avg_ring: Vec::new(),
            avg_pos: 0,
            avg_sum: 0.0,
            attack: MIN_ATTACK,
            latency: 0,
            release_coeff: 0.0,
            held: 1.0,
            delay: MultiDelay::default(),
            pos: 0,
            engaged: None,
            event_on: db_to_amp(-EVENT_ON_DB),
            event_off: db_to_amp(-EVENT_OFF_DB),
            split: 48_000,
        };
        l.update_ceiling();
        l
    }

    /// Attack (moving-average) length and total latency for a sample rate.
    pub fn timing(sample_rate: f32) -> (usize, usize) {
        let budget = (sample_rate * LOOKAHEAD_BUDGET_MS / 1000.0 + 1e-3).floor() as usize;
        let attack = (budget + 1).saturating_sub(H + S).max(MIN_ATTACK);
        (attack, H + S + attack - 1)
    }

    fn update_ceiling(&mut self) {
        self.ceiling = db_to_amp(self.ceiling_db);
        self.target = db_to_amp(self.ceiling_db - HEADROOM_DB);
    }

    fn update_release(&mut self) {
        self.release_coeff = 1.0 - (-1000.0 / (self.release_ms * self.sr)).exp();
    }

    /// Feed one input frame (sample `i` of every channel) and return the gain for the output
    /// sample leaving the delay line now.
    #[inline]
    fn gain_step(&mut self, block: &AudioBlock<'_>, i: usize) -> f32 {
        let w = self.hist_pos;
        let mut peak = 0.0f32;
        for c in 0..block.channels() {
            let raw = block.channel(c)[i];
            // A non-finite sample would make `max` ignore real peaks around it: store 0 and treat
            // the frame as an infinite peak (full reduction) instead.
            let x = if raw.is_finite() { raw } else { 0.0 };
            if !raw.is_finite() {
                peak = f32::INFINITY;
            }
            let h = &mut self.hist[c];
            h[w] = x;
            h[w + HIST] = x;
            // Oldest-first window of the last HIST samples: x[m − 6 ..= m + 6], m = newest − 6.
            let full = &h[w + 1..w + 1 + HIST];
            let win = &full[1..]; // x[m − 5 ..= m + 6]: the interpolator's taps
            let mut p = win[H - 1].abs();
            for k in &self.kernels {
                p = p.max(dot(k, win).abs());
            }
            peak = peak.max(p + HF_WEIGHT * hf_amplitude(full));
        }
        self.hist_pos = if w + 1 == HIST { 0 } else { w + 1 };

        let n = self.pos;
        self.pos += 1;
        let max = self.peaks.push(n, peak);
        // Required gain for the whole window (NaN-safe: `max` is finite or inf, never NaN for
        // finite input; inf gives 0).
        let need = if max > self.target { self.target / max } else { 1.0 };
        let rel = self.held + (1.0 - self.held) * self.release_coeff;
        let rel = if rel > UNITY_SNAP { 1.0 } else { rel };
        self.held = need.min(rel);

        // Moving average over `attack` samples (running sum, re-summed exactly once per lap so
        // rounding can't drift).
        let a = self.attack;
        self.avg_sum += self.held as f64 - self.avg_ring[self.avg_pos] as f64;
        self.avg_ring[self.avg_pos] = self.held;
        self.avg_pos += 1;
        if self.avg_pos == a {
            self.avg_pos = 0;
            self.avg_sum = self.avg_ring.iter().map(|&v| v as f64).sum();
        }
        if self.avg_sum >= a as f64 { 1.0 } else { (self.avg_sum / a as f64) as f32 }
    }
}

impl Module for Limiter {
    fn id(&self) -> ModuleId {
        ModuleId::Limiter
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize, channels: usize) {
        self.sr = sample_rate;
        let (attack, latency) = Self::timing(sample_rate);
        self.attack = attack;
        self.latency = latency;
        self.peaks.prepare(attack + 2 * S);
        self.avg_ring = vec![1.0; attack];
        self.delay.prepare(channels, latency);
        self.split = (sample_rate * EVENT_SPLIT_S).round().max(1.0) as u64;
        self.update_release();
        self.reset();
    }

    fn reset(&mut self) {
        self.hist = [[0.0; 2 * HIST]; MAX_CHANNELS];
        self.hist_pos = 0;
        self.peaks.reset();
        self.avg_ring.fill(1.0);
        self.avg_pos = 0;
        self.avg_sum = self.avg_ring.len() as f64;
        self.held = 1.0;
        self.delay.reset();
        self.pos = 0;
        self.engaged = None;
    }

    fn latency_samples(&self) -> usize {
        self.latency
    }

    fn process(&mut self, block: &mut AudioBlock<'_>, telemetry: &mut TelemetryWriter) {
        let n = block.frames();
        let ch = block.channels();
        if self.avg_ring.is_empty() {
            return; // not prepared: pass through
        }
        // Telemetry clock: the output written at block index `i` entered this module `latency`
        // samples earlier, i.e. at chain-input position `block_start + i − latency` (same
        // convention as de-smack: upstream latency is not subtracted).
        let chain_start = telemetry.block_start() as i64;
        let lat = self.latency as i64;
        let ceil = self.ceiling;
        let mut min_g = 1.0f32;

        // Gains are computed from the undelayed input before the block is delayed in place; a
        // small stack buffer keeps it allocation-free for any block size.
        const CHUNK: usize = 256;
        let mut gains = [1.0f32; CHUNK];
        let mut start = 0;
        while start < n {
            let len = CHUNK.min(n - start);
            for (j, g) in gains[..len].iter_mut().enumerate() {
                *g = self.gain_step(block, start + j);
            }
            let mut sub = block.slice_mut(start, len);
            self.delay.process(&mut sub);
            for (j, &g) in gains[..len].iter().enumerate() {
                if g < 1.0 {
                    min_g = min_g.min(g);
                    for c in 0..ch {
                        let y = &mut sub.channel_mut(c)[j];
                        // inf × 0 = NaN, and clamp passes NaN: never let that out of the last module.
                        let v = *y * g;
                        *y = if v.is_finite() { v.clamp(-ceil, ceil) } else { 0.0 };
                    }
                } else {
                    for c in 0..ch {
                        let y = &mut sub.channel_mut(c)[j];
                        if !y.is_finite() {
                            *y = 0.0;
                        } else if y.abs() > ceil {
                            *y = y.clamp(-ceil, ceil);
                        }
                    }
                }
                // Telemetry: one event per engagement (hysteresis), split every EVENT_SPLIT_S.
                let at = chain_start + (start + j) as i64 - lat;
                match self.engaged {
                    None if g < self.event_on => self.engaged = Some((at, g)),
                    Some((s, m)) => {
                        let m = m.min(g);
                        let ended = g > self.event_off;
                        if ended || at - s >= self.split as i64 {
                            telemetry.push_event(Event {
                                module: ModuleId::Limiter,
                                kind: EventKind::Limit,
                                start_sample: s.max(0) as u64,
                                duration_samples: (at - s) as u32,
                                energy_removed_db: -crate::amp_to_db(m),
                            });
                            self.engaged = if ended { None } else { Some((at, g)) };
                        } else {
                            self.engaged = Some((s, m));
                        }
                    }
                    None => {}
                }
            }
            start += len;
        }
        telemetry.set_gain_reduction(ModuleId::Limiter, -crate::amp_to_db(min_g));
    }

    fn params(&self) -> &[ParamDesc] {
        &PARAMS
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let Some(p) = PARAMS.get(index) else { return };
        let v = p.clamp(value);
        match index {
            0 => {
                self.ceiling_db = v;
                self.update_ceiling();
            }
            _ => {
                self.release_ms = v;
                self.update_release();
            }
        }
    }

    fn param(&self, index: usize) -> f32 {
        match index {
            0 => self.ceiling_db,
            1 => self.release_ms,
            _ => f32::NAN,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sliding_max_matches_brute_force() {
        let mut m = SlidingMax::default();
        m.prepare(7);
        let mut rng = 12345u32;
        let mut xs = Vec::new();
        for n in 0..500u64 {
            rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let v = (rng >> 8) as f32 / (1u32 << 24) as f32;
            xs.push(v);
            let got = m.push(n, v);
            let lo = xs.len().saturating_sub(7);
            let want = xs[lo..].iter().cloned().fold(f32::MIN, f32::max);
            assert_eq!(got, want, "n {n}");
        }
    }

    #[test]
    fn hf_detector_gains() {
        let gain = |f: f32| {
            // Amplitude reading of a unit sine at f·fs, max over phases.
            (0..16)
                .map(|p| {
                    let ph = p as f32 * core::f32::consts::PI / 8.0;
                    let x: Vec<f32> =
                        (0..HIST).map(|n| (2.0 * core::f32::consts::PI * f * n as f32 + ph).sin()).collect();
                    hf_amplitude(&x)
                })
                .fold(0.0f32, f32::max)
        };
        assert!((0.9..1.5).contains(&gain(0.48)), "0.48 fs: {}", gain(0.48));
        assert!((0.8..1.1).contains(&gain(0.42)), "0.42 fs: {}", gain(0.42));
        assert!(gain(0.25) < 0.03, "fs/4: {}", gain(0.25));
        assert!(gain(0.1) < 1e-4, "0.1 fs: {}", gain(0.1));
    }

    #[test]
    fn interpolator_is_dc_exact_and_halfway_symmetric() {
        let k = interpolator();
        for row in &k {
            assert!((row.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        }
        for t in 0..TAPS {
            assert!((k[1][t] - k[1][TAPS - 1 - t]).abs() < 1e-6, "½ phase symmetric");
            assert!((k[0][t] - k[2][TAPS - 1 - t]).abs() < 1e-6, "¼ and ¾ mirror");
        }
    }

    #[test]
    fn timing_fits_the_budget() {
        for (sr, attack, lat) in [(44_100.0, 15, 22), (48_000.0, 17, 24), (96_000.0, 41, 48)] {
            assert_eq!(Limiter::timing(sr), (attack, lat));
        }
        // Very low rates keep a minimum attack (latency then exceeds 0.5 ms; not a target rate).
        assert_eq!(Limiter::timing(8_000.0).0, MIN_ATTACK);
    }

    #[test]
    fn params_clamp() {
        let mut l = Limiter::new();
        l.set_param(0, -20.0);
        assert_eq!(l.param(0), -6.0);
        l.set_param(1, f32::NAN);
        assert_eq!(l.param(1), 100.0);
        assert!(l.param(5).is_nan());
    }
}
