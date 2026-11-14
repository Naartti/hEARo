//! De-click (§7.3): finds short impulsive clicks *inside* speech or music with a linear-prediction
//! residual and repairs them by least-squares autoregressive (LSAR) interpolation.
//!
//! ```text
//! in ─┬─▶ ring buffer (2.5 ms lookahead) ──────────────── repair written in place ──▶ out
//!     └─▶ mono ─▶ HP 2 kHz ─▶ LPC residual, forward + backward (order 16)
//!                               ├─▶ trigger / extend / end ─▶ region ─▶ short? broadband? ─┐
//!                               └─▶ HP 6 kHz (broadband share) ───────────────────────────┤
//!          ring buffer ─▶ Burg AR(32) fit on the 10 ms before ─▶ anchored LSAR solve ◀─────┘
//! ```
//!
//! *Detection.* The detection signal is the input high-passed at 2 kHz. A short-term LPC model
//! (order 16 at 48 kHz, Hann window of 10 ms, refit every 4 ms) predicts each sample from the
//! previous ones; the residual (prediction error) is small for anything the recent past explains
//! — tones, formants, steady noise — and jumps at an impulse. A click *triggers* when the residual
//! is both ≫ its running median (a log-domain quantile tracker: the MAD-like robust level) and ≫
//! its recent peak (so a train of glottal pulses, each about as big as the last, doesn't). The
//! forward residual smears a click over the `order` samples after it, so the region's end comes
//! from the *backward* residual (each sample predicted from the ones after it), which the
//! lookahead makes available. A region closes after 0.4 ms without flags and is accepted only if
//! it is **short** (closed before the lookahead runs out, i.e. ≤ ~2 ms) and **broadband** (its
//! residual is white: a fair share above 6 kHz). Drum hits, plucks and plosives are long; glottal
//! pulses and formant ringing leave their residual in 2–6 kHz.
//!
//! *Repair.* At the last possible moment (just before the region's first sample is output) an
//! AR(32) model is fitted per channel to the 10 ms before the region (Burg, regularised), and the
//! region is replaced by the samples that minimise the AR prediction error over the region and
//! the (≤ 0.7 ms of) context after it — Janssen/Veldhuis/Godsill LSAR — plus a penalty for
//! changing the region's content below ~1.5 kHz (see [`lsar`]). Solved by banded Givens QR in
//! O(m·p²), only for accepted regions, at most 20 per second on average (CPU guard).
//!
//! Latency is 2.5 ms (project budget: de-smack already takes 40 of the 45 ms ceiling). That is
//! what limits the repairable click length to ~2 ms — the region and a little context after it
//! must be known before its first sample leaves; longer clicks are left untouched — and the
//! short post-context is what limits the repair quality (deep dive 06, "De-click").
//!
//! Telemetry: one [`EventKind::Click`] per repair (start in the chain-input clock, flagged
//! length, region energy before/after in dB); the gain-reduction meter shows the largest of
//! those in the block (0 when nothing was repaired).

use crate::filters::Biquad;
use crate::module::{ParamDesc, ParamUnit};
use crate::telemetry::{Event, EventKind};
use crate::{AudioBlock, MAX_CHANNELS, Module, ModuleId, TelemetryWriter};

/// Lookahead (= latency). Region + post context must fit in it.
const LOOKAHEAD_MS: f32 = 2.5;
/// Detection band (4th-order high-pass).
const DET_HP_HZ: f64 = 2_000.0;
/// "Broadband" check band (4th-order high-pass).
const HI_HP_HZ: f64 = 6_000.0;
/// Detection LPC order, analysis window and refit hop.
const DET_ORDER_48K: usize = 16;
const MAX_DET_ORDER: usize = 32;
const DET_WINDOW_MS: f32 = 10.0;
const DET_HOP_MS: f32 = 4.0;
/// White-noise correction for the detection LPC (caps the prediction gain at ~40 dB).
const DET_WNC: f64 = 1e-4;
/// Interpolation AR order at 48 kHz (scaled with the sample rate, clamped to 24..=48) and the
/// fit window before the region.
const AR_ORDER_48K: usize = 32;
const MAX_AR_ORDER: usize = 48;
/// At most this many repairs in one process call (bounded worst-case callback time).
const MAX_REPAIRS_PER_CALL: u32 = 1;
const AR_WINDOW_MS: f32 = 10.0;
/// Burg regularisation: the fit behaves as if white noise this far below the context (power
/// ratio) were present — a safety net against f32 rounding on perfectly clean tones.
const AR_NOISE_FLOOR: f64 = 3e-4;
/// Low-band anchor: the interpolation is also asked to keep the region's content below ~this
/// close to what was observed (weight `LF_ANCHOR`). A click lives almost entirely above it, while
/// AR interpolation of the low band (speech and music are 10–20× oversampled there at 48 kHz) is
/// badly conditioned and would otherwise add audible low "bumps". See `lsar`.
const LF_ANCHOR_HZ: f64 = 1500.0;
const LF_ANCHOR: f64 = 1.0;
/// Anchor rows are taken every this long (the anchored band is ≤ 1.5 kHz: 4× oversampled by a
/// 12 kHz row rate), which cuts the solver's work by about a third.
const ANCHOR_STRIDE_MS: f32 = 1.0 / 12.0;
/// Padding added around the flagged samples before repair.
const PRE_PAD_MS: f32 = 0.1;
const POST_PAD_MS: f32 = 0.15;
/// A region closes after this long without a flagged sample.
const MERGE_MS: f32 = 0.4;
/// After rejecting a long transient, don't retrigger for this long (its tail).
const LOCKOUT_MS: f32 = 10.0;
/// Median tracker slew (log domain, up and down equal = 50 % quantile).
const FLOOR_SLEW_DB_PER_S: f32 = 800.0;
/// Recent-peak tracker decay time constant (spans several glottal periods).
const PEAK_TAU_S: f32 = 0.030;
/// The recent peak only learns from residuals at least this old, so a click's own first samples
/// (its onset can take a few samples to peak, especially at 96 kHz) don't raise the bar for it.
const PEAK_DELAY_MS: f32 = 0.15;
const AE_HIST: usize = 64;
/// Trigger: residual above K_FLOOR × median and K_PEAK × recent peak (at sensitivity 0.5).
const K_FLOOR: f32 = 10.0;
const K_PEAK: f32 = 2.5;
/// Extension: residual above K_EXT × median (frozen at trigger) or EXT_FRAC × region peak.
const K_EXT: f32 = 4.0;
const EXT_FRAC: f32 = 0.03;
/// Residuals below this (≈ −74 dBFS) never trigger or extend.
const ABS_MIN: f32 = 2e-4;
/// Broadband: the share of the region's LPC-residual energy above 6 kHz must be at least this
/// fraction of what white noise would put there (≈ 0.82 at 48 kHz). A click's residual is white;
/// glottal pulses and formant ringing leave most of theirs in 2–6 kHz (measured ≤ 0.6 vs ≥ 0.85).
const BROADBAND_OF_WHITE: f32 = 0.85;
/// Sensitivity 0 → thresholds +12 dB, 1 → −12 dB.
const SENS_RANGE_DB: f32 = 24.0;
const MAX_PENDING: usize = 8;
/// CPU guard: at most this many repairs per second on average (token bucket, burst
/// `REPAIR_BURST`). Real mouth clicks come a few per second; a signal that "clicks" more often
/// than this is noise-like, and repairing all of it would cost ~0.4 ms per click.
const MAX_REPAIRS_PER_S: f32 = 20.0;
const REPAIR_BURST: f32 = 8.0;

