#![allow(clippy::print_stderr)] // margins are printed for the PROGRESS log

//! Golden tests for de-hum (HANDOVER §7.1).
//!
//! - speech-like + 50 Hz hum with 5 harmonics → hum energy reduced ≥ 30 dB
//! - energy outside ±3 Hz of the notches changes < 0.5 dB
//! - no hum → no change (notches stay disengaged)
//! - 60 Hz mains is detected as 60 Hz; DC is removed; works at 44.1/48/96 kHz

use hearo_dsp::modules::dehum::DeHum;
use hearo_dsp::{AudioBlock, Chain, Module, ModuleId};
use hearo_testkit::metrics::{energy_db, energy_excluding_db, octave_bands_db};
use hearo_testkit::signals::{add_at, hum, speech_like};

const SECS: f32 = 10.0;
/// Measure after the detector has converged.
const SETTLE_S: f32 = 3.0;

fn process(input: &[f32], sr: f32) -> (Vec<f32>, Chain) {
    let mut chain = Chain::new(vec![Box::new(DeHum::new()) as Box<dyn Module>]);
    chain.prepare(sr, 512, 1);
    let mut out = input.to_vec();
    for chunk in out.chunks_mut(512) {
        chain.process(&mut AudioBlock::from_planar(chunk, 1));
    }
    (out, chain)
}

fn tail(x: &[f32], sr: f32) -> &[f32] {
    &x[(SETTLE_S * sr) as usize..]
}

fn notch_bands(f0: f32, sr: f32, half_width: f32) -> Vec<(f32, f32)> {
    (1..=8).map(|k| k as f32 * f0).filter(|f| *f < sr / 2.0).map(|f| (f - half_width, f + half_width)).collect()
}

fn check_hum_removed(sr: f32, f0: f32) {
    check_hum_removed_at(sr, f0, f0);
}

/// `f0` is the actual hum frequency; `nominal` the mains standard it should be detected as.
fn check_hum_removed_at(sr: f32, f0: f32, nominal: f32) {
    let speech = speech_like(sr, SECS, 11);
    let h = hum(sr, f0, 5, SECS, 0.05);
    let mut input = speech.clone();
    add_at(&mut input, &h, 0);
    let (out, chain) = process(&input, sr);

    // Hum residual by superposition: once engaged, de-hum is linear, so
    // out(speech + 2·hum) − out(speech + hum) = out(hum) exactly — speech cancels. (Speech
    // energy sitting within a fraction of a hertz of the hum harmonics makes any single-signal
    // measurement speech-limited; see deep dive 04.)
    let mut input2 = speech.clone();
    let h2: Vec<f32> = h.iter().map(|x| 2.0 * x).collect();
    add_at(&mut input2, &h2, 0);
    let (out2, _) = process(&input2, sr);
    let residual: Vec<f32> = tail(&out2, sr).iter().zip(tail(&out, sr)).map(|(a, b)| a - b).collect();
    let reduction = energy_db(tail(&h, sr)) - energy_db(&residual);
    assert!(reduction >= 30.0, "sr {sr} f0 {f0}: hum reduced only {reduction:.1} dB");
    eprintln!("MARGIN sr {sr} f0 {f0}: hum −{reduction:.1} dB");

    let wide = notch_bands(f0, sr, 3.0);
    let rest_in = energy_excluding_db(tail(&input, sr), sr, &wide);
    let rest_out = energy_excluding_db(tail(&out, sr), sr, &wide);
    assert!((rest_in - rest_out).abs() < 0.5, "sr {sr}: energy outside notches changed {:.2} dB", rest_in - rest_out);

    let m = chain.telemetry().meters(ModuleId::DeHum);
    assert!(
        (m.detail[0] - f0).abs() < 0.5 || (m.detail[0] - nominal).abs() < 0.5,
        "detected f0 {} for {f0}",
        m.detail[0]
    );
    assert!(m.detail[2] > 0.95, "notch depth {} should be fully engaged", m.detail[2]);
}

#[test]
fn removes_50hz_hum_at_48k() {
    check_hum_removed(48_000.0, 50.0);
}

#[test]
fn detects_and_removes_60hz_hum() {
    check_hum_removed(48_000.0, 60.0);
}

#[test]
fn works_at_44k1_and_96k() {
    check_hum_removed(44_100.0, 50.0);
    check_hum_removed(96_000.0, 60.0);
}

