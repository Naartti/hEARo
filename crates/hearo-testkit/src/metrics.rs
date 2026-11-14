//! Objective metrics used by golden tests and reports.

use realfft::RealFftPlanner;

/// Amplitude to dB (floored).
pub fn db(amp: f32) -> f32 {
    20.0 * amp.abs().max(1e-10).log10()
}

/// Root mean square.
pub fn rms(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>() / x.len() as f64).sqrt() as f32
}

/// RMS in dBFS.
pub fn rms_db(x: &[f32]) -> f32 {
    db(rms(x))
}

/// Absolute peak.
pub fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

/// Peak in dBFS.
pub fn peak_db(x: &[f32]) -> f32 {
    db(peak(x))
}

/// Total energy (sum of squares) in dB.
pub fn energy_db(x: &[f32]) -> f32 {
    let e: f64 = x.iter().map(|v| (*v as f64) * (*v as f64)).sum();
    (10.0 * e.max(1e-30).log10()) as f32
}

/// Energy between `lo` and `hi` Hz, in dB, from one FFT over the whole signal.
pub fn band_energy_db(x: &[f32], sr: f32, lo: f32, hi: f32) -> f32 {
    let bands = band_energies_db(x, sr, &[(lo, hi)]);
    bands[0]
}

/// Energies of several `(lo, hi)` bands in dB, from one FFT over the whole signal.
pub fn band_energies_db(x: &[f32], sr: f32, bands: &[(f32, f32)]) -> Vec<f32> {
    let n = x.len().max(2);
    let fft = RealFftPlanner::<f32>::new().plan_fft_forward(n);
    let mut input = vec![0.0; n];
    input[..x.len()].copy_from_slice(x);
    let mut spec = fft.make_output_vec();
    // Lengths come from the planner.
    let _ = fft.process(&mut input, &mut spec);
    let df = sr / n as f32;
    bands
        .iter()
        .map(|&(lo, hi)| {
            let e: f64 = spec
                .iter()
                .enumerate()
                .filter(|(k, _)| {
                    let f = *k as f32 * df;
                    f >= lo && f < hi
                })
                .map(|(_, c)| c.norm_sqr() as f64)
                .sum();
            (10.0 * e.max(1e-30).log10()) as f32
        })
        .collect()
}

/// Energy of everything *except* the given `(lo, hi)` bands, in dB (one FFT over the signal).
pub fn energy_excluding_db(x: &[f32], sr: f32, exclude: &[(f32, f32)]) -> f32 {
    let n = x.len().max(2);
    let fft = RealFftPlanner::<f32>::new().plan_fft_forward(n);
    let mut input = vec![0.0; n];
    input[..x.len()].copy_from_slice(x);
    let mut spec = fft.make_output_vec();
    let _ = fft.process(&mut input, &mut spec);
    let df = sr / n as f32;
    let e: f64 = spec
        .iter()
        .enumerate()
        .filter(|(k, _)| {
            let f = *k as f32 * df;
            !exclude.iter().any(|&(lo, hi)| f >= lo && f < hi)
        })
        .map(|(_, c)| c.norm_sqr() as f64)
        .sum();
    (10.0 * e.max(1e-30).log10()) as f32
}

/// Coherent amplitude of a sinusoid at each exact frequency (single-bin DFT over the whole
/// signal, 4-term Blackman–Harris window: −92 dB sidelobes, so nearby non-tonal energy
/// can't leak into the measurement). This measures *stationary tones* (e.g. hum)
/// and rejects signals whose energy moves around in frequency (e.g. speech harmonics).
pub fn tone_amplitudes(x: &[f32], sr: f32, freqs: &[f32]) -> Vec<f32> {
    let n = x.len();
    let w: Vec<f64> = (0..n)
        .map(|i| {
            let t = 2.0 * std::f64::consts::PI * i as f64 / n as f64;
            0.35875 - 0.48829 * t.cos() + 0.14128 * (2.0 * t).cos() - 0.01168 * (3.0 * t).cos()
        })
        .collect();
    let wsum: f64 = w.iter().sum();
    freqs
        .iter()
        .map(|&f| {
            let dw = 2.0 * std::f64::consts::PI * f as f64 / sr as f64;
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for (i, (&v, &wi)) in x.iter().zip(&w).enumerate() {
                let p = dw * i as f64;
                re += v as f64 * wi * p.cos();
                im -= v as f64 * wi * p.sin();
            }
            (2.0 * (re * re + im * im).sqrt() / wsum) as f32
        })
        .collect()
}

/// Largest absolute sample-to-sample difference (discontinuity detector).
pub fn max_jump(x: &[f32]) -> f32 {
    x.windows(2).fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()))
}

/// Largest absolute difference between two equal-length signals.
pub fn max_abs_diff(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).fold(0.0f32, |m, (x, y)| m.max((x - y).abs()))
}

/// Signal-to-noise ratio of `test` against `reference`, in dB.
pub fn snr_db(reference: &[f32], test: &[f32]) -> f32 {
    let s: f64 = reference.iter().map(|v| (*v as f64).powi(2)).sum();
    let n: f64 = reference.iter().zip(test).map(|(r, t)| ((*r - *t) as f64).powi(2)).sum();
    (10.0 * (s.max(1e-30) / n.max(1e-30)).log10()) as f32
}

/// True if every sample is finite.
pub fn all_finite(x: &[f32]) -> bool {
    x.iter().all(|v| v.is_finite())
}

/// Octave-band centres used in reports (31.5 Hz – 16 kHz).
pub const OCTAVE_CENTERS: [f32; 10] = [31.5, 63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];

/// Energy per octave band in dB.
pub fn octave_bands_db(x: &[f32], sr: f32) -> Vec<(f32, f32)> {
    let bands: Vec<(f32, f32)> =
        OCTAVE_CENTERS.iter().map(|c| (c / std::f32::consts::SQRT_2, c * std::f32::consts::SQRT_2)).collect();
    OCTAVE_CENTERS.iter().copied().zip(band_energies_db(x, sr, &bands)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signals::{SR, sine};

    #[test]
    fn sine_metrics() {
        let s = sine(SR, 1000.0, 1.0, 0.5);
        assert!((peak_db(&s) - db(0.5)).abs() < 0.01);
        assert!((rms_db(&s) - db(0.5 / 2f32.sqrt())).abs() < 0.01);
        let in_band = band_energy_db(&s, SR, 900.0, 1100.0);
        let out_band = band_energy_db(&s, SR, 2000.0, 4000.0);
        assert!(in_band - out_band > 60.0);
    }

    #[test]
    fn tone_amplitude_reads_a_sine() {
        let s = sine(SR, 50.0, 4.0, 0.25);
        let a = tone_amplitudes(&s, SR, &[50.0, 60.0]);
        assert!((a[0] - 0.25).abs() < 1e-3, "{a:?}");
        assert!(a[1] < 1e-4, "{a:?}");
    }

    #[test]
    fn snr_of_identical_is_huge() {
        let s = sine(SR, 440.0, 0.1, 0.5);
        assert!(snr_db(&s, &s) > 200.0);
        let noisy: Vec<f32> = s.iter().map(|v| v * 0.9).collect();
        assert!((snr_db(&s, &noisy) - 20.0).abs() < 0.01);
    }
}