const PARAMS: [ParamDesc; 1] = [ParamDesc {
    key: "sensitivity",
    name: "Sensitivity",
    unit: ParamUnit::Percent,
    min: 0.0,
    max: 1.0,
    default: 0.5,
    step: 0.0,
    choices: &[],
}];

#[derive(Debug, Clone, Copy)]
struct Open {
    start: u64,
    /// Last sample flagged by the forward residual (lags the click end by up to the order).
    last: u64,
    /// Last sample flagged by the backward residual (exact click end; known order samples late).
    last_b: u64,
    peak: f32,
    floor0: f32,
    sum_e: f32,
    sum_re: f32,
}

#[derive(Debug, Clone, Copy, Default)]
struct Region {
    /// First repaired sample (module-input clock).
    g0: u64,
    /// One past the last repaired sample.
    g1: u64,
    /// Flagged extent (for telemetry).
    start: u64,
    end: u64,
}

/// The de-click module.
#[derive(Debug)]
pub struct DeClick {
    sensitivity: f32,
    k_floor: f32,
    k_peak: f32,
    sr: f32,
    channels: usize,
    lookahead: usize,
    pre: u64,
    post: u64,
    merge: u64,
    lockout: u64,
    det_win: usize,
    det_hop: usize,
    hop_ctr: usize,
    ar_win: usize,
    ar_order: usize,
    m_max: usize,
    /// Zero-phase low-pass taps (p + 1 at most) for the low-band anchor.
    lf_h: Vec<f64>,
    /// Only every `anchor_stride`-th anchor row is used (the low band is heavily oversampled).
    anchor_stride: usize,
    /// Required residual energy share above `HI_HP_HZ` (see `BROADBAND_OF_WHITE`).
    broadband: f32,
    warmup: u64,
    hp_det: [Biquad; 2],
    hp_res: [Biquad; 2],
    det_order: usize,
    det_a: [f32; MAX_DET_ORDER + 1],
    dring: Vec<f32>,
    dmask: usize,
    /// Per-channel input history, `rsize` samples each; the output reads it `lookahead` late.
    ring: Vec<f32>,
    rsize: usize,
    rmask: usize,
    det_hann: Vec<f64>,
    work: Vec<f64>,
    work2: Vec<f64>,
    /// Repair scratch: original region, LSAR segment, solver buffers.
    orig: Vec<f64>,
    seg: Vec<f64>,
    cvec: Vec<f64>,
    bvec: Vec<f64>,
    band: Vec<f64>,
    ar: [f64; MAX_AR_ORDER + 1],
    floor: f32,
    floor_up: f32,
    floor_down: f32,
    pk: f32,
    pk_decay: f32,
    pk_delay: usize,
    ae_hist: [f32; AE_HIST],
    active: Option<Open>,
    pending: [Region; MAX_PENDING],
    p_head: usize,
    p_len: usize,
    pos: u64,
    lock_until: u64,
    tokens: f32,
    token_rate: f32,
}

impl Default for DeClick {
    fn default() -> Self {
        Self::new()
    }
}

fn sens_scale(s: f32) -> f32 {
    crate::db_to_amp((0.5 - s) * SENS_RANGE_DB)
}

fn hann(n: usize) -> Vec<f64> {
    (0..n).map(|i| 0.5 - 0.5 * (2.0 * core::f64::consts::PI * (i as f64 + 0.5) / n as f64).cos()).collect()
}

