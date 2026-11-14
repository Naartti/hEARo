#![allow(clippy::print_stderr)] // margins are printed for the PROGRESS log

//! Golden tests for the de-esser (HANDOVER §7.2). Measurable outcomes, not exact samples.
//!
//! The de-esser is linked across channels and, for a given gain curve, linear. So a stereo pair
//! `L = tone + s`, `R = s − tone` is processed with one gain curve, and `(L' + R')/2`, `(L' − R')/2`
//! are exactly what happened to the "s" and to the voiced tone. The linked detector (mean power
//! of L and R) hears `tone² + s²`, the same as a mono mix: neither part is diluted.

use hearo_dsp::modules::deess::DeEss;
use hearo_dsp::{AudioBlock, Chain, Event, Module, ModuleId};
use hearo_testkit::Rng;
use hearo_testkit::filters::Biquad;
use hearo_testkit::metrics::{band_energy_db, energy_db, max_abs_diff, max_jump};
use hearo_testkit::signals::{add_at, music_like, normalize_rms, sibilance, sine, speech_like};

fn chain(sr: f32, block: usize, channels: usize) -> Chain {
    let mut chain = Chain::new(vec![Box::new(DeEss::new()) as Box<dyn Module>]);
    chain.prepare(sr, block.min(4096), channels);
    assert_eq!(chain.latency_samples(), 0, "de-ess must not add latency (budget)");
    chain
}

/// Run `chans` (equal length) through the de-esser in `block`-sized pieces.
fn process(chans: &[&[f32]], sr: f32, block: usize) -> (Vec<Vec<f32>>, Vec<Event>, Chain) {
    let ch = chans.len();
    let n = chans[0].len();
    let mut chain = chain(sr, block, ch);
    let mut out: Vec<Vec<f32>> = vec![Vec::with_capacity(n); ch];
    let mut events = Vec::new();
    let mut buf = vec![0.0f32; ch * block];
    let mut pos = 0;
    while pos < n {
        let k = block.min(n - pos);
        for (c, x) in chans.iter().enumerate() {
            buf[c * k..(c + 1) * k].copy_from_slice(&x[pos..pos + k]);
        }
        chain.process(&mut AudioBlock::from_planar(&mut buf[..ch * k], ch));
        for (c, o) in out.iter_mut().enumerate() {
            o.extend_from_slice(&buf[c * k..(c + 1) * k]);
        }
        events.extend_from_slice(chain.telemetry().events());
        chain.telemetry_mut().clear_events();
        pos += k;
    }
    (out, events, chain)
}

fn ms(sr: f32, m: f32) -> usize {
    (sr * m / 1000.0) as usize
}

/// A sustained, *rich* vowel-like tone: 150 Hz with every harmonic up to 10 kHz at −6 dB/oct
/// (1/k), −20 dBFS RMS. Its own 5–9 kHz content sits just below the default threshold, so the
/// "tone unchanged" check means something (a tone with harmonics only up to 3 kHz has nothing in
/// the de-esser's band and passes trivially).
fn voiced_tone(sr: f32, secs: f32) -> Vec<f32> {
    let mut x = vec![0.0f32; (sr * secs).round() as usize];
    let top = (10_000.0 / 150.0) as usize;
    for k in 1..=top {
        add_at(&mut x, &sine(sr, 150.0 * k as f32, secs, 1.0 / k as f32), 0);
    }
    normalize_rms(&mut x, 0.1);
    x
}

/// Band-limited noise like testkit's `sibilance` (2× high-pass 6 kHz, 2× low-pass 8 kHz) under
/// an arbitrary envelope `env(t)` (t in seconds), scaled to sample peak `peak`.
fn shaped_band_noise(sr: f32, rng: &mut Rng, secs: f32, peak: f32, env: impl Fn(f32) -> f32) -> Vec<f32> {
    let n = (sr * secs).round() as usize;
    let mut f = [
        Biquad::highpass(sr, 6_000.0, 0.707),
        Biquad::highpass(sr, 6_000.0, 0.707),
        Biquad::lowpass(sr, 8_000.0, 0.707),
        Biquad::lowpass(sr, 8_000.0, 0.707),
    ];
    // Let the filters settle on noise first so the envelope alone shapes the onset.
    let settle = (sr * 0.01) as usize;
    let mut out: Vec<f32> = (0..settle + n)
        .map(|_| f.iter_mut().fold(rng.gaussian(), |x, b| b.tick(x)))
        .skip(settle)
        .enumerate()
        .map(|(i, x)| x * env(i as f32 / sr))
        .collect();
    let p = out.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-9);
    out.iter_mut().for_each(|v| *v *= peak / p);
    out
}

