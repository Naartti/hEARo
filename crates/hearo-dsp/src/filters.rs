//! Filters used by modules. Biquads run in f64: high-Q filters at low frequencies (2 Hz wide
//! notches at 50 Hz) put poles within ~1e-4 of the unit circle, where f32 coefficient rounding
//! would visibly move the notch and limit its depth.

use core::f64::consts::PI;

/// Transposed direct-form-II biquad in f64.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    s1: f64,
    s2: f64,
}

impl Biquad {
    fn normalized(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Self { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0, s1: 0.0, s2: 0.0 }
    }

    fn w0_alpha(sr: f64, f: f64, q: f64) -> (f64, f64) {
        let w0 = 2.0 * PI * f / sr;
        (w0, w0.sin() / (2.0 * q))
    }

    /// A filter that passes everything unchanged (`y = x`).
    pub const IDENTITY: Biquad = Biquad { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0, s1: 0.0, s2: 0.0 };

    /// RBJ notch at `f` with quality `q` (-3 dB bandwidth = f / q).
    pub fn notch(sr: f64, f: f64, q: f64) -> Self {
        let (w0, a) = Self::w0_alpha(sr, f, q);
        let c = w0.cos();
        Self::normalized(1.0, -2.0 * c, 1.0, 1.0 + a, -2.0 * c, 1.0 - a)
    }