/// Zero-phase Hann-windowed-sinc low-pass with `2k+1` taps and unity DC gain.
fn windowed_sinc_lowpass(k: usize, fc: f64, sr: f64) -> Vec<f64> {
    use core::f64::consts::PI;
    let wc = 2.0 * PI * fc / sr;
    let mut h: Vec<f64> = (0..=2 * k)
        .map(|i| {
            let n = i as f64 - k as f64;
            let sinc = if i == k { wc / PI } else { (wc * n).sin() / (PI * n) };
            sinc * (0.5 + 0.5 * (PI * n / (k as f64 + 1.0)).cos())
        })
        .collect();
    let sum: f64 = h.iter().sum();
    h.iter_mut().for_each(|v| *v /= sum);
    h
}

/// `r[lag] = Σ x[i]·x[i+lag]` for `lag` in `0..r.len()`.
fn autocorr(x: &[f64], r: &mut [f64]) {
    for (lag, rl) in r.iter_mut().enumerate() {
        let mut s = 0.0;
        for i in lag..x.len() {
            s += x[i] * x[i - lag];
        }
        *rl = s;
    }
}

/// Levinson–Durbin: prediction-error filter `a` (a[0] = 1, length p+1) from autocorrelation `r`.
/// `e[n] = Σ a[k]·x[n−k]`. Stops early (keeping a stable filter) if the error vanishes.
fn levinson(r: &[f64], a: &mut [f64]) {
    let p = a.len() - 1;
    a.fill(0.0);
    a[0] = 1.0;
    let mut err = r[0];
    if err.is_nan() || err <= 0.0 {
        return;
    }
    for i in 1..=p {
        let mut acc = r[i];
        for j in 1..i {
            acc += a[j] * r[i - j];
        }
        let k = -acc / err;
        if k.is_nan() || k.abs() >= 1.0 {
            return;
        }
        for j in 1..=i / 2 {
            let (x, y) = (a[j], a[i - j]);
            a[j] = x + k * y;
            a[i - j] = y + k * x;
        }
        a[i] = k;
        err *= 1.0 - k * k;
        if err <= 0.0 {
            return;
        }
    }
}

/// Burg's method: prediction-error filter `a` (a[0] = 1) fitted to the data in `f` without
/// windowing (much sharper spectral peaks than windowed autocorrelation — what interpolating
/// tones needs). `f` is overwritten; `b` is scratch of the same length. The filter is always
/// stable (every reflection coefficient has |k| ≤ 1).
fn burg(f: &mut [f64], b: &mut [f64], a: &mut [f64], noise_floor: f64) {
    let p = a.len() - 1;
    let n = f.len();
    a.fill(0.0);
    a[0] = 1.0;
    b[..n].copy_from_slice(f);
    let floor = 2.0 * noise_floor * f.iter().map(|v| v * v).sum::<f64>() / n.max(1) as f64;
    for m in 1..=p.min(n.saturating_sub(1)) {
        let (mut num, mut den) = (0.0, 0.0);
        for i in m..n {
            num += f[i] * b[i - 1];
            den += f[i] * f[i] + b[i - 1] * b[i - 1];
        }
        den += floor * (n - m) as f64;
        if den <= 1e-30 {
            return;
        }
        let k = -2.0 * num / den;
        for j in 1..=m / 2 {
            let (u, v) = (a[j], a[m - j]);
            a[j] = u + k * v;
            a[m - j] = v + k * u;
        }
        a[m] = k;
        for i in (m..n).rev() {
            let fi = f[i];
            f[i] = fi + k * b[i - 1];
            b[i] = b[i - 1] + k * fi;
        }
    }
}

/// Rotate one row (support `row[..w]` starting at column `j_lo`, right-hand side `rhs`) into the
/// banded upper-triangular factor `rb` (R[j][j+d] at `rb[j·w + d]`) with Givens rotations.
fn qr_absorb(rb: &mut [f64], z: &mut [f64], w: usize, m: usize, row: &mut [f64], mut rhs: f64, j_lo: usize) {
    for j in j_lo..m {
        let v = row[0];
        if v != 0.0 {
            let rj = &mut rb[j * w..(j + 1) * w];
            if rj[0] == 0.0 {
                // Empty pivot row: this row becomes it.
                rj.copy_from_slice(&row[..w]);
                z[j] = rhs;
                return;
            }
            let h = rj[0].hypot(v);
            let (cs, sn) = (rj[0] / h, v / h);
            for (x0, x1) in rj.iter_mut().zip(row.iter_mut()) {
                let (r0, r1) = (*x0, *x1);
                *x0 = cs * r0 + sn * r1;
                *x1 = cs * r1 - sn * r0;
            }
            let zj = z[j];
            z[j] = cs * zj + sn * rhs;
            rhs = cs * rhs - sn * zj;
        }
        row.copy_within(1..w, 0);
        row[w - 1] = 0.0;
    }
}

