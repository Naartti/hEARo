//! [`AudioBlock`]: a planar view over a block of multi-channel audio.

/// Maximum number of channels a block can hold. Covers multi-output interfaces.
pub const MAX_CHANNELS: usize = 8;

/// A planar, mutable view of `channels × frames` samples.
///
/// Each channel is its own contiguous slice, so per-channel DSP (biquads, FFTs) is cache
/// friendly, and the chain can take sub-blocks without copying. Constructing a block never
/// allocates.
#[derive(Debug)]
pub struct AudioBlock<'a> {
    chans: [&'a mut [f32]; MAX_CHANNELS],
    channels: usize,
    frames: usize,
}

impl<'a> AudioBlock<'a> {
    /// View a planar buffer (`channel 0 samples, then channel 1 samples, ...`) as a block.
    ///
    /// # Panics
    /// If `channels` is 0 or above [`MAX_CHANNELS`], or `data.len()` isn't a multiple of it.
    pub fn from_planar(data: &'a mut [f32], channels: usize) -> Self {
        assert!((1..=MAX_CHANNELS).contains(&channels), "unsupported channel count");
        assert_eq!(data.len() % channels, 0, "planar buffer length must be a multiple of channels");
        let frames = data.len() / channels;
        let mut chans: [&'a mut [f32]; MAX_CHANNELS] = Default::default();
        let mut rest = data;
        for slot in chans.iter_mut().take(channels) {
            let (head, tail) = rest.split_at_mut(frames);
            *slot = head;
            rest = tail;
        }
        Self { chans, channels, frames }
    }

    /// Build a block from separate channel slices of equal length.
    ///
    /// # Panics
    /// If the slices differ in length, or the count is 0 or above [`MAX_CHANNELS`].
    pub fn from_channels<I>(slices: I) -> Self
    where
        I: IntoIterator<Item = &'a mut [f32]>,
    {
        let mut chans: [&'a mut [f32]; MAX_CHANNELS] = Default::default();
        let mut channels = 0;
        for s in slices {
            assert!(channels < MAX_CHANNELS, "too many channels");
            chans[channels] = s;
            channels += 1;
        }
        assert!(channels > 0, "a block needs at least one channel");
        let frames = chans[0].len();
        assert!(chans[..channels].iter().all(|c| c.len() == frames), "channel length mismatch");
        Self { chans, channels, frames }
    }

    /// Number of channels.
    #[inline]
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Number of frames (samples per channel).
    #[inline]
    pub fn frames(&self) -> usize {
        self.frames
    }

    /// One channel's samples.
    #[inline]
    pub fn channel(&self, c: usize) -> &[f32] {
        debug_assert!(c < self.channels, "channel {c} out of range");
        &self.chans[c][..]
    }

    /// One channel's samples, mutably.
    #[inline]
    pub fn channel_mut(&mut self, c: usize) -> &mut [f32] {
        debug_assert!(c < self.channels, "channel {c} out of range");
        &mut self.chans[c][..]
    }

    /// Iterate channels mutably.
    pub fn channels_mut(&mut self) -> impl Iterator<Item = &mut [f32]> {
        self.chans[..self.channels].iter_mut().map(|c| &mut c[..])
    }

    /// Iterate channels.
    pub fn channels_iter(&self) -> impl Iterator<Item = &[f32]> {
        self.chans[..self.channels].iter().map(|c| &c[..])
    }

    /// A sub-block covering frames `start..start + len` of every channel. Never allocates.
    pub fn slice_mut(&mut self, start: usize, len: usize) -> AudioBlock<'_> {
        let mut chans: [&mut [f32]; MAX_CHANNELS] = Default::default();
        for (dst, src) in chans.iter_mut().zip(self.chans[..self.channels].iter_mut()) {
            *dst = &mut src[start..start + len];
        }
        AudioBlock { chans, channels: self.channels, frames: len }
    }

    /// Copy all samples from `other` (same shape) into this block.
    pub fn copy_from(&mut self, other: &AudioBlock<'_>) {
        debug_assert!(other.channels == self.channels && other.frames == self.frames, "block shape mismatch");
        for c in 0..self.channels {
            self.chans[c].copy_from_slice(other.channel(c));
        }
    }

    /// True if every sample is finite (no NaN or infinity).
    pub fn is_finite(&self) -> bool {
        self.channels_iter().all(|c| c.iter().all(|x| x.is_finite()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planar_layout() {
        let mut data: Vec<f32> = (0..8).map(|i| i as f32).collect();
        let b = AudioBlock::from_planar(&mut data, 2);
        assert_eq!(b.frames(), 4);
        assert_eq!(b.channel(0), &[0.0, 1.0, 2.0, 3.0]);
        assert_eq!(b.channel(1), &[4.0, 5.0, 6.0, 7.0]);
    }

    #[test]
    fn slicing_writes_through() {
        let mut data = vec![0.0f32; 8];
        let mut b = AudioBlock::from_planar(&mut data, 2);
        {
            let mut s = b.slice_mut(1, 2);
            s.channel_mut(0).fill(1.0);
            s.channel_mut(1).fill(2.0);
        }
        assert_eq!(data, vec![0.0, 1.0, 1.0, 0.0, 0.0, 2.0, 2.0, 0.0]);
    }

    #[test]
    #[should_panic(expected = "channel length mismatch")]
    fn mismatched_channels_panic() {
        let mut a = [0.0f32; 3];
        let mut b = [0.0f32; 2];
        let _ = AudioBlock::from_channels([&mut a[..], &mut b[..]]);
    }
}