    /// RBJ high-pass.
    pub fn highpass(sr: f64, f: f64, q: f64) -> Self {
        let (w0, a) = Self::w0_alpha(sr, f, q);
        let c = w0.cos();
        Self::normalized((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + a, -2.0 * c, 1.0 - a)
    }

    /// RBJ low-pass.
    pub fn lowpass(sr: f64, f: f64, q: f64) -> Self {
        let (w0, a) = Self::w0_alpha(sr, f, q);
        let c = w0.cos();
        Self::normalized((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + a, -2.0 * c, 1.0 - a)
    }

    /// Q values of the two sections of a 4th-order Butterworth filter (cascade both at the same
    /// corner for a maximally flat 24 dB/octave slope, −3 dB at the corner).
    pub const BUTTERWORTH4_Q: [f64; 2] = [0.541_196_100_146_197, 1.306_562_964_876_376_7];

    /// RBJ band-pass with 0 dB peak gain.
    pub fn bandpass(sr: f64, f: f64, q: f64) -> Self {
        let (w0, a) = Self::w0_alpha(sr, f, q);
        let c = w0.cos();
        Self::normalized(a, 0.0, -a, 1.0 + a, -2.0 * c, 1.0 - a)
    }

    /// Take `other`'s coefficients but keep this filter's state, so a parameter glide (small
    /// coefficient steps on a running filter) doesn't restart it from silence.
    #[inline]
    pub fn retune(&mut self, other: &Biquad) {
        let (s1, s2) = (self.s1, self.s2);
        *self = Self { s1, s2, ..*other };
    }

    /// Clear the filter state.
    #[inline]
    pub fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }

    /// Take over another filter's state (e.g. after recomputing coefficients for a new corner,
    /// so a running filter continues instead of restarting from silence).
    #[inline]
    pub fn copy_state_from(&mut self, other: &Biquad) {
        self.s1 = other.s1;
        self.s2 = other.s2;
    }

    /// Filter one sample.
    #[inline]
    pub fn tick(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.s1;
        self.s1 = self.b1 * x - self.a1 * y + self.s2;
        self.s2 = self.b2 * x - self.a2 * y;
        y
    }

    /// Solve `y = u − c·H(y)` sample by sample (this filter `H` in a delay-free negative
    /// feedback loop; transfer function `1 / (1 + c·H)`), update the state with `y`, return `y`.
    /// Possible because `H`'s output is `b0·y + s1`. With a positive-real `H` such as
    /// [`bandpass`](Self::bandpass) it is stable for any `c ≥ 0`, and `c = 0` returns `u` exactly.
    #[inline]
    pub fn tick_feedback(&mut self, u: f64, c: f64) -> f64 {
        let y = (u - c * self.s1) / (1.0 + c * self.b0);
        self.tick(y);
        y
    }

    /// Magnitude response at `f` Hz.
    pub fn magnitude(&self, sr: f64, f: f64) -> f64 {
        let w = 2.0 * PI * f / sr;
        let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
        let num = ((self.b0 + self.b1 * c1 + self.b2 * c2).powi(2) + (self.b1 * s1 + self.b2 * s2).powi(2)).sqrt();
        let den = ((1.0 + self.a1 * c1 + self.a2 * c2).powi(2) + (self.a1 * s1 + self.a2 * s2).powi(2)).sqrt();
        num / den
    }
}

/// One-pole smoothing coefficient for time constant `tau_s` at `sr` (y += a·(x − y)).
#[inline]
pub fn one_pole_coeff(sr: f32, tau_s: f32) -> f32 {
    1.0 - (-1.0 / (tau_s * sr)).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notch_is_deep_at_center_and_flat_away() {
        let sr = 48_000.0;
        let n = Biquad::notch(sr, 50.0, 25.0); // 2 Hz wide
        assert!(n.magnitude(sr, 50.0) < 1e-6);
        assert!((n.magnitude(sr, 60.0) - 1.0).abs() < 0.01);
        assert!((n.magnitude(sr, 1000.0) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn highpass_passes_highs_and_cuts_lows() {
        let sr = 48_000.0;
        let h = Biquad::highpass(sr, 1500.0, 0.707);
        assert!((h.magnitude(sr, 10_000.0) - 1.0).abs() < 0.01);
        assert!(h.magnitude(sr, 150.0) < 0.02);
    }

    #[test]
    fn lowpass_passes_lows_and_cuts_highs() {
        let sr = 48_000.0;
        let l = Biquad::lowpass(sr, 150.0, 0.707);
        assert!((l.magnitude(sr, 10.0) - 1.0).abs() < 0.01);
        assert!((l.magnitude(sr, 150.0) - core::f64::consts::FRAC_1_SQRT_2).abs() < 0.01);
        assert!(l.magnitude(sr, 1500.0) < 0.011);
    }

    #[test]
    fn butterworth4_cascade_is_flat_and_minus_3_db_at_corner() {
        let sr = 96_000.0;
        let [q1, q2] = Biquad::BUTTERWORTH4_Q;
        let (a, b) = (Biquad::highpass(sr, 150.0, q1), Biquad::highpass(sr, 150.0, q2));
        let m = |f: f64| a.magnitude(sr, f) * b.magnitude(sr, f);
        assert!((m(150.0) - core::f64::consts::FRAC_1_SQRT_2).abs() < 1e-3, "{}", m(150.0));
        assert!((m(600.0) - 1.0).abs() < 0.01, "flat passband: {}", m(600.0));
        // 4th order: 24 dB per octave well below the corner.
        let slope = 20.0 * (m(37.5) / m(18.75)).log10();
        assert!((slope - 24.0).abs() < 0.5, "{slope}");
    }

    #[test]
    fn bandpass_peaks_at_unity() {
        let sr = 44_100.0;
        let b = Biquad::bandpass(sr, 150.0, 75.0);
        assert!((b.magnitude(sr, 150.0) - 1.0).abs() < 1e-6);
        assert!(b.magnitude(sr, 170.0) < 0.1);
    }

    #[test]
    fn retune_keeps_state_and_takes_coefficients() {
        let sr = 48_000.0;
        let mut a = Biquad::bandpass(sr, 5000.0, 1.0);
        for i in 0..100 {
            a.tick((i as f64 * 0.3).sin());
        }
        let (s1, s2) = (a.s1, a.s2);
        let b = Biquad::bandpass(sr, 7000.0, 2.0);
        a.retune(&b);
        assert_eq!((a.s1, a.s2), (s1, s2));
        assert_eq!(a.magnitude(sr, 3000.0), b.magnitude(sr, 3000.0));
    }

    #[test]
    fn feedback_loop_matches_one_over_one_plus_c_h() {
        let sr = 48_000.0;
        let c = 3.0;
        for f in [1000.0, 6000.0, 7000.0, 9000.0] {
            let mut h = Biquad::bandpass(sr, 7000.0, 1.0);
            let mut energy = 0.0;
            for i in 0..48_000 {
                let y = h.tick_feedback((2.0 * PI * f * i as f64 / sr).sin(), c);
                if i >= 24_000 {
                    energy += y * y;
                }
            }
            let amplitude = (2.0 * energy / 24_000.0).sqrt();
            // |1 / (1 + c·H)| with H = 1 / (1 + jQ(r − 1/r)) (analog prototype; close below 10 kHz).
            let r = f / 7000.0;
            let y = r - 1.0 / r;
            let expect = (1.0 + y * y).sqrt() / ((1.0 + c).powi(2) + y * y).sqrt();
            assert!((amplitude - expect).abs() < 0.02, "{f} Hz: {amplitude} vs {expect}");
        }
        let mut h = Biquad::bandpass(sr, 7000.0, 1.0);
        assert_eq!(h.tick_feedback(0.123, 0.0), 0.123, "c = 0 is a wire");
    }

    #[test]
    fn tick_matches_magnitude() {
        let sr = 48_000.0;
        let mut n = Biquad::notch(sr, 1000.0, 2.0);
        let mut energy = 0.0;
        for i in 0..48_000 {
            let y = n.tick((2.0 * PI * 3000.0 * i as f64 / sr).sin());
            if i >= 24_000 {
                energy += y * y;
            }
        }
        let amplitude = (2.0 * energy / 24_000.0).sqrt();
        assert!((amplitude - n.magnitude(sr, 3000.0)).abs() < 1e-3, "{amplitude}");
    }
}