/// Least-squares AR interpolation (Janssen/Veldhuis/Godsill LSAR) with a low-band anchor.
///
/// `y` = `[p pre-context | m unknowns | q post-context]`, the unknown slots holding the
/// observed (clicked) samples on entry. They are overwritten with the `u` minimising
///
/// `Σ_t e[t]²  +  λ·Σ_n (L·(u − y_obs))[n]²`
///
/// where `e = a ∗ y` are the AR prediction errors that touch the unknowns (from the first unknown
/// to the end of the post-context) and `L` is the zero-phase low-pass `lf` (2K+1 taps, 2K ≤ p).
/// The second term keeps the region's low band close to what was observed: a click lives almost
/// entirely above ~2 kHz, while an AR model of speech or music (10–20× oversampled below 2 kHz
/// at 48 kHz) leaves low-frequency "bumps" nearly unpenalised — the classic LSAR failure on long
/// gaps. Solved as one least-squares problem with banded Givens QR (the normal equations of a
/// sharp model are far too ill-conditioned). Scratch: `c` ≥ m+q, `z` ≥ m, `rb` ≥ m·(p+1).
/// Cost O((m+q+2K)·p²).
#[allow(clippy::too_many_arguments)]
fn lsar(
    a: &[f64],
    lf: &[f64],
    lambda: f64,
    stride: usize,
    y: &mut [f64],
    m: usize,
    q: usize,
    c: &mut [f64],
    z: &mut [f64],
    rb: &mut [f64],
) {
    let p = a.len() - 1;
    let w = p + 1;
    let nt = m + q;
    let kk = lf.len() / 2;
    if m == 0 || p > MAX_AR_ORDER || 2 * kk > p {
        return;
    }
    // Known part of each error: c[t] = Σ_k a[k]·y[p+t−k] over known samples.
    for (t, ct) in c.iter_mut().enumerate().take(nt) {
        let mut s = 0.0;
        for (k, ak) in a.iter().enumerate() {
            let idx = p + t - k;
            if idx < p || idx >= p + m {
                s += ak * y[idx];
            }
        }
        *ct = s;
    }
    rb[..m * w].fill(0.0);
    z[..m].fill(0.0);
    let mut row = [0.0f64; MAX_AR_ORDER + 1];
    // AR rows: row t has a[t−j] in column j. Every column j gets its pivot from row t = j.
    for t in 0..nt {
        let j_lo = t.saturating_sub(p);
        if j_lo >= m {
            break; // later rows don't involve unknowns
        }
        for (d, r) in row.iter_mut().enumerate().take(w) {
            let j = j_lo + d;
            *r = if j <= t && j < m { a[t - j] } else { 0.0 };
        }
        qr_absorb(rb, z, w, m, &mut row, -c[t], j_lo);
    }
    // Low-band anchor rows: √λ·(L u)[n] = √λ·(L y_obs)[n] for n in −K..m+K (u = y_obs = 0
    // outside the region does not enter: the anchor only concerns the region's own change).
    if lambda > 0.0 && !lf.is_empty() {
        let stride = stride.max(1);
        let sl = (lambda * stride as f64).sqrt();
        for n in (0..m + 2 * kk).step_by(stride) {
            // Region column j contributes lf[n − j] (n is offset by K).
            let j_lo = n.saturating_sub(2 * kk);
            let j_hi = n.min(m - 1);
            if j_lo > j_hi {
                continue;
            }
            let mut rhs = 0.0;
            for (d, r) in row.iter_mut().enumerate().take(w) {
                let j = j_lo + d;
                *r = if j <= j_hi { sl * lf[n - j] } else { 0.0 };
                if j <= j_hi {
                    rhs += *r * y[p + j];
                }
            }
            qr_absorb(rb, z, w, m, &mut row, rhs, j_lo);
        }
    }
    // Back-substitute R u = z.
    for j in (0..m).rev() {
        let mut s = z[j];
        for d in 1..w.min(m - j) {
            s -= rb[j * w + d] * y[p + j + d];
        }
        let r = rb[j * w];
        y[p + j] = if r != 0.0 { s / r } else { 0.0 };
    }
}

impl DeClick {
    /// De-click with default parameters.
    pub fn new() -> Self {
        Self {
            sensitivity: PARAMS[0].default,
            k_floor: K_FLOOR,
            k_peak: K_PEAK,
            sr: 48_000.0,
            channels: 0,
            lookahead: 0,
            pre: 0,
            post: 0,
            merge: 1,
            lockout: 0,
            det_win: 0,
            det_hop: 1,
            hop_ctr: 0,
            ar_win: 0,
            m_max: 0,
            warmup: 0,
            hp_det: [Biquad::default(); 2],
            hp_res: [Biquad::default(); 2],
            det_order: DET_ORDER_48K,
            det_a: [0.0; MAX_DET_ORDER + 1],
            dring: Vec::new(),
            dmask: 0,
            ring: Vec::new(),
            rsize: 0,
            rmask: 0,
            det_hann: Vec::new(),
            work: Vec::new(),
            work2: Vec::new(),
            seg: Vec::new(),
            ar_order: AR_ORDER_48K,
            lf_h: Vec::new(),
            anchor_stride: 1,
            broadband: 0.0,
            orig: Vec::new(),
            cvec: Vec::new(),
            bvec: Vec::new(),
            band: Vec::new(),
            ar: [0.0; MAX_AR_ORDER + 1],
            floor: 0.0,
            floor_up: 1.0,
            floor_down: 1.0,
            pk: 0.0,
            pk_decay: 0.0,
            pk_delay: 1,
            ae_hist: [0.0; AE_HIST],
            active: None,
            pending: [Region::default(); MAX_PENDING],
            p_head: 0,
            p_len: 0,
            pos: 0,
            lock_until: 0,
            tokens: REPAIR_BURST,
            token_rate: 0.0,
        }
    }

    fn samples(&self, ms: f32) -> usize {
        (self.sr * ms / 1000.0).round() as usize
    }

    fn set_sensitivity(&mut self, s: f32) {
        self.sensitivity = s;
        let k = sens_scale(s);
        self.k_floor = K_FLOOR * k;
        self.k_peak = K_PEAK * k;
    }

