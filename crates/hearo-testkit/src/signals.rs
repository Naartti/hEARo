//! Deterministic test-signal generators. Every generator takes a seed or an [`Rng`], so the
//! same call always produces the same samples. All signals are mono `Vec<f32>`.

use std::f32::consts::PI;

use serde::Serialize;

use crate::Rng;
use crate::filters::Biquad;

/// Default sample rate for generated signals.
pub const SR: f32 = 48_000.0;

/// Where a synthetic event was injected (ground truth for detection tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Injected {
    /// First sample.
    pub start: usize,
    /// Length in samples.
    pub len: usize,
}

impl Injected {
    /// One past the last sample.
    pub fn end(&self) -> usize {
        self.start + self.len
    }
}

fn samples(sr: f32, secs: f32) -> usize {
    (sr * secs).round() as usize
}

/// Sine at `freq` Hz.
pub fn sine(sr: f32, freq: f32, secs: f32, amp: f32) -> Vec<f32> {
    (0..samples(sr, secs)).map(|i| amp * (2.0 * PI * freq * i as f32 / sr).sin()).collect()
}

/// Logarithmic (exponential) sine sweep from `f0` to `f1`.
pub fn log_sweep(sr: f32, f0: f32, f1: f32, secs: f32, amp: f32) -> Vec<f32> {
    let n = samples(sr, secs);
    let k = (f1 / f0).ln();
    let mut phase = 0.0f64;
    (0..n)
        .map(|i| {
            let f = f0 * (k * i as f32 / n as f32).exp();
            phase += 2.0 * std::f64::consts::PI * f as f64 / sr as f64;
            amp * (phase.sin() as f32)
        })
        .collect()
}

/// Gaussian white noise with standard deviation `amp`.
pub fn white(rng: &mut Rng, n: usize, amp: f32) -> Vec<f32> {
    (0..n).map(|_| amp * rng.gaussian()).collect()
}

/// Pink (1/f) noise via Paul Kellet's refined filter, roughly `amp` RMS.
pub fn pink(rng: &mut Rng, n: usize, amp: f32) -> Vec<f32> {
    let mut b = [0.0f32; 7];
    let mut out: Vec<f32> = (0..n)
        .map(|_| {
            let w = rng.gaussian();
            b[0] = 0.99886 * b[0] + w * 0.0555179;
            b[1] = 0.99332 * b[1] + w * 0.0750759;
            b[2] = 0.96900 * b[2] + w * 0.153_852;
            b[3] = 0.86650 * b[3] + w * 0.3104856;
            b[4] = 0.55000 * b[4] + w * 0.5329522;
            b[5] = -0.7616 * b[5] - w * 0.0168980;
            let y = b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[6] + w * 0.5362;
            b[6] = w * 0.115926;
            y
        })
        .collect();
    normalize_rms(&mut out, amp);
    out
}