/// Raised-cosine ramp 0 → 1 over `len` seconds.
fn ramp(t: f32, len: f32) -> f32 {
    0.5 - 0.5 * (std::f32::consts::PI * (t / len).clamp(0.0, 1.0)).cos()
}

/// A fast-onset "s": 10 ms raised-cosine rise, 40 ms hold, 10 ms fall (60 ms).
fn fast_s(sr: f32, rng: &mut Rng, peak: f32) -> Vec<f32> {
    shaped_band_noise(sr, rng, 0.06, peak, |t| ramp(t, 0.01).min(ramp(0.06 - t, 0.01)))
}

/// A bright drum snap in the "s" band: 6–8 kHz noise, 1 ms rise, 5 ms exponential decay.
fn snap(sr: f32, rng: &mut Rng, peak: f32) -> Vec<f32> {
    shaped_band_noise(sr, rng, 0.041, peak, |t| if t < 0.001 { ramp(t, 0.001) } else { (-(t - 0.001) / 0.005).exp() })
}

/// Process `tone` and `s` (equal length) as the stereo pair (tone + s, s − tone) and return
/// (processed tone, processed s, events, chain); see the file comment.
fn split_process(tone: &[f32], s: &[f32], sr: f32) -> (Vec<f32>, Vec<f32>, Vec<Event>, Chain) {
    let l: Vec<f32> = tone.iter().zip(s).map(|(t, s)| s + t).collect();
    let r: Vec<f32> = tone.iter().zip(s).map(|(t, s)| s - t).collect();
    let (out, events, chain) = process(&[&l, &r], sr, 512);
    let tone_out = out[0].iter().zip(&out[1]).map(|(l, r)| 0.5 * (l - r)).collect();
    let s_out = out[0].iter().zip(&out[1]).map(|(l, r)| 0.5 * (l + r)).collect();
    (tone_out, s_out, events, chain)
}

/// Peak of a 2 ms sliding RMS, in dB (a sample peak of noise is itself noisy).
fn peak_rms_db(x: &[f32], sr: f32) -> f32 {
    let w = ms(sr, 2.0);
    let mut acc = 0.0f64;
    let mut best = 0.0f64;
    for i in 0..x.len() {
        acc += (x[i] as f64).powi(2);
        if i >= w {
            acc -= (x[i - w] as f64).powi(2);
        }
        best = best.max(acc / w as f64);
    }
    (10.0 * best.max(1e-30).log10()) as f32
}

fn sample_peak_db(x: &[f32]) -> f32 {
    20.0 * x.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-15).log10()
}

const BURST_PEAK: f32 = 0.25; // −12 dBFS: a loud "s" over a −20 dBFS RMS vowel