    /// Refit the detection LPC on the window of the detection signal ending at `pos`.
    fn refit_detector(&mut self, pos: u64) {
        let start = (pos + 1 - self.det_win as u64) as usize;
        for k in 0..self.det_win {
            self.work[k] = self.dring[(start + k) & self.dmask] as f64 * self.det_hann[k];
        }
        let o = self.det_order;
        let mut r = [0.0f64; MAX_DET_ORDER + 1];
        autocorr(&self.work[..self.det_win], &mut r[..=o]);
        r[0] = r[0] * (1.0 + DET_WNC) + 1e-24;
        let mut a = [0.0f64; MAX_DET_ORDER + 1];
        levinson(&r[..=o], &mut a[..=o]);
        for (d, s) in self.det_a.iter_mut().zip(a) {
            *d = s as f32;
        }
    }

    /// Feed one mono sample at `self.pos` to the detector.
    #[inline]
    fn detect(&mut self, x: f32) {
        let pos = self.pos;
        let xd = x.clamp(-1e4, 1e4) as f64;
        let mut d = xd;
        for f in &mut self.hp_det {
            d = f.tick(d);
        }
        let d = d as f32;
        let di = pos as usize & self.dmask;
        let mut e = d;
        let o = self.det_order;
        for k in 1..=o {
            e += self.det_a[k] * self.dring[di.wrapping_sub(k) & self.dmask];
        }
        self.dring[di] = d;
        let ae = e.abs();
        // Backward residual for sample `pos − order` (predicted from the samples after it). A click
        // smears the forward residual over the order samples *after* it, the backward residual
        // over the order samples *before* it: the forward residual finds the start, the backward
        // one the end, so short clicks get short regions.
        let nb = pos.wrapping_sub(o as u64);
        let mut eb = 0.0f32;
        for k in 0..=o {
            eb += self.det_a[k] * self.dring[(nb as usize).wrapping_add(k) & self.dmask];
        }
        let mut re = e as f64;
        for f in &mut self.hp_res {
            re = f.tick(re);
        }
        let (e2, re2) = (e * e, (re * re) as f32);

        if let Some(mut r) = self.active {
            let ext = (K_EXT * r.floor0).max(EXT_FRAC * r.peak).max(ABS_MIN);
            if ae > ext {
                r.last = pos;
                r.peak = r.peak.max(ae);
            }
            if nb >= r.start && eb.abs() > ext {
                r.last_b = nb;
            }
            r.sum_e += e2;
            r.sum_re += re2;
            if pos - r.last >= self.merge {
                self.active = None;
                self.close(r);
            } else if pos >= r.start - self.pre + self.lookahead as u64 {
                // Still going when its first sample must leave: too long for a click.
                self.active = None;
                self.reject(r);
            } else {
                self.active = Some(r);
            }
        } else {
            let thr = (self.k_floor * self.floor).max(self.k_peak * self.pk).max(ABS_MIN);
            if pos >= self.warmup && pos >= self.lock_until && ae > thr {
                self.active = Some(Open {
                    start: pos,
                    last: pos,
                    last_b: pos,
                    peak: ae,
                    floor0: self.floor,
                    sum_e: e2,
                    sum_re: re2,
                });
            } else {
                self.floor *= if ae > self.floor { self.floor_up } else { self.floor_down };
                self.floor = self.floor.max(1e-9);
                let old = self.ae_hist[(pos as usize).wrapping_sub(self.pk_delay) % AE_HIST];
                self.pk = old.max(self.pk * self.pk_decay);
            }
        }

        self.ae_hist[pos as usize % AE_HIST] = ae;
        self.tokens = (self.tokens + self.token_rate).min(REPAIR_BURST);
        self.hop_ctr += 1;
        if self.hop_ctr >= self.det_hop {
            self.hop_ctr = 0;
            // Don't fit the model to a click while it's being measured.
            if self.active.is_none() && pos + 1 >= self.det_win as u64 {
                self.refit_detector(pos);
            }
        }
    }

    fn close(&mut self, r: Open) {
        if r.sum_re >= self.broadband * r.sum_e && self.p_len < MAX_PENDING && self.tokens >= 1.0 {
            // `merge` > order, so the backward residual has covered every flagged sample by now.
            let end = r.last_b.min(r.last) + 1;
            let new = Region { g0: r.start - self.pre, g1: end + self.post, start: r.start, end };
            // A click's decaying tail can re-trigger just after its region closed. Repairing the
            // two separately would leave the first with the second inside its post-context, so a
            // still-pending neighbour closer than the AR order is extended instead.
            if self.p_len > 0 {
                let last = &mut self.pending[(self.p_head + self.p_len - 1) % MAX_PENDING];
                if new.g0 < last.g1 + self.ar_order as u64 && ((new.g1 - last.g0) as usize) <= self.m_max {
                    last.g1 = new.g1;
                    last.end = new.end;
                    return;
                }
            }
            let idx = (self.p_head + self.p_len) % MAX_PENDING;
            self.pending[idx] = new;
            self.p_len += 1;
            self.tokens -= 1.0;
        } else {
            self.reject(r);
        }
    }

    fn reject(&mut self, r: Open) {
        // Not a click: it's part of the signal, so it counts towards the recent peak.
        self.pk = self.pk.max(r.peak);
        self.lock_until = self.pos + self.lockout;
    }