#[test]
fn leaves_clean_speech_alone() {
    let sr = 48_000.0;
    let speech = speech_like(sr, SECS, 12);
    let (out, chain) = process(&speech, sr);
    // Not thinned: every octave band within 0.1 dB. (The 2 Hz DC blocker shifts low-frequency
    // phase slightly, which is inaudible, so we compare spectra rather than waveforms.)
    for ((f, a), (_, b)) in octave_bands_db(tail(&speech, sr), sr).iter().zip(octave_bands_db(tail(&out, sr), sr)) {
        assert!((a - b).abs() < 0.1, "{f} Hz band changed {:.3} dB", a - b);
    }
    assert!(chain.telemetry().meters(ModuleId::DeHum).detail[2] < 0.05, "notches engaged without hum");
}

#[test]
fn removes_dc_offset() {
    let sr = 48_000.0;
    let input: Vec<f32> = speech_like(sr, 4.0, 13).iter().map(|x| x + 0.2).collect();
    let (out, _) = process(&input, sr);
    let t = tail(&out, sr);
    let mean = t.iter().sum::<f32>() / t.len() as f32;
    assert!(mean.abs() < 1e-3, "DC left: {mean}");
}

// ── Review findings (2026-11-13): realistic mains drift and musical false positives ─────────

#[test]
fn removes_hum_with_mains_drift() {
    for f0 in [49.9, 50.1, 59.9, 60.1] {
        check_hum_removed_at(48_000.0, f0, if f0 < 55.0 { 50.0 } else { 60.0 });
    }
}

#[test]
fn held_bass_notes_do_not_engage() {
    let sr = 48_000.0;
    for note in [49.0, 98.0, 100.0, 120.0, 150.0] {
        let mut input = speech_like(sr, SECS, 14);
        // A sustained bass note with a few harmonics at -26 dBFS, like a held synth bass.
        add_at(&mut input, &hum(sr, note, 3, SECS, 0.1), 0);
        let (_, chain) = process(&input, sr);
        let depth = chain.telemetry().meters(ModuleId::DeHum).detail[2];
        assert!(depth < 0.05, "a held {note} Hz note engaged the notches (depth {depth:.2})");
    }
}

/// Hum that differs between channels (e.g. a ground loop on one side, or out of phase) must be
/// detected and removed from both channels (dsp-reviewer finding: a mono sum can cancel it).
#[test]
fn removes_out_of_phase_stereo_hum() {
    let sr = 48_000.0;
    let speech = speech_like(sr, SECS, 15);
    let h = hum(sr, 50.0, 5, SECS, 0.05);
    // L = speech + g·hum, R = speech − g·hum.
    let run = |g: f32| -> (Vec<f32>, Vec<f32>, f32) {
        let l: Vec<f32> = speech.iter().zip(&h).map(|(s, h)| s + g * h).collect();
        let r: Vec<f32> = speech.iter().zip(&h).map(|(s, h)| s - g * h).collect();
        let mut chain = Chain::new(vec![Box::new(DeHum::new()) as Box<dyn Module>]);
        chain.prepare(sr, 512, 2);
        let mut buf = vec![0.0f32; 1024];
        let (mut ol, mut or) = (Vec::new(), Vec::new());
        for (cl, cr) in l.chunks(512).zip(r.chunks(512)) {
            let k = cl.len();
            buf[..k].copy_from_slice(cl);
            buf[k..2 * k].copy_from_slice(cr);
            chain.process(&mut AudioBlock::from_planar(&mut buf[..2 * k], 2));
            ol.extend_from_slice(&buf[..k]);
            or.extend_from_slice(&buf[k..2 * k]);
        }
        (ol, or, chain.telemetry().meters(ModuleId::DeHum).detail[2])
    };
    let (l1, r1, depth) = run(1.0);
    assert!(depth > 0.95, "out-of-phase hum not engaged (depth {depth})");
    let (l2, r2, _) = run(2.0);
    // Superposition, as in check_hum_removed: the difference is the processed hum alone.
    for (a, b, name) in [(&l2, &l1, "L"), (&r2, &r1, "R")] {
        let resid: Vec<f32> = tail(a, sr).iter().zip(tail(b, sr)).map(|(x, y)| x - y).collect();
        let red = energy_db(tail(&h, sr)) - energy_db(&resid);
        assert!(red >= 30.0, "{name}: hum reduced only {red:.1} dB");
        eprintln!("MARGIN out-of-phase stereo {name}: hum −{red:.1} dB");
    }
}