/// An "s" (`burst`) mixed into a rich voiced tone at 600 ms → burst peak −6 dB or more (2 ms RMS
/// and sample peak), voiced tone changed < 0.3 dB, exactly one event that starts inside the
/// burst and ends with it (event length follows the target gain, not the release tail).
fn check_s_over_tone(label: &str, sr: f32, burst: &[f32], min_peak_red: f32) {
    let secs = 1.5;
    let tone = voiced_tone(sr, secs);
    let at = ms(sr, 600.0);
    let mut burst_track = vec![0.0f32; tone.len()];
    add_at(&mut burst_track, burst, at);
    let (tone_out, processed_burst, events, chain) = split_process(&tone, &burst_track, sr);

    let win = (at, at + burst.len());
    let rms_red = peak_rms_db(&burst_track[win.0..win.1], sr) - peak_rms_db(&processed_burst[win.0..win.1], sr);
    let peak_red = sample_peak_db(&burst_track) - sample_peak_db(&processed_burst);
    let energy_red = energy_db(&burst_track[win.0..win.1]) - energy_db(&processed_burst[win.0..win.1]);

    // The voiced tone, measured where the de-esser works hardest and over the whole signal.
    let pad = ms(sr, 100.0);
    let (a, b) = (win.0 - pad, win.1 + pad);
    let tone_burst = (energy_db(&tone_out[a..b]) - energy_db(&tone[a..b])).abs();
    let tone_all = (energy_db(&tone_out) - energy_db(&tone)).abs();
    // Away from the burst the tone (whose top harmonics sit just under the threshold) is left alone.
    let quiet = ..win.0 - pad;
    let tone_hf_quiet =
        band_energy_db(&tone[quiet], sr, 5000.0, 9000.0) - band_energy_db(&tone_out[quiet], sr, 5000.0, 9000.0);

    let ev: Vec<&Event> = events.iter().filter(|e| e.module == ModuleId::DeEss).collect();
    let burst_ms = burst.len() as f32 / sr * 1000.0;
    let dur_ms = ev.first().map_or(0.0, |e| e.duration_samples as f32 / sr * 1000.0);
    eprintln!(
        "MARGIN {label} sr {sr}: burst peak −{rms_red:.2} dB (2 ms RMS) / −{peak_red:.2} dB (sample), energy \
         −{energy_red:.2} dB, voiced tone \
         {tone_burst:.3} dB during burst / {tone_all:.3} dB overall / {tone_hf_quiet:.3} dB 5–9 kHz away from it, \
         {} event(s), {dur_ms:.1} ms (burst {burst_ms:.0}) −{:.1} dB",
        ev.len(),
        ev.first().map_or(0.0, |e| e.energy_removed_db)
    );

    assert!(rms_red >= min_peak_red, "{label} sr {sr}: burst peak (2 ms RMS) reduced only {rms_red:.2} dB");
    assert!(peak_red >= min_peak_red, "{label} sr {sr}: burst sample peak reduced only {peak_red:.2} dB");
    assert!(energy_red >= 6.0, "{label} sr {sr}: burst energy reduced only {energy_red:.2} dB");
    assert!(tone_burst < 0.3, "{label} sr {sr}: voiced tone changed {tone_burst:.3} dB during the burst");
    assert!(tone_all < 0.3, "{label} sr {sr}: voiced tone changed {tone_all:.3} dB overall");
    assert!(tone_hf_quiet.abs() < 0.1, "{label} sr {sr}: tone's 5–9 kHz band changed {tone_hf_quiet:.3} dB");
    assert_eq!(ev.len(), 1, "{label} sr {sr}: expected one sibilance event, got {ev:?}");
    let e = ev[0];
    assert!(
        (e.start_sample as usize) >= at && (e.start_sample as usize) < at + burst.len() / 2,
        "{label} sr {sr}: event starts at {} (burst {}..{})",
        e.start_sample,
        win.0,
        win.1
    );
    assert!(dur_ms <= burst_ms, "{label} sr {sr}: event lasts {dur_ms:.1} ms, burst {burst_ms:.0} ms");
    assert!(e.energy_removed_db > 3.0, "{label} sr {sr}: event reports {:.1} dB removed", e.energy_removed_db);
    assert_eq!(chain.telemetry().meters(ModuleId::DeEss).triggers, 1);
}

/// §7.2: 120 ms "s" burst (6–8 kHz noise, raised-cosine envelope) over a voiced tone.
fn check_burst_over_tone(sr: f32, seed: u64) {
    let burst = sibilance(sr, &mut Rng::new(seed), 0.12, BURST_PEAK);
    check_s_over_tone("120 ms s", sr, &burst, 6.0);
}

/// The hard case for a zero-lookahead de-esser that must ignore drum snaps: an "s" that
/// reaches full level in 10 ms. The onset gate arms ≈ 5–7 ms after the band crosses the knee
/// (it has to see the band *keep rising* after a snap would have peaked), so the first,
/// almost-full-level milliseconds pass: the burst's energy is cut ≥ 6 dB, its very first peak
/// only ≥ 3 dB (median ≈ 6.8 dB over 20 seeds, worst 4.4 dB 2 ms RMS / 3.5 dB sample peak).
fn check_fast_s_over_tone(sr: f32, seed: u64) {
    let burst = fast_s(sr, &mut Rng::new(seed), BURST_PEAK);
    check_s_over_tone("fast s", sr, &burst, 3.0);
}

#[test]
fn fast_onset_s_at_48k() {
    check_fast_s_over_tone(48_000.0, 31);
}

#[test]
fn fast_onset_s_at_44k1() {
    check_fast_s_over_tone(44_100.0, 32);
}

#[test]
fn fast_onset_s_at_96k() {
    check_fast_s_over_tone(96_000.0, 33);
}

#[test]
fn burst_over_voiced_tone_at_48k() {
    check_burst_over_tone(48_000.0, 1);
}