    /// Replace region `r` in every channel's history by its anchored LSAR interpolation.
    /// Called at input position `self.pos` (= `r.g0 + lookahead`). Returns the region energy
    /// (before, after), or `None` if the region could not be repaired.
    fn repair(&mut self, r: Region, next_g0: u64, ch: usize) -> Option<(f64, f64)> {
        let p = self.ar_order;
        let w = self.ar_win;
        let g0 = r.g0;
        let m = r.g1.saturating_sub(g0) as usize;
        if m == 0 || m > self.m_max || g0 < w as u64 {
            return None;
        }
        // Post-context: what has arrived, but not into the next (still unrepaired) click.
        let known = (self.pos + 1).saturating_sub(r.g1) as usize;
        let clean = next_g0.saturating_sub(r.g1) as usize;
        let q = known.min(clean).min(p);
        let len = p + m + q;
        let w0 = (g0 as usize).wrapping_sub(w);
        let s0 = (g0 as usize).wrapping_sub(p);
        let (mut e_orig, mut e_new) = (0.0, 0.0);
        for c in 0..ch {
            let base = c * self.rsize;
            for (k, v) in self.work[..w].iter_mut().enumerate() {
                *v = self.ring[base + (w0.wrapping_add(k) & self.rmask)] as f64;
            }
            burg(&mut self.work[..w], &mut self.work2[..w], &mut self.ar[..=p], AR_NOISE_FLOOR);
            for (k, v) in self.seg[..len].iter_mut().enumerate() {
                *v = self.ring[base + (s0.wrapping_add(k) & self.rmask)] as f64;
            }
            self.orig[..m].copy_from_slice(&self.seg[p..p + m]);
            lsar(
                &self.ar[..=p],
                &self.lf_h,
                LF_ANCHOR,
                self.anchor_stride,
                &mut self.seg[..len],
                m,
                q,
                &mut self.cvec,
                &mut self.bvec,
                &mut self.band,
            );
            for j in 0..m {
                let old = self.orig[j];
                let v = self.seg[p + j];
                let new = if v.is_finite() { v.clamp(-4.0, 4.0) } else { old };
                e_orig += old * old;
                e_new += new * new;
                self.ring[base + (s0.wrapping_add(p + j) & self.rmask)] = new as f32;
            }
        }
        Some((e_orig, e_new))
    }
}

impl Module for DeClick {
    fn id(&self) -> ModuleId {
        ModuleId::DeClick
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize, channels: usize) {
        self.sr = sample_rate;
        self.channels = channels.clamp(1, MAX_CHANNELS);
        self.lookahead = (sample_rate * LOOKAHEAD_MS / 1000.0).floor() as usize;
        self.pre = self.samples(PRE_PAD_MS).max(1) as u64;
        self.post = self.samples(POST_PAD_MS).max(1) as u64;
        self.det_order = ((DET_ORDER_48K as f32 * sample_rate / 48_000.0).round() as usize).clamp(12, MAX_DET_ORDER);
        self.merge = (self.samples(MERGE_MS) as u64).max(self.post + 1).max(self.det_order as u64 + 1);
        self.lockout = self.samples(LOCKOUT_MS) as u64;
        self.det_win = self.samples(DET_WINDOW_MS).max(4 * self.det_order);
        self.det_hop = self.samples(DET_HOP_MS).max(1);
        // Capped at the 48 kHz order: repair cost grows ~p², and at 96 kHz the scaled order blew the
        // 64-frame callback budget (rt-safety review: 766 µs vs 667 µs).
        self.ar_order =
            ((AR_ORDER_48K as f32 * sample_rate / 48_000.0).round() as usize).clamp(24, AR_ORDER_48K.max(24));
        self.ar_win = self.samples(AR_WINDOW_MS).max(4 * self.ar_order);
        // Longest region: the flagged part must close before pre + lookahead runs out.
        self.m_max = self.lookahead + self.post as usize + 2;
        self.warmup = (self.ar_win + self.pre as usize + 1).max(self.det_win) as u64 + 1;
        let sr = sample_rate as f64;
        self.hp_det = [Biquad::highpass(sr, DET_HP_HZ, 0.541), Biquad::highpass(sr, DET_HP_HZ, 1.307)];
        let hi = HI_HP_HZ.min(0.25 * sr);
        self.hp_res = [Biquad::highpass(sr, hi, 0.541), Biquad::highpass(sr, hi, 1.307)];
        let nyq = 0.5 * sr;
        self.broadband = BROADBAND_OF_WHITE * ((nyq - hi) / (nyq - DET_HP_HZ)) as f32;
        self.anchor_stride = self.samples(ANCHOR_STRIDE_MS).max(1);
        // The anchor's low-pass must fit the QR band: 2K + 1 ≤ p + 1.
        self.lf_h = windowed_sinc_lowpass(self.ar_order / 2, LF_ANCHOR_HZ.min(0.2 * sr), sr);
        let dsize = (self.det_win + MAX_DET_ORDER + 2).next_power_of_two();
        self.dring = vec![0.0; dsize];
        self.dmask = dsize - 1;
        self.rsize = (self.ar_win + self.lookahead + 4).next_power_of_two();
        self.rmask = self.rsize - 1;
        self.ring = vec![0.0; self.channels * self.rsize];
        self.det_hann = hann(self.det_win);
        self.work = vec![0.0; self.det_win.max(self.ar_win)];
        self.work2 = vec![0.0; self.ar_win];
        let p = self.ar_order;
        self.orig = vec![0.0; self.m_max];
        self.seg = vec![0.0; 2 * p + self.m_max];
        self.cvec = vec![0.0; self.m_max + p];
        self.bvec = vec![0.0; self.m_max + p];
        self.band = vec![0.0; self.m_max * (p + 1)];
        self.floor_up = crate::db_to_amp(FLOOR_SLEW_DB_PER_S / sample_rate);
        self.floor_down = 1.0 / self.floor_up;
        self.pk_decay = (-1.0 / (PEAK_TAU_S * sample_rate)).exp();
        self.pk_delay = self.samples(PEAK_DELAY_MS).clamp(1, AE_HIST - 1);
        self.token_rate = MAX_REPAIRS_PER_S / sample_rate;
        self.reset();
    }

