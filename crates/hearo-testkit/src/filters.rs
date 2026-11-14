//! Minimal RBJ biquads for building test signals (deliberately independent of hearo-dsp).

/// Direct-form-I biquad.
#[derive(Debug, Clone, Copy)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    fn from_coeffs(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        Self { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0, x1: 0.0, x2: 0.0, y1: 0.0, y2: 0.0 }
    }

    fn w0_alpha(sr: f32, f: f32, q: f32) -> (f32, f32) {
        let w0 = 2.0 * core::f32::consts::PI * f / sr;
        (w0, w0.sin() / (2.0 * q))
    }

    /// Low-pass.
    pub fn lowpass(sr: f32, f: f32, q: f32) -> Self {
        let (w0, a) = Self::w0_alpha(sr, f, q);
        let c = w0.cos();
        Self::from_coeffs((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + a, -2.0 * c, 1.0 - a)
    }

    /// High-pass.
    pub fn highpass(sr: f32, f: f32, q: f32) -> Self {
        let (w0, a) = Self::w0_alpha(sr, f, q);
        let c = w0.cos();
        Self::from_coeffs((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + a, -2.0 * c, 1.0 - a)
    }

    /// Band-pass with 0 dB peak gain.
    pub fn bandpass(sr: f32, f: f32, q: f32) -> Self {
        let (w0, a) = Self::w0_alpha(sr, f, q);
        let c = w0.cos();
        Self::from_coeffs(a, 0.0, -a, 1.0 + a, -2.0 * c, 1.0 - a)
    }

    /// Retune to a new band-pass while keeping the filter state (for gliding formants).
    pub fn retune_bandpass(&mut self, sr: f32, f: f32, q: f32) {
        let n = Self::bandpass(sr, f, q);
        (self.b0, self.b1, self.b2, self.a1, self.a2) = (n.b0, n.b1, n.b2, n.a1, n.a2);
    }

    /// Filter one sample.
    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    /// Filter a buffer in place.
    pub fn run(&mut self, buf: &mut [f32]) {
        for x in buf {
            *x = self.tick(*x);
        }
    }
}