#[test]
fn burst_over_voiced_tone_at_44k1() {
    check_burst_over_tone(44_100.0, 2);
}

#[test]
fn burst_over_voiced_tone_at_96k() {
    check_burst_over_tone(96_000.0, 3);
}

/// No sibilance → (almost) untouched: overall level, the 5–9 kHz band itself, and the number of
/// sibilance events per minute.
fn check_clean(name: &str, x: &[f32], sr: f32, max_change_db: f32, max_events_per_min: f32) {
    let (out, events, _) = process(&[x], sr, 512);
    let change = (energy_db(&out[0]) - energy_db(x)).abs();
    let hf = band_energy_db(x, sr, 5000.0, 9000.0) - band_energy_db(&out[0], sr, 5000.0, 9000.0);
    let per_min = events.len() as f32 / (x.len() as f32 / sr / 60.0);
    eprintln!(
        "MARGIN {name} sr {sr}: overall change {change:.3} dB, 5–9 kHz band −{hf:.3} dB, {} events ({per_min:.1}/min)",
        events.len()
    );
    assert!(change < max_change_db, "{name} sr {sr}: changed {change:.3} dB");
    assert!(hf.abs() < 0.5, "{name} sr {sr}: 5–9 kHz band changed {hf:.3} dB");
    assert!(per_min <= max_events_per_min, "{name} sr {sr}: {per_min:.1} events/min");
}

#[test]
fn clean_speech_is_untouched() {
    for (sr, seed) in [(48_000.0, 11), (44_100.0, 12), (96_000.0, 13)] {
        check_clean("speech_like", &speech_like(sr, 60.0, seed), sr, 0.5, 1.0);
    }
}

#[test]
fn music_is_untouched() {
    for (sr, seed) in [(48_000.0, 21), (44_100.0, 22), (96_000.0, 23)] {
        check_clean("music_like", &music_like(sr, 60.0, seed), sr, 1.0, 2.0);
    }
}

/// Bright snaps right in the "s" band (6–8 kHz, 1 ms rise, 5 ms decay, −6 dBFS peak) every
/// 250 ms → no sibilance events at all.
#[test]
fn snap_train_is_ignored() {
    for (sr, seed) in [(48_000.0, 41), (44_100.0, 42), (96_000.0, 43)] {
        let mut rng = Rng::new(seed);
        let mut x = vec![0.0f32; (sr * 30.0) as usize];
        let mut t = ms(sr, 100.0);
        while t + ms(sr, 250.0) < x.len() {
            add_at(&mut x, &snap(sr, &mut rng, 0.5), t);
            t += ms(sr, 250.0);
        }
        let (out, events, _) = process(&[&x], sr, 512);
        let hf = band_energy_db(&x, sr, 5000.0, 9000.0) - band_energy_db(&out[0], sr, 5000.0, 9000.0);
        eprintln!("MARGIN snap train sr {sr}: {} events, 5–9 kHz band −{hf:.3} dB", events.len());
        assert!(events.is_empty(), "sr {sr}: {} events on snaps: {:?}", events.len(), &events[..events.len().min(3)]);
        assert!(hf.abs() < 0.5, "sr {sr}: snaps' band changed {hf:.3} dB");
    }
}

/// Speech with an "s" every couple of seconds (for the smoothness/block-size test).
fn speech_with_sibilants(sr: f32, secs: f32, seed: u64) -> (Vec<f32>, Vec<(usize, usize)>) {
    let mut x = speech_like(sr, secs, seed);
    let mut rng = Rng::new(seed + 100);
    let mut wins = Vec::new();
    let mut t = ms(sr, 500.0);
    while t + ms(sr, 300.0) < x.len() {
        let len = rng.range(0.06, 0.2);
        let amp = rng.range(0.1, 0.3);
        let s = sibilance(sr, &mut rng, len, amp);
        add_at(&mut x, &s, t);
        wins.push((t, t + s.len()));
        t += ms(sr, rng.range(1500.0, 2500.0));
    }
    (x, wins)
}