/// Scale so the RMS equals `target` (no-op on silence).
pub fn normalize_rms(x: &mut [f32], target: f32) {
    let rms = (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt();
    if rms > 0.0 {
        let g = target / rms;
        x.iter_mut().for_each(|v| *v *= g);
    }
}

/// Add `src` into `dst` starting at `offset` (clipped to `dst`'s length).
pub fn add_at(dst: &mut [f32], src: &[f32], offset: usize) {
    for (d, s) in dst.iter_mut().skip(offset).zip(src) {
        *d += s;
    }
}

const VOWELS: [[f32; 3]; 5] = [
    [730.0, 1090.0, 2440.0], // a
    [530.0, 1840.0, 2480.0], // e
    [270.0, 2290.0, 3010.0], // i
    [570.0, 840.0, 2410.0],  // o
    [300.0, 870.0, 2240.0],  // u
];

/// Cutoff of each of the two glottal-tilt one-pole low-passes in [`speech_like`]. The value
/// is the one the original fixed coefficient (0.25) had at 48 kHz, so 48 kHz signals are
/// unchanged; at other rates the spectrum is now the same instead of brighter at 96 kHz.
pub const GLOTTAL_TILT_HZ: f32 = 2197.7;

/// One-pole coefficient for [`GLOTTAL_TILT_HZ`] at `sr`: `1 − e^(−2π·fc/sr)`. Computed as
/// `1 − 0.75^(48 kHz/sr)` (the same pole) so it is exactly 0.25 at 48 kHz.
fn glottal_tilt_coeff(sr: f32) -> f32 {
    1.0 - 0.75f32.powf(48_000.0 / sr)
}

/// A speech-like signal: a glottal pulse train with intonation, through three moving formant
/// resonators, shaped into syllables with pauses. RMS ≈ -20 dBFS over the whole signal.
///
/// It is not speech, but it has what matters for our detectors: pitched harmonics, formants,
/// syllabic amplitude modulation, silence gaps and impulsive glottal excitation.
pub fn speech_like(sr: f32, secs: f32, seed: u64) -> Vec<f32> {
    speech_like_with_gaps(sr, secs, seed).0
}

/// [`speech_like`] plus the pauses between syllables (ground truth for pause-aware tests).
pub fn speech_like_with_gaps(sr: f32, secs: f32, seed: u64) -> (Vec<f32>, Vec<Injected>) {
    let n = samples(sr, secs);
    let mut rng = Rng::new(seed);
    let mut out = vec![0.0f32; n];

    // Syllable plan: (start, len, vowel, gain).
    let mut plan = Vec::new();
    let mut t = 0usize;
    while t < n {
        let len = samples(sr, rng.range(0.12, 0.30));
        plan.push((t, len.min(n - t), rng.range_usize(0, VOWELS.len()), rng.range(0.6, 1.0)));
        t += len;
        if rng.unit() < 0.35 {
            t += samples(sr, rng.range(0.03, 0.15));
        }
    }
    let mut gaps = Vec::new();
    let mut prev_end = 0;
    for &(start, len, _, _) in &plan {
        if start > prev_end {
            gaps.push(Injected { start: prev_end, len: start - prev_end });
        }
        prev_end = start + len;
    }

    let mut phase = 0.0f32;
    let base_f0 = rng.range(100.0, 140.0);
    let mut tilt1 = 0.0f32;
    let mut tilt2 = 0.0f32;
    let a_tilt = glottal_tilt_coeff(sr);
    let mut formants = [Biquad::bandpass(sr, 500.0, 5.0); 3];
    let mut current = VOWELS[0];

    for &(start, len, vowel, gain) in &plan {
        let target = VOWELS[vowel];
        for i in 0..len {
            let idx = start + i;
            let time = idx as f32 / sr;
            // Glide formants towards the target over the first 40 ms, retune every 32 samples.
            if i % 32 == 0 {
                let k = (i as f32 / (0.04 * sr)).min(1.0);
                for (j, f) in formants.iter_mut().enumerate() {
                    let freq = current[j] + (target[j] - current[j]) * k;
                    f.retune_bandpass(sr, freq, freq / 90.0);
                }
            }
            let f0 = base_f0 * (1.0 + 0.15 * (2.0 * PI * 0.3 * time).sin()) + 2.0 * rng.gaussian();
            phase += f0 / sr;
            let pulse = if phase >= 1.0 {
                phase -= 1.0;
                1.0
            } else {
                0.0
            };
            // Glottal spectral tilt: two one-pole low-passes.
            tilt1 += a_tilt * (pulse - tilt1);
            tilt2 += a_tilt * (tilt1 - tilt2);
            let src = tilt2 + 0.002 * rng.gaussian();
            let voiced: f32 = formants.iter_mut().enumerate().map(|(j, f)| f.tick(src) / (j + 1) as f32).sum();
            // Raised-cosine syllable envelope.
            let env = (PI * i as f32 / len as f32).sin().powf(0.6) * gain;
            out[idx] = voiced * env;
        }
        current = target;
    }
    normalize_rms(&mut out, 0.1);
    (out, gaps)
}

/// Speech-like signal with real pauses over a room-tone floor, and lip smacks injected into
/// pauses: `count` smacks with peaks `level_db` (range) dBFS, 5–20 ms long (calibrated to the
/// human's captured smacks, which measured 5–15 ms), starting 15–30 ms into a pause — "a short
/// pause, a smack, then speech continues". Returns (signal, smacks).
pub fn speech_with_pause_smacks(
    sr: f32,
    secs: f32,
    seed: u64,
    count: usize,
    level_db: (f32, f32),
    floor_db: f32,
) -> (Vec<f32>, Vec<Injected>) {
    let (mut s, gaps) = speech_like_with_gaps(sr, secs, seed);
    let mut rng = Rng::new(seed ^ 0x5AC);
    let floor = pink(&mut rng, s.len(), 10f32.powf(floor_db / 20.0));
    add_at(&mut s, &floor, 0);
    // Room for the smack (≤ 40 ms) plus some silence after it.
    let usable: Vec<Injected> =
        gaps.into_iter().filter(|g| g.len >= samples(sr, 0.075) && g.start > samples(sr, 0.5)).collect();
    let step = (usable.len() / count.max(1)).max(1);
    let mut smacks = Vec::new();
    for g in usable.iter().step_by(step).take(count) {
        let amp = 10f32.powf(rng.range(level_db.0, level_db.1) / 20.0);
        let len = rng.range(0.005, 0.020);
        let sm = smack_len(sr, &mut rng, amp, len);
        let start = g.start + samples(sr, rng.range(0.015, 0.03));
        let len = sm.len().min(g.end().saturating_sub(start));
        add_at(&mut s, &sm[..len], start);
        smacks.push(Injected { start, len });
    }
    (s, smacks)
}

/// Mains hum: `f0` plus `harmonics` overtones (amplitude falling as 1/k), peak ≈ `amp`.
pub fn hum(sr: f32, f0: f32, harmonics: usize, secs: f32, amp: f32) -> Vec<f32> {
    let norm: f32 = (1..=harmonics + 1).map(|k| 1.0 / k as f32).sum();
    (0..samples(sr, secs))
        .map(|i| {
            let t = i as f32 / sr;
            (1..=harmonics + 1).map(|k| (2.0 * PI * f0 * k as f32 * t).sin() / k as f32).sum::<f32>() * amp / norm
        })
        .collect()
}

/// A short broadband mouth click: a sharp asymmetric spike with a fast-decaying, high-passed
/// noise tail. `dur_ms` in 0.2–5 ms.
pub fn click(sr: f32, rng: &mut Rng, dur_ms: f32, amp: f32) -> Vec<f32> {
    let n = samples(sr, dur_ms / 1000.0).max(2);
    let mut hp = Biquad::highpass(sr, 2_500.0, 0.707);
    let tau = n as f32 / 4.0;
    let sign = if rng.unit() < 0.5 { 1.0 } else { -1.0 };
    let mut out: Vec<f32> = (0..n)
        .map(|i| {
            let spike = if i == 0 {
                sign
            } else if i == 1 {
                -0.6 * sign
            } else {
                0.0
            };
            spike + 0.5 * rng.gaussian() * (-(i as f32) / tau).exp()
        })
        .collect();
    hp.run(&mut out);
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-9);
    out.iter_mut().for_each(|v| *v *= amp / peak);
    out
}

