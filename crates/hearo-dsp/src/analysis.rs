//! Spectral analysis: a streaming STFT analyser and 1/N-octave log-frequency bands.
//!
//! This feeds the visualizer and the offline reports. See
//! `docs/deep-dives/03-signals-and-spectra.md` for the theory.

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

/// Periodic Hann window of length `n` (the right choice for STFTs with overlap).
pub fn hann(n: usize) -> Vec<f32> {
    (0..n).map(|i| 0.5 - 0.5 * (2.0 * core::f32::consts::PI * i as f32 / n as f32).cos()).collect()
}

/// Power (not amplitude) to dB, floored to stay finite.
#[inline]
pub fn power_to_db(p: f32) -> f32 {
    10.0 * p.max(1e-20).log10()
}

/// Streaming, mono STFT analyser producing power spectra every `hop` samples.
///
/// Power is normalised so a full-scale sine reads 0 dB at its peak bin.
pub struct StftAnalyzer {
    fft: Arc<dyn RealToComplex<f32>>,
    size: usize,
    hop: usize,
    window: Vec<f32>,
    ring: Vec<f32>,
    pos: usize,
    total: u64,
    frame: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    power: Vec<f32>,
    norm: f32,
    enbw_bins: f32,
}

impl core::fmt::Debug for StftAnalyzer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StftAnalyzer").field("size", &self.size).field("hop", &self.hop).finish()
    }
}

impl StftAnalyzer {
    /// Allocate an analyser with FFT `size` and `hop` (not real-time safe).
    pub fn new(size: usize, hop: usize) -> Self {
        assert!(size >= 2 && hop >= 1 && hop <= size);
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(size);
        let window = hann(size);
        let sum: f32 = window.iter().sum();
        let sum_sq: f32 = window.iter().map(|w| w * w).sum();
        let enbw_bins = size as f32 * sum_sq / (sum * sum);
        let spectrum = fft.make_output_vec();
        let scratch = fft.make_scratch_vec();
        Self {
            power: vec![0.0; spectrum.len()],
            fft,
            size,
            hop,
            window,
            ring: vec![0.0; size],
            pos: 0,
            total: 0,
            frame: vec![0.0; size],
            spectrum,
            scratch,
            norm: (2.0 / sum).powi(2),
            enbw_bins,
        }
    }

    /// FFT size.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Number of bins in each power spectrum (`size / 2 + 1`).
    pub fn bins(&self) -> usize {
        self.power.len()
    }

    /// Equivalent noise bandwidth of the window in bins (1.5 for Hann).
    pub fn enbw_bins(&self) -> f32 {
        self.enbw_bins
    }

    /// Clear history.
    pub fn reset(&mut self) {
        self.ring.fill(0.0);
        self.pos = 0;
        self.total = 0;
    }

    /// Push samples; `on_frame` receives each completed power spectrum. Real-time safe.
    pub fn push(&mut self, input: &[f32], mut on_frame: impl FnMut(&[f32])) {
        for &x in input {
            self.ring[self.pos] = x;
            self.pos = (self.pos + 1) % self.size;
            self.total += 1;
            if self.total >= self.size as u64 && (self.total - self.size as u64).is_multiple_of(self.hop as u64) {
                let (newer, older) = self.ring.split_at(self.pos);
                let split = older.len();
                self.frame[..split].copy_from_slice(older);
                self.frame[split..].copy_from_slice(newer);
                for (s, w) in self.frame.iter_mut().zip(&self.window) {
                    *s *= w;
                }
                // Sizes come from the planner, so this cannot fail.
                let _ = self.fft.process_with_scratch(&mut self.frame, &mut self.spectrum, &mut self.scratch);
                for (p, c) in self.power.iter_mut().zip(&self.spectrum) {
                    *p = c.norm_sqr() * self.norm;
                }
                on_frame(&self.power);
            }
        }
    }
}

/// Maps FFT bins to log-spaced bands (e.g. 1/12 octave from 20 Hz to 20 kHz).
#[derive(Debug, Clone)]
pub struct LogBands {
    centers: Vec<f32>,
    ranges: Vec<(usize, usize)>,
    enbw_bins: f32,
}

impl LogBands {
    /// Bands with `per_octave` bands per octave between `f_min` and `f_max` (capped at Nyquist).
    pub fn new(sample_rate: f32, fft_size: usize, f_min: f32, f_max: f32, per_octave: usize) -> Self {
        let df = sample_rate / fft_size as f32;
        let nyquist_bin = fft_size / 2;
        let f_max = f_max.min(sample_rate / 2.0);
        let half = 2f32.powf(0.5 / per_octave as f32);
        let mut centers = Vec::new();
        let mut ranges = Vec::new();
        let mut k = 0;
        loop {
            let c = f_min * 2f32.powf(k as f32 / per_octave as f32);
            if c > f_max {
                break;
            }
            let lo = ((c / half) / df).ceil() as usize;
            let hi = (((c * half) / df).floor() as usize + 1).min(nyquist_bin + 1);
            let range = if lo < hi {
                (lo, hi)
            } else {
                let b = ((c / df).round() as usize).min(nyquist_bin);
                (b, b + 1)
            };
            centers.push(c);
            ranges.push(range);
            k += 1;
        }
        Self { centers, ranges, enbw_bins: 1.5 }
    }