    fn reset(&mut self) {
        self.hp_det.iter_mut().chain(self.hp_res.iter_mut()).for_each(Biquad::reset);
        self.det_a = [0.0; MAX_DET_ORDER + 1];
        self.det_a[0] = 1.0;
        self.dring.fill(0.0);
        self.ring.fill(0.0);
        self.hop_ctr = 0;
        self.floor = 1e-9;
        self.pk = 0.0;
        self.ae_hist = [0.0; AE_HIST];
        self.active = None;
        self.p_head = 0;
        self.p_len = 0;
        self.pos = 0;
        self.lock_until = 0;
        self.tokens = REPAIR_BURST;
    }

    fn latency_samples(&self) -> usize {
        self.lookahead
    }

    fn process(&mut self, block: &mut AudioBlock<'_>, telemetry: &mut TelemetryWriter) {
        if self.ring.is_empty() {
            return;
        }
        let n = block.frames();
        let ch = block.channels().min(self.channels);
        if ch == 0 {
            return; // 0/0 would poison the detector filters for good
        }
        let lat = self.lookahead as u64;
        // CPU guard per callback (the token bucket only limits per second): at most one repair per
        // process call; a second region due in the same call passes through unrepaired.
        let mut repairs_left = MAX_REPAIRS_PER_CALL;
        let chain_start = telemetry.block_start();
        let block_in_start = self.pos;
        let mut max_removed = 0.0f32;
        for i in 0..n {
            let pos = self.pos;
            let w = pos as usize & self.rmask;
            let mut sum = 0.0f32;
            for c in 0..ch {
                let raw = block.channel(c)[i];
                let x = if raw.is_finite() { raw } else { 0.0 };
                self.ring[c * self.rsize + w] = x;
                sum += x;
            }
            self.detect(sum / ch as f32);

            // Repair every accepted region whose first sample is about to be output.
            while self.p_len > 0 && pos >= lat && self.pending[self.p_head].g0 <= pos - lat {
                let mut r = self.pending[self.p_head];
                self.p_head = (self.p_head + 1) % MAX_PENDING;
                self.p_len -= 1;
                r.g0 = r.g0.max(pos - lat); // never touch samples already output
                let next_g0 = if self.p_len > 0 {
                    self.pending[self.p_head].g0
                } else if let Some(o) = self.active {
                    o.start - self.pre
                } else {
                    u64::MAX
                };
                if repairs_left == 0 {
                    continue;
                }
                repairs_left -= 1;
                let Some((e_old, e_new)) = self.repair(r, next_g0, ch) else { continue };
                let removed = ((10.0 * (e_old.max(1e-30) / e_new.max(1e-30)).log10()) as f32).clamp(-200.0, 200.0);
                max_removed = max_removed.max(removed);
                telemetry.push_event(Event {
                    module: ModuleId::DeClick,
                    kind: EventKind::Click,
                    // Module-input clock → chain-input clock (offset of this block).
                    start_sample: (chain_start as i64 + r.start as i64 - block_in_start as i64).max(0) as u64,
                    duration_samples: (r.end - r.start) as u32,
                    energy_removed_db: removed,
                });
            }

            let o = (pos as usize).wrapping_sub(self.lookahead) & self.rmask;
            for c in 0..ch {
                block.channel_mut(c)[i] = self.ring[c * self.rsize + o];
            }
            self.pos += 1;
        }
        telemetry.set_gain_reduction(ModuleId::DeClick, max_removed);
    }

    fn params(&self) -> &[ParamDesc] {
        &PARAMS
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if let Some(p) = PARAMS.get(index) {
            self.set_sensitivity(p.clamp(value));
        }
    }

    fn primary_param(&self) -> Option<usize> {
        Some(0) // sensitivity: a repair has no depth; more sensitive = more clicks repaired
    }