/// Inject `count` clicks at random, non-overlapping positions separated by at least
/// `min_gap_s`. Returns the ground-truth positions (sorted).
pub fn inject_clicks(
    signal: &mut [f32],
    sr: f32,
    rng: &mut Rng,
    count: usize,
    amp: (f32, f32),
    dur_ms: (f32, f32),
    min_gap_s: f32,
) -> Vec<Injected> {
    let gap = samples(sr, min_gap_s);
    let margin = samples(sr, 0.05);
    let span = signal.len().saturating_sub(2 * margin);
    let slot = span / count.max(1);
    assert!(slot > gap, "too many clicks for this signal length");
    (0..count)
        .map(|k| {
            let dur = rng.range(dur_ms.0, dur_ms.1);
            let a = rng.range(amp.0, amp.1);
            let c = click(sr, rng, dur, a);
            let start = margin + k * slot + rng.range_usize(0, slot - gap);
            add_at(signal, &c, start);
            Injected { start, len: c.len() }
        })
        .collect()
}

/// A lip smack: a 10–40 ms damped noise burst with a low-frequency thump.
pub fn smack(sr: f32, rng: &mut Rng, amp: f32) -> Vec<f32> {
    let len = rng.range(0.010, 0.040);
    smack_len(sr, rng, amp, len)
}

/// A lip smack of `secs` length (damped band-passed noise + low thump), peak ≈ `amp`.
pub fn smack_len(sr: f32, rng: &mut Rng, amp: f32, secs: f32) -> Vec<f32> {
    let n = samples(sr, secs);
    let mut bp = Biquad::bandpass(sr, rng.range(1_500.0, 4_000.0), 1.0);
    let thump_f = rng.range(60.0, 120.0);
    let tau = n as f32 / 5.0;
    let mut out: Vec<f32> = (0..n)
        .map(|i| {
            let e = (-(i as f32) / tau).exp();
            bp.tick(rng.gaussian()) * e + 0.5 * (2.0 * PI * thump_f * i as f32 / sr).sin() * e
        })
        .collect();
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-9);
    out.iter_mut().for_each(|v| *v *= amp / peak);
    out
}