    /// Set the analysis window's ENBW (defaults to Hann's 1.5 bins).
    pub fn with_enbw(mut self, enbw_bins: f32) -> Self {
        self.enbw_bins = enbw_bins.max(1e-3);
        self
    }

    /// Number of bands.
    pub fn len(&self) -> usize {
        self.centers.len()
    }

    /// True if there are no bands.
    pub fn is_empty(&self) -> bool {
        self.centers.is_empty()
    }

    /// Band centre frequencies in Hz.
    pub fn centers(&self) -> &[f32] {
        &self.centers
    }

    /// Band power (linear) from a power spectrum. A full-scale sine whose main lobe fits in a
    /// band reads ≈ 1.0 (0 dB). Real-time safe.
    pub fn compute(&self, power: &[f32], out: &mut [f32]) {
        for ((o, &(lo, hi)), _) in out.iter_mut().zip(&self.ranges).zip(&self.centers) {
            let hi = hi.min(power.len());
            let lo = lo.min(hi);
            *o = power[lo..hi].iter().sum::<f32>() / self.enbw_bins;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(f: f32, sr: f32, n: usize, a: f32) -> Vec<f32> {
        (0..n).map(|i| a * (2.0 * core::f32::consts::PI * f * i as f32 / sr).sin()).collect()
    }

    #[test]
    fn full_scale_sine_peaks_near_0db() {
        let sr = 48_000.0;
        let n = 1024;
        let f = 46.875 * 20.0; // exactly bin 20 (937.5 Hz)
        let mut a = StftAnalyzer::new(n, n / 2);
        let mut peak = f32::MIN;
        a.push(&sine(f, sr, 4096, 1.0), |p| {
            peak = peak.max(p.iter().copied().fold(f32::MIN, f32::max));
        });
        assert!(power_to_db(peak).abs() < 0.1, "peak {}", power_to_db(peak));
        assert!((a.enbw_bins() - 1.5).abs() < 1e-3);
    }

    #[test]
    fn frame_count_matches_hop() {
        let mut a = StftAnalyzer::new(256, 64);
        let mut frames = 0;
        a.push(&vec![0.0; 256 + 64 * 10], |_| frames += 1);
        assert_eq!(frames, 11);
    }

    #[test]
    fn band_reads_sine_level() {
        let sr = 48_000.0;
        let n = 4096;
        let bands = LogBands::new(sr, n, 20.0, 20_000.0, 3);
        let mut a = StftAnalyzer::new(n, n);
        let mut out = vec![0.0; bands.len()];
        a.push(&sine(1000.0, sr, n, 0.5), |p| bands.compute(p, &mut out));
        let max_db = out.iter().map(|&p| power_to_db(p)).fold(f32::MIN, f32::max);
        assert!((max_db - (-6.02)).abs() < 0.5, "{max_db}");
    }

    #[test]
    fn low_bands_use_nearest_bin_and_read_level() {
        let sr = 48_000.0;
        let n = 2048; // 23.4 Hz bins: every 1/12-octave band below ~300 Hz falls back to one bin
        let bands = LogBands::new(sr, n, 20.0, 20_000.0, 12);
        let mut a = StftAnalyzer::new(n, n / 2);
        let mut out = vec![0.0; bands.len()];
        a.push(&sine(117.1875, sr, 3 * n, 0.5), |p| bands.compute(p, &mut out)); // exactly bin 5
        let max_db = out.iter().map(|&p| power_to_db(p)).fold(f32::MIN, f32::max);
        assert!((max_db - (-6.02)).abs() < 2.0, "{max_db}");
    }

    #[test]
    fn bands_stop_at_nyquist() {
        let bands = LogBands::new(16_000.0, 1024, 20.0, 20_000.0, 3);
        assert!(*bands.centers().last().unwrap() <= 8_000.0);
        assert_eq!(bands.len(), 26); // 20 Hz .. 8 kHz in 1/3 octaves
    }

    #[test]
    fn enbw_is_clamped_positive() {
        let bands = LogBands::new(48_000.0, 1024, 20.0, 20_000.0, 3).with_enbw(0.0);
        let mut out = vec![0.0; bands.len()];
        bands.compute(&vec![1.0; 513], &mut out);
        assert!(out.iter().all(|x| x.is_finite()));
    }

    #[test]
    fn stft_is_chunking_invariant_and_resettable() {
        let x = sine(1234.0, 48_000.0, 10_000, 0.3);
        let collect = |a: &mut StftAnalyzer, chunks: &[usize]| {
            let mut frames: Vec<Vec<f32>> = Vec::new();
            let mut pos = 0;
            let mut k = 0;
            while pos < x.len() {
                let n = chunks[k % chunks.len()].min(x.len() - pos);
                a.push(&x[pos..pos + n], |p| frames.push(p.to_vec()));
                pos += n;
                k += 1;
            }
            frames
        };
        let mut a = StftAnalyzer::new(512, 128);
        let whole = collect(&mut a, &[usize::MAX]);
        let mut b = StftAnalyzer::new(512, 128);
        assert_eq!(whole, collect(&mut b, &[1, 7, 300, 33]));
        b.reset();
        assert_eq!(whole, collect(&mut b, &[64]));
    }

    #[test]
    fn twelfth_octave_covers_audio_range() {
        let bands = LogBands::new(48_000.0, 2048, 20.0, 20_000.0, 12);
        assert_eq!(bands.len(), 120);
        assert!(bands.centers().windows(2).all(|w| w[1] > w[0]));
    }
}