    fn param(&self, index: usize) -> f32 {
        match index {
            0 => self.sensitivity,
            _ => f32::NAN,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levinson_recovers_an_ar2_process() {
        // x[n] = 1.6 x[n−1] − 0.8 x[n−2] + w[n] → a = [1, −1.6, 0.8].
        let mut x = vec![0.0f64; 20_000];
        let mut s = 1u64;
        for n in 2..x.len() {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let w = ((s >> 33) as f64 / (1u64 << 31) as f64) - 0.5;
            x[n] = 1.6 * x[n - 1] - 0.8 * x[n - 2] + w;
        }
        let mut r = [0.0; 3];
        autocorr(&x, &mut r);
        let mut a = [0.0; 3];
        levinson(&r, &mut a);
        assert!((a[1] + 1.6).abs() < 0.02 && (a[2] - 0.8).abs() < 0.02, "{a:?}");
    }

    #[test]
    fn lsar_fills_a_gap_in_a_sine() {
        let p = 8;
        let f = 0.03;
        let x: Vec<f64> = (0..2000).map(|i| (2.0 * core::f64::consts::PI * f * i as f64).sin()).collect();
        let mut r = vec![0.0; p + 1];
        autocorr(&x[..1000], &mut r);
        r[0] *= 1.0 + 1e-9;
        let mut a = vec![0.0; p + 1];
        levinson(&r, &mut a);
        let (m, q) = (40, p);
        let mut y: Vec<f64> = x[1000 - p..1000 + m + q].to_vec();
        for v in &mut y[p..p + m] {
            *v = 5.0;
        }
        let (mut c, mut b, mut band) = (vec![0.0; m + q], vec![0.0; m + q], vec![0.0; m * (p + 1)]);
        lsar(&a, &[], 0.0, 1, &mut y, m, q, &mut c, &mut b, &mut band);
        let err = (0..m).map(|j| (y[p + j] - x[1000 + j]).abs()).fold(0.0, f64::max);
        assert!(err < 1e-3, "max error {err}");
    }

    #[test]
    fn burg_recovers_an_ar2_process() {
        let mut x = vec![0.0f64; 5_000];
        let mut s = 7u64;
        for n in 2..x.len() {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let w = ((s >> 33) as f64 / (1u64 << 31) as f64) - 0.5;
            x[n] = 1.6 * x[n - 1] - 0.8 * x[n - 2] + w;
        }
        let mut b = vec![0.0; x.len()];
        let mut a = [0.0; 3];
        burg(&mut x, &mut b, &mut a, 0.0);
        assert!((a[1] + 1.6).abs() < 0.03 && (a[2] - 0.8).abs() < 0.03, "{a:?}");
        // Digital silence gives the trivial (identity) filter, not NaN.
        let mut z = vec![0.0; 100];
        let mut a = [0.0; 5];
        burg(&mut z, &mut vec![0.0; 100], &mut a, AR_NOISE_FLOOR);
        assert_eq!(a, [1.0, 0.0, 0.0, 0.0, 0.0]);
    }

    /// The anchored solve on a sharp model of a chord: a 1.25 ms gap whose observed samples carry
    /// a high-frequency doublet (the click) is filled to within −25 dB, with or without
    /// post-context. Without the anchor the low band drifts (the classic long-gap failure).
    #[test]
    fn anchored_lsar_fills_a_chord_gap() {
        let sr = 48_000.0;
        let x: Vec<f64> = (0..6000)
            .map(|i| {
                [220.0, 330.0, 440.0, 660.0, 1320.0, 2640.0]
                    .iter()
                    .map(|f| (2.0 * core::f64::consts::PI * f * i as f64 / sr).sin())
                    .sum::<f64>()
                    * 0.05
            })
            .collect();
        let (p, g0, m) = (32, 5000, 60);
        let mut a = vec![0.0; p + 1];
        let mut f = x[g0 - 480..g0].to_vec();
        burg(&mut f, &mut vec![0.0; 480], &mut a, AR_NOISE_FLOOR);
        let lf = windowed_sinc_lowpass(p / 2, LF_ANCHOR_HZ, sr);
        let snr = |lambda: f64, q: usize| {
            let mut y = x[g0 - p..g0 + m + q].to_vec();
            y[p + 10] += 0.2;
            y[p + 11] -= 0.2;
            let (mut c, mut z, mut rb) = (vec![0.0; m + q], vec![0.0; m], vec![0.0; m * (p + 1)]);
            lsar(&a, &lf, lambda, 4, &mut y, m, q, &mut c, &mut z, &mut rb);
            let e: f64 = (0..m).map(|j| (y[p + j] - x[g0 + j]).powi(2)).sum();
            let s: f64 = (0..m).map(|j| x[g0 + j].powi(2)).sum();
            10.0 * (s / e).log10()
        };
        for q in [0, 8, 32] {
            assert!(snr(LF_ANCHOR, q) > 25.0, "q {q}: {:.1} dB", snr(LF_ANCHOR, q));
        }
        assert!(snr(LF_ANCHOR, 8) > snr(0.0, 8) + 6.0, "anchor should help: {} vs {}", snr(LF_ANCHOR, 8), snr(0.0, 8));
    }

    /// A two-sample click gets a region of about its own length (backward residual), not the
    /// LPC order, and the output is bit-exact delayed input everywhere else.
    #[test]
    fn short_click_gets_a_short_region() {
        let sr = 48_000.0;
        let n = 20_000;
        let clean: Vec<f32> =
            (0..n).map(|i| 0.1 * (2.0 * core::f32::consts::PI * 300.0 * i as f32 / sr).sin()).collect();
        let mut x = clean.clone();
        x[15_000] += 0.3;
        x[15_001] -= 0.3;
        let mut d = DeClick::new();
        d.prepare(sr, 256, 1);
        let lat = d.latency_samples();
        let mut buf = x.clone();
        buf.extend(core::iter::repeat_n(0.0, lat));
        let mut t = TelemetryWriter::new();
        let mut events = Vec::new();
        for (k, chunk) in buf.chunks_mut(256).enumerate() {
            t.set_block_start((k * 256) as u64);
            d.process(&mut AudioBlock::from_planar(chunk, 1), &mut t);
            events.extend_from_slice(t.events());
            t.clear_events();
        }
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].start_sample, 15_000);
        assert!(events[0].duration_samples <= 6, "{events:?}"); // order is 16
        let out = &buf[lat..];
        let err = (14_990..15_020).map(|i| (out[i] - clean[i]).abs()).fold(0.0f32, f32::max);
        assert!(err < 0.01, "repair error {err}");
        assert!(out[..14_990] == x[..14_990] && out[15_020..] == x[15_020..]);
    }

    #[test]
    fn latency_is_2_5_ms_floored() {
        let mut d = DeClick::new();
        for (sr, lat) in [(44_100.0, 110), (48_000.0, 120), (96_000.0, 240)] {
            d.prepare(sr, 512, 2);
            assert_eq!(d.latency_samples(), lat);
        }
    }

    #[test]
    fn sensitivity_scales_thresholds() {
        let mut d = DeClick::new();
        d.set_param(0, 1.0);
        assert!(d.k_floor < K_FLOOR && d.k_peak < K_PEAK);
        d.set_param(0, f32::NAN);
        assert_eq!(d.param(0), 0.5);
        assert!((d.k_floor - K_FLOOR).abs() < 1e-5);
        assert!(d.param(1).is_nan());
    }
}
