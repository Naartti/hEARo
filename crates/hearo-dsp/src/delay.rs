//! Multi-channel integer delay line used for latency compensation.

use crate::AudioBlock;

/// Delays every channel by a fixed number of samples. Bit-exact: samples are copied, never
/// scaled, which is what makes bypass bit-exact.
#[derive(Debug, Default)]
pub struct MultiDelay {
    buf: Vec<f32>,
    delay: usize,
    channels: usize,
    pos: usize,
}

impl MultiDelay {
    /// Allocate for `channels` channels and a delay of `delay` samples (may allocate).
    pub fn prepare(&mut self, channels: usize, delay: usize) {
        self.channels = channels;
        self.delay = delay;
        self.buf = vec![0.0; channels * delay];
        self.pos = 0;
    }

    /// Delay in samples.
    pub fn delay(&self) -> usize {
        self.delay
    }

    /// Zero the history.
    pub fn reset(&mut self) {
        self.buf.fill(0.0);
        self.pos = 0;
    }

    /// Delay the block in place. Real-time safe.
    pub fn process(&mut self, block: &mut AudioBlock<'_>) {
        let d = self.delay;
        if d == 0 {
            return;
        }
        debug_assert_eq!(block.channels(), self.channels, "delay prepared for a different channel count");
        let channels = block.channels().min(self.channels);
        let frames = block.frames();
        for c in 0..channels {
            let line = &mut self.buf[c * d..(c + 1) * d];
            let mut p = self.pos;
            for x in block.channel_mut(c).iter_mut() {
                core::mem::swap(&mut line[p], x);
                p += 1;
                if p == d {
                    p = 0;
                }
            }
        }
        self.pos = (self.pos + frames) % d;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delays_exactly() {
        let mut d = MultiDelay::default();
        d.prepare(1, 3);
        let mut data: Vec<f32> = (1..=8).map(|i| i as f32).collect();
        // Process in two uneven chunks to exercise the wrap-around.
        {
            let mut b = AudioBlock::from_planar(&mut data[..5], 1);
            d.process(&mut b);
        }
        {
            let mut b = AudioBlock::from_planar(&mut data[5..], 1);
            d.process(&mut b);
        }
        assert_eq!(data, vec![0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
    }

    #[test]
    fn zero_delay_is_identity() {
        let mut d = MultiDelay::default();
        d.prepare(2, 0);
        let mut data = vec![1.0, 2.0, 3.0, 4.0];
        let mut b = AudioBlock::from_planar(&mut data, 2);
        d.process(&mut b);
        assert_eq!(data, vec![1.0, 2.0, 3.0, 4.0]);
    }
}