/// A long, multi-tick lip smack with a room tail, modelled on a human-labelled miss
/// (2026-11-14): 3–4 ticks spread over 20–30 ms, then a band-passed tail (−12 dB, decaying
/// ≈ 1 dB/ms) for 50 ms. Returns (signal, length of the ticks part in samples).
pub fn multi_tick_smack(sr: f32, rng: &mut Rng, amp: f32) -> (Vec<f32>, usize) {
    let span = samples(sr, rng.range(0.020, 0.030));
    let ticks = rng.range_usize(3, 5);
    let tail_len = samples(sr, 0.050);
    let mut out = vec![0.0f32; span + samples(sr, 0.008) + tail_len];
    let mut ticks_end = 0;
    for k in 0..ticks {
        let at = span * k / (ticks - 1);
        let level = if k == ticks / 2 { 1.0 } else { rng.range(0.4, 0.8) };
        let tick_len = rng.range(0.004, 0.008);
        let t = smack_len(sr, rng, amp * level, tick_len);
        ticks_end = ticks_end.max(at + t.len());
        add_at(&mut out, &t, at);
    }
    let mut bp = Biquad::bandpass(sr, rng.range(1_500.0, 3_000.0), 0.7);
    let tau = sr * 0.0087; // amplitude τ 8.7 ms ≈ 1 dB/ms (measured on the labelled miss)
    let tail: Vec<f32> = (0..tail_len).map(|i| bp.tick(rng.gaussian()) * (-(i as f32) / tau).exp()).collect();
    let peak = tail.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-9);
    let tail: Vec<f32> = tail.iter().map(|v| v * 0.25 * amp / peak).collect();
    add_at(&mut out, &tail, ticks_end.saturating_sub(samples(sr, 0.002)));
    (out, ticks_end)
}

/// Phrases of [`speech_like`] separated by 300 ms pauses, a [`multi_tick_smack`] 40–150 ms into
/// each pause. Returns (signal, ticks of each smack as ground truth).
pub fn speech_with_long_smacks(
    sr: f32,
    secs: f32,
    seed: u64,
    level_db: (f32, f32),
    floor_db: f32,
) -> (Vec<f32>, Vec<Injected>) {
    let mut rng = Rng::new(seed ^ 0x7111);
    let phrase = samples(sr, 0.7);
    let pause = samples(sr, 0.3);
    let n = samples(sr, secs);
    let speech = speech_like(sr, secs, seed);
    let mut s = vec![0.0f32; n];
    let mut smacks = Vec::new();
    let mut t = 0;
    while t + phrase + pause <= n {
        s[t..t + phrase].copy_from_slice(&speech[t..t + phrase]);
        // Short fade so the phrase end isn't itself a click.
        let fade = samples(sr, 0.01);
        for i in 0..fade {
            s[t + phrase - 1 - i] *= i as f32 / fade as f32;
        }
        if t > 0 {
            let amp = 10f32.powf(rng.range(level_db.0, level_db.1) / 20.0);
            let (sm, ticks) = multi_tick_smack(sr, &mut rng, amp);
            let at = t + phrase + samples(sr, rng.range(0.04, 0.15));
            add_at(&mut s, &sm, at);
            smacks.push(Injected { start: at, len: ticks });
        }
        t += phrase + pause;
    }
    let floor = pink(&mut rng, n, 10f32.powf(floor_db / 20.0));
    add_at(&mut s, &floor, 0);
    (s, smacks)
}

/// A synthetic "s": band-limited noise (6–8 kHz) with a raised-cosine envelope, peak ≈ `amp`.
pub fn sibilance(sr: f32, rng: &mut Rng, secs: f32, amp: f32) -> Vec<f32> {
    let n = samples(sr, secs);
    let mut hp = [Biquad::highpass(sr, 6_000.0, 0.707), Biquad::highpass(sr, 6_000.0, 0.707)];
    let mut lp = [Biquad::lowpass(sr, 8_000.0, 0.707), Biquad::lowpass(sr, 8_000.0, 0.707)];
    let mut out: Vec<f32> = (0..n)
        .map(|i| {
            let mut x = rng.gaussian();
            for f in hp.iter_mut().chain(lp.iter_mut()) {
                x = f.tick(x);
            }
            x * (PI * i as f32 / n as f32).sin().powi(2)
        })
        .collect();
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-9);
    out.iter_mut().for_each(|v| *v *= amp / peak);
    out
}