/// The `amount` parameter (the simple UI's "how much"): hum reduction ≈ −20·log10(1 − amount),
/// and the gain-reduction meter reports it (capped by the tonal score — here the hum stands out by
/// more than 20 dB, so the cap doesn't bite).
#[test]
fn amount_scales_the_reduction() {
    let sr = 48_000.0;
    let speech = speech_like(sr, SECS, 11);
    let h = hum(sr, 50.0, 5, SECS, 0.05);
    let run = |hum_gain: f32, amount: f32| {
        let mut input = speech.clone();
        let hh: Vec<f32> = h.iter().map(|x| hum_gain * x).collect();
        add_at(&mut input, &hh, 0);
        let mut chain = Chain::new(vec![Box::new(DeHum::new()) as Box<dyn Module>]);
        chain.prepare(sr, 512, 1);
        let amount_idx = chain.module(0).unwrap().params().iter().position(|p| p.key == "amount").unwrap();
        assert_eq!(chain.module(0).unwrap().primary_param(), Some(amount_idx));
        chain.set_param(0, amount_idx, amount);
        for chunk in input.chunks_mut(512) {
            chain.process(&mut AudioBlock::from_planar(chunk, 1));
        }
        (input, chain.telemetry().meters(ModuleId::DeHum).gain_reduction_db)
    };
    for (amount, lo, hi) in [(0.0, -0.5, 0.5), (0.5, 5.0, 7.0), (0.9, 18.0, 22.0)] {
        let (a, gr) = run(1.0, amount);
        let (b, _) = run(2.0, amount);
        let residual: Vec<f32> = tail(&b, sr).iter().zip(tail(&a, sr)).map(|(x, y)| x - y).collect();
        let reduction = energy_db(tail(&h, sr)) - energy_db(&residual);
        assert!((lo..=hi).contains(&reduction), "amount {amount}: hum reduced {reduction:.1} dB");
        eprintln!("MARGIN amount {amount}: hum −{reduction:.1} dB (window {lo}..{hi})");
        let expected = -20.0 * (1.0 - amount).log10();
        assert!((gr - expected).abs() < 1.5, "amount {amount}: meter {gr:.1} dB, expected ≈ {expected:.1}");
    }
}

/// Turning `amount` down and up while hum is being removed ramps (no click) and the full
/// reduction comes back.
#[test]
fn amount_changes_mid_stream_are_smooth() {
    let sr = 48_000.0;
    let mut input = speech_like(sr, SECS, 11);
    add_at(&mut input, &hum(sr, 50.0, 5, SECS, 0.05), 0);
    let run = |switch: bool| {
        let mut chain = Chain::new(vec![Box::new(DeHum::new()) as Box<dyn Module>]);
        chain.prepare(sr, 512, 1);
        let mut out = input.clone();
        for (i, chunk) in out.chunks_mut(512).enumerate() {
            if switch {
                let t = i * 512;
                // amount 1 → 0 at 4 s, back to 1 at 6 s.
                if t == 375 * 512 {
                    chain.set_param(0, 3, 0.0);
                } else if t == 563 * 512 {
                    chain.set_param(0, 3, 1.0);
                }
            }
            chain.process(&mut AudioBlock::from_planar(chunk, 1));
        }
        out
    };
    let (plain, switched) = (run(false), run(true));
    let max_step = |x: &[f32]| x.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
    let from = (3.5 * sr) as usize;
    let (a, b) = (max_step(&plain[from..]), max_step(&switched[from..]));
    // The only extra sample-to-sample movement is the hum itself fading back in (≤ its slope).
    let hum_slope = 0.05 * 5.0 * 2.0 * std::f32::consts::PI * 250.0 / sr;
    assert!(b <= a + 2.0 * hum_slope, "amount change clicked: max step {b} vs {a}");
    // Back at full depth after the 0.25 s depth ramp (+ margin): hum gone again.
    let end = (9.0 * sr) as usize;
    let diff: Vec<f32> = switched[end..].iter().zip(&plain[end..]).map(|(x, y)| x - y).collect();
    assert!(energy_db(&diff) - energy_db(&plain[end..]) < -30.0, "full reduction did not come back");
}
