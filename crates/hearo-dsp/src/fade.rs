//! Smooth crossfade curve shared by module toggles and global bypass.

/// Raised-cosine crossfade gain for position `pos` of `len` (0.0 at start, 1.0 at end).
///
/// The two gains `g` and `1 - g` always sum to 1, which is right for crossfading *correlated*
/// signals (dry and processed versions of the same audio): no level bump mid-fade.
#[inline]
pub fn fade_gain(pos: usize, len: usize) -> f32 {
    if len == 0 {
        return 1.0;
    }
    let t = (pos as f32 / len as f32).clamp(0.0, 1.0);
    0.5 - 0.5 * (core::f32::consts::PI * t).cos()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_and_monotonic() {
        assert_eq!(fade_gain(0, 100), 0.0);
        assert_eq!(fade_gain(100, 100), 1.0);
        let mut prev = -1.0;
        for i in 0..=100 {
            let g = fade_gain(i, 100);
            assert!(g >= prev);
            prev = g;
        }
    }
}