/// A low-frequency plosive pop: one decaying cycle of 30–80 Hz plus low-passed noise, all
/// below ~150 Hz, peak ≈ `amp`.
pub fn plosive(sr: f32, rng: &mut Rng, amp: f32) -> Vec<f32> {
    let n = samples(sr, rng.range(0.04, 0.08));
    let f = rng.range(30.0, 80.0);
    let mut lp = [Biquad::lowpass(sr, 150.0, 0.707), Biquad::lowpass(sr, 150.0, 0.707)];
    let mut out: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let env = (PI * t).sin() * (1.0 - t);
            let mut x = (2.0 * PI * f * i as f32 / sr).sin() * env + 0.3 * rng.gaussian() * env;
            for l in &mut lp {
                x = l.tick(x);
            }
            x
        })
        .collect();
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-9);
    out.iter_mut().for_each(|v| *v *= amp / peak);
    out
}

/// A plucked string (Karplus–Strong).
pub fn pluck(sr: f32, rng: &mut Rng, freq: f32, secs: f32, amp: f32) -> Vec<f32> {
    let period = (sr / freq).round().max(2.0) as usize;
    let mut line: Vec<f32> = (0..period).map(|_| rng.range(-1.0, 1.0)).collect();
    let mut out = Vec::with_capacity(samples(sr, secs));
    let mut p = 0;
    for _ in 0..samples(sr, secs) {
        let next = (p + 1) % period;
        let y = 0.996 * 0.5 * (line[p] + line[next]);
        out.push(line[p] * amp);
        line[p] = y;
        p = next;
    }
    out
}

/// A drum-like hit: a pitch-dropping sine "kick" plus a noise "snap".
pub fn drum_hit(sr: f32, rng: &mut Rng, amp: f32) -> Vec<f32> {
    let n = samples(sr, 0.25);
    let mut phase = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / sr;
            let f = 50.0 + 100.0 * (-t * 30.0).exp();
            phase += 2.0 * PI * f / sr;
            amp * (phase.sin() * (-t * 12.0).exp() + 0.4 * rng.gaussian() * (-t * 60.0).exp())
        })
        .collect()
}

/// Music-like test material: plucked notes on a pentatonic scale over a drum pattern.
/// Used to check that detectors do not fire on musical transients.
pub fn music_like(sr: f32, secs: f32, seed: u64) -> Vec<f32> {
    let mut rng = Rng::new(seed);
    let n = samples(sr, secs);
    let mut out = vec![0.0f32; n];
    let beat = samples(sr, 0.5);
    let scale = [196.0, 220.0, 246.9, 293.7, 329.6, 392.0];
    let mut t = 0;
    while t < n {
        let d = drum_hit(sr, &mut rng, 0.3);
        add_at(&mut out, &d, t);
        let f = scale[rng.range_usize(0, scale.len())];
        let p = pluck(sr, &mut rng, f, 0.9, 0.25);
        add_at(&mut out, &p, t + beat / 2);
        t += beat;
    }
    out
}