/// The gain curve, read off a 7 kHz probe tone in channel 1 (at the bell centre the output is
/// exactly g·probe), is smooth; and the output is identical for any block size.
#[test]
fn gain_curve_is_smooth_and_block_size_invariant() {
    let sr = 48_000.0;
    let (sig, wins) = speech_with_sibilants(sr, 20.0, 7);
    let amp = 1e-3;
    // Phase in f64: an f32 phase `2π·f·i/sr` jitters by ~0.1 rad after 20 s and spoils the estimate.
    let probe: Vec<f32> =
        (0..sig.len()).map(|i| amp * (std::f64::consts::TAU * 7000.0 * i as f64 / sr as f64).sin() as f32).collect();
    let probe = &probe[..];
    let (reference, ref_events, _) = process(&[&sig, probe], sr, 1);
    assert!(ref_events.len() >= wins.len() * 9 / 10, "{} events for {} sibilants", ref_events.len(), wins.len());

    // Gain estimate where the probe is well away from its zero crossings (after 10 ms settling).
    let mut last: Option<(usize, f32)> = None;
    let mut max_step = 0.0f32;
    let mut min_g = 1.0f32;
    for i in ms(sr, 10.0)..probe.len() {
        if probe[i].abs() < 0.7 * amp {
            continue;
        }
        let g = reference[1][i] / probe[i];
        min_g = min_g.min(g);
        if let Some((j, gj)) = last {
            max_step = max_step.max((g - gj).abs() / (i - j) as f32);
        }
        last = Some((i, g));
    }
    // A 1 ms attack moves at most (1 − e^(−1/48))·(1 − g_min) ≈ 0.016 per sample.
    assert!(min_g < 0.5, "probe saw no reduction (min gain {min_g:.3})");
    assert!(max_step < 0.02, "gain stepped {max_step:.4} per sample");
    assert!(max_jump(&reference[0]) <= max_jump(&sig) * 1.05 + 1e-4, "de-essing clicked");

    let mut worst = 0.0f32;
    for block in [64, 333, 512, 2048] {
        let (out, events, _) = process(&[&sig, probe], sr, block);
        let d = max_abs_diff(&out[0], &reference[0]).max(max_abs_diff(&out[1], &reference[1]));
        assert!(d <= 1e-6, "block {block}: output differs from block 1 by {d:e}");
        assert_eq!(events, ref_events, "block {block}: events differ");
        worst = worst.max(d);
    }
    eprintln!("MARGIN gain curve: max step {max_step:.4}/sample, min gain {min_g:.3}, block-size diff {worst:e}");
}

/// Parameter changes mid-stream (freq/Q glide, threshold/range jumps) don't click.
#[test]
fn parameter_changes_are_smooth() {
    let sr = 48_000.0;
    let (sig, _) = speech_with_sibilants(sr, 10.0, 9);
    let mut chain = chain(sr, 256, 1);
    let mut out = sig.clone();
    let mut rng = Rng::new(3);
    for (k, chunk) in out.chunks_mut(256).enumerate() {
        if k % 20 == 0 {
            let p = rng.range_usize(0, 6);
            let v = rng.range(-100.0, 20_000.0);
            chain.set_param(0, p, v);
        }
        chain.process(&mut AudioBlock::from_planar(chunk, 1));
    }
    assert!(out.iter().all(|x| x.is_finite()));
    let jump = max_jump(&out);
    let bound = max_jump(&sig) * 1.1 + 1e-3;
    eprintln!("MARGIN param changes: max jump {jump:.4} (input {:.4})", max_jump(&sig));
    assert!(jump <= bound, "parameter change clicked: jump {jump:.4} > {bound:.4}");
}

/// Gain reduction meter and trigger count follow the bursts.
#[test]
fn reports_gain_reduction_and_triggers() {
    let sr = 48_000.0;
    let (sig, wins) = speech_with_sibilants(sr, 20.0, 5);
    let (_, events, chain) = process(&[&sig], sr, 512);
    let m = chain.telemetry().meters(ModuleId::DeEss);
    assert_eq!(m.triggers as usize, events.len());
    let hits = wins
        .iter()
        .filter(|w| {
            events.iter().any(|e| (e.start_sample as usize) < w.1 && (e.start_sample as usize) + ms(sr, 5.0) >= w.0)
        })
        .count();
    assert!(hits >= wins.len() * 9 / 10, "{hits}/{} sibilants reported", wins.len());
    assert!(events.len() <= wins.len() + 2, "{} events for {} sibilants", events.len(), wins.len());
    for e in &events {
        assert_eq!(e.module, ModuleId::DeEss);
        assert!(e.energy_removed_db > 0.0);
    }
    assert!((m.detail[0] - 7000.0).abs() < 1.0, "band centre {}", m.detail[0]);
}
