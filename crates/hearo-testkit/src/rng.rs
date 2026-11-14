//! A tiny, dependency-free, seeded PRNG (SplitMix64). Its output is fixed forever, so
//! generated test signals never change because an upstream crate changed.

/// SplitMix64 generator.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    /// Seeded generator.
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// Next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform in `[lo, hi)`.
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    /// Uniform integer in `[lo, hi)`.
    pub fn range_usize(&mut self, lo: usize, hi: usize) -> usize {
        assert!(hi > lo);
        lo + (self.next_u64() % (hi - lo) as u64) as usize
    }

    /// Standard normal sample (Box–Muller).
    pub fn gaussian(&mut self) -> f32 {
        let u1 = self.unit().max(1e-12);
        let u2 = self.unit();
        (-2.0 * u1.ln()).sqrt() * (2.0 * core::f32::consts::PI * u2).cos()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_in_range() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..1000 {
            let x = a.unit();
            assert_eq!(x, b.unit());
            assert!((0.0..1.0).contains(&x));
        }
        // Pin the stream: if this changes, every golden signal changes.
        assert_eq!(Rng::new(1).next_u64(), 0x910A_2DEC_8902_5CC1);
    }
}