/// Stop consonants after a closure — the realistic false-positive risk for smack detection: a
/// 30–120 ms closure (room tone), a 2–15 ms noise burst (/p/ low, /k/ mid, /t/ high band),
/// then either aspiration (voiceless, 30–80 ms) or voicing 5–20 ms later, then a vowel.
/// Returns (signal, bursts). Burst peak is −6…+3 dB relative to the following vowel's peak.
pub fn speech_with_stop_bursts(sr: f32, secs: f32, seed: u64, floor_db: f32) -> (Vec<f32>, Vec<Injected>) {
    let n = samples(sr, secs);
    let mut rng = Rng::new(seed);
    let mut out = pink(&mut rng, n, 10f32.powf(floor_db / 20.0));
    let mut bursts = Vec::new();
    let mut t = samples(sr, 0.5);
    while t + samples(sr, 0.6) < n {
        t += samples(sr, rng.range(0.03, 0.12)); // closure: room tone only
        let vowel = speech_like(sr, rng.range(0.15, 0.3), rng.next_u64());
        let vpeak = vowel.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-6);
        let blen = samples(sr, rng.range(0.002, 0.015)).max(2);
        let tau = rng.range(0.0015, 0.004) * sr;
        let kind = rng.range_usize(0, 3);
        let mut f: Vec<Biquad> = match kind {
            0 => vec![Biquad::lowpass(sr, 1500.0, 0.707)],
            1 => vec![Biquad::bandpass(sr, 2200.0, 2.0)],
            _ => vec![Biquad::highpass(sr, 3500.0, 0.707)],
        };
        let mut burst: Vec<f32> = (0..blen)
            .map(|i| {
                let mut x = rng.gaussian() * (-(i as f32) / tau).exp();
                for b in &mut f {
                    x = b.tick(x);
                }
                x
            })
            .collect();
        let bpeak = burst.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-9);
        let gain = vpeak * 10f32.powf(rng.range(-6.0, 3.0) / 20.0) / bpeak;
        burst.iter_mut().for_each(|v| *v *= gain);
        add_at(&mut out, &burst, t);
        bursts.push(Injected { start: t, len: blen });
        t += blen;
        if rng.unit() < 0.5 {
            // Voiceless: aspiration (high-passed noise) for the voice-onset time.
            let vot = samples(sr, rng.range(0.03, 0.08));
            let mut hp = Biquad::highpass(sr, 1000.0, 0.707);
            let level = vpeak * 10f32.powf(rng.range(-25.0, -15.0) / 20.0);
            let asp: Vec<f32> = (0..vot).map(|_| hp.tick(rng.gaussian()) * level).collect();
            add_at(&mut out, &asp, t);
            t += vot;
        } else {
            t += samples(sr, rng.range(0.005, 0.02));
        }
        add_at(&mut out, &vowel, t);
        t += vowel.len();
    }
    (out, bursts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn generators_are_deterministic() {
        assert_eq!(speech_like(SR, 1.0, 7), speech_like(SR, 1.0, 7));
        assert_ne!(speech_like(SR, 1.0, 7), speech_like(SR, 1.0, 8));
        assert_eq!(music_like(SR, 1.0, 3), music_like(SR, 1.0, 3));
    }

    #[test]
    fn pause_smacks_land_in_pauses() {
        let (s, smacks) = speech_with_pause_smacks(SR, 20.0, 3, 12, (-20.0, -12.0), -55.0);
        assert!(smacks.len() >= 8, "only {} smacks fit", smacks.len());
        assert!(s.iter().all(|x| x.is_finite()));
        let (_, gaps) = speech_like_with_gaps(SR, 20.0, 3);
        for sm in &smacks {
            assert!(gaps.iter().any(|g| g.start <= sm.start && sm.end() <= g.end()), "smack outside a pause");
        }
    }

    #[test]
    fn glottal_tilt_is_sample_rate_independent() {
        assert_eq!(glottal_tilt_coeff(48_000.0), 0.25, "48 kHz signals must not change");
        for sr in [44_100.0f32, 48_000.0, 96_000.0] {
            let from_hz = 1.0 - (-2.0 * PI * GLOTTAL_TILT_HZ / sr).exp();
            assert!((glottal_tilt_coeff(sr) - from_hz).abs() < 1e-4, "sr {sr}");
        }
    }

    #[test]
    fn speech_like_is_sane() {
        let s = speech_like(SR, 3.0, 1);
        assert_eq!(s.len(), 144_000);
        assert!((rms(&s) - 0.1).abs() < 1e-3);
        assert!(s.iter().all(|x| x.is_finite() && x.abs() < 1.0));
    }

    #[test]
    fn clicks_are_where_we_say() {
        let mut s = vec![0.0; 48_000];
        let mut rng = Rng::new(1);
        let inj = inject_clicks(&mut s, SR, &mut rng, 10, (0.2, 0.5), (0.2, 5.0), 0.02);
        assert_eq!(inj.len(), 10);
        assert!(inj.windows(2).all(|w| w[0].end() < w[1].start));
        for i in &inj {
            assert!(s[i.start..i.end()].iter().any(|x| x.abs() > 0.1));
            assert!(i.len >= 9 && i.len <= 240);
        }
    }

    #[test]
    fn bursts_hit_their_peak() {
        let mut rng = Rng::new(5);
        for sig in [sibilance(SR, &mut rng, 0.12, 0.5), plosive(SR, &mut rng, 0.5), smack(SR, &mut rng, 0.5)] {
            let peak = sig.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!((peak - 0.5).abs() < 1e-4);
        }
    }
}
