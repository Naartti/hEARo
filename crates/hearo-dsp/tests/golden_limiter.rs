#![allow(clippy::print_stderr)] // margins are printed for the PROGRESS log

//! Golden tests for the true-peak safety limiter (HANDOVER §7.5): no output sample and no
//! 4×-oversampled true peak above the ceiling on hot material, bit-exact below it, smooth and
//! block-size invariant, at 44.1/48/96 kHz. Measurable outcomes, not exact samples.

use std::f32::consts::PI;

use hearo_dsp::modules::limiter::Limiter;
use hearo_dsp::{AudioBlock, Chain, Event, EventKind, Module, ModuleId};
use hearo_testkit::Rng;
use hearo_testkit::metrics::{db, max_jump, peak};
use hearo_testkit::signals::{drum_hit, music_like, pink, speech_like};
use proptest::prelude::*;

const CEILING_DB: f32 = -1.0;
/// Label prefix for signals with strong content above 0.4·fs.
const NEAR_NYQUIST: &str = "[HF]";
const RATES: [f32; 3] = [44_100.0, 48_000.0, 96_000.0];

/// Reference true-peak meter: 4× oversampling with a long (64 taps per phase) Kaiser-windowed
/// sinc in f64 — much more accurate than the limiter's own 12-tap estimate, so the test checks
/// the limiter against an independent measurement, not against itself.
fn true_peak(x: &[f32]) -> f32 {
    true_peak_inner(x, 0)
}

/// [`true_peak`] ignoring the first and last `skip` samples.
fn true_peak_inner(x: &[f32], skip: usize) -> f32 {
    const HALF: i64 = 32;
    const BETA: f64 = 8.0;
    let i0 = |v: f64| {
        let (mut s, mut t) = (1.0f64, 1.0f64);
        for k in 1..60 {
            t *= v * v / 4.0 / (k * k) as f64;
            s += t;
        }
        s
    };
    let kernels: Vec<Vec<f64>> = (1..4)
        .map(|p| {
            let f = p as f64 / 4.0;
            let k: Vec<f64> = (-HALF + 1..=HALF)
                .map(|j| {
                    let u = j as f64 - f;
                    let sinc = (std::f64::consts::PI * u).sin() / (std::f64::consts::PI * u);
                    let r = (u / HALF as f64).clamp(-1.0, 1.0);
                    sinc * i0(BETA * (1.0 - r * r).sqrt()) / i0(BETA)
                })
                .collect();
            let dc: f64 = k.iter().sum();
            k.into_iter().map(|v| v / dc).collect()
        })
        .collect();
    let n = x.len() as i64;
    let mut tp = peak(&x[skip..x.len() - skip]) as f64;
    for m in skip as i64..n - skip as i64 {
        for k in &kernels {
            let mut acc = 0.0f64;
            for (t, c) in k.iter().enumerate() {
                let idx = m + t as i64 - HALF + 1;
                if (0..n).contains(&idx) {
                    acc += c * x[idx as usize] as f64;
                }
            }
            tp = tp.max(acc.abs());
        }
    }
    tp as f32
}

/// Stereo chain run, latency-compensated, in `block`-sized pieces; returns (L, R, events).
fn run_stereo(left: &[f32], right: &[f32], sr: f32, block: usize) -> (Vec<f32>, Vec<f32>, Vec<Event>) {
    let mut chain = Chain::new(vec![Box::new(Limiter::new()) as Box<dyn Module>]);
    chain.prepare(sr, block.min(4096), 2);
    let lat = chain.latency_samples();
    let n = left.len() + lat;
    let pad = |x: &[f32]| x.iter().copied().chain(std::iter::repeat_n(0.0, lat)).collect::<Vec<f32>>();
    let (l, r) = (pad(left), pad(right));
    let (mut ol, mut or) = (Vec::with_capacity(n), Vec::with_capacity(n));
    let mut events = Vec::new();
    let mut buf = vec![0.0f32; 2 * block];
    let mut pos = 0;
    while pos < n {
        let k = block.min(n - pos);
        buf[..k].copy_from_slice(&l[pos..pos + k]);
        buf[k..2 * k].copy_from_slice(&r[pos..pos + k]);
        chain.process(&mut AudioBlock::from_planar(&mut buf[..2 * k], 2));
        ol.extend_from_slice(&buf[..k]);
        or.extend_from_slice(&buf[k..2 * k]);
        events.extend_from_slice(chain.telemetry().events());
        chain.telemetry_mut().clear_events();
        pos += k;
    }
    assert_eq!(chain.faults(), 0);
    ol.drain(..lat);
    or.drain(..lat);
    (ol, or, events)
}

fn run_mono(x: &[f32], sr: f32) -> Vec<f32> {
    run_stereo(x, x, sr, 512).0
}

fn scale_to_peak_db(mut x: Vec<f32>, peak_db: f32) -> Vec<f32> {
    let g = 10f32.powf(peak_db / 20.0) / peak(&x);
    x.iter_mut().for_each(|v| *v *= g);
    x
}

fn sine_phase(sr: f32, freq: f32, secs: f32, amp: f32, phase: f32) -> Vec<f32> {
    (0..(sr * secs) as usize).map(|i| amp * (2.0 * PI * freq * i as f32 / sr + phase).sin()).collect()
}

/// Hot test signals for one sample rate: (name, signal).
fn hot_signals(sr: f32) -> Vec<(String, Vec<f32>)> {
    let amp = |d: f32| 10f32.powf(d / 20.0);
    let mut v: Vec<(String, Vec<f32>)> = Vec::new();
    for f in [50.0, 1000.0, 5000.0] {
        for d in [6.0, 12.0] {
            v.push((format!("sine {f:.0} Hz +{d} dBFS"), sine_phase(sr, f, 1.0, amp(d), 0.0)));
        }
    }
    for d in [6.0, 12.0] {
        v.push((format!("{NEAR_NYQUIST} sine 0.42·fs +{d} dBFS"), sine_phase(sr, 0.42 * sr, 1.0, amp(d), 0.0)));
    }
    v.push((
        format!("{NEAR_NYQUIST} white noise 0 dBFS RMS"),
        hearo_testkit::signals::white(&mut Rng::new(5), sr as usize, 1.0),
    ));
    // fs/4 at 45°: every sample is at 0.707 × the true peak (+3 dB inter-sample peaks).
    v.push(("fs/4 45° samples −1.5 dBFS (TP +1.5)".into(), sine_phase(sr, sr / 4.0, 1.0, amp(1.5), PI / 4.0)));
    v.push(("fs/4 45° +12 dBTP".into(), sine_phase(sr, sr / 4.0, 1.0, amp(12.0), PI / 4.0)));
    // Square-ish: a naive square (Gibbs overshoot between samples) and a soft-clipped sine.
    let sq: Vec<f32> = sine_phase(sr, 997.0, 1.0, 1.0, 0.1).iter().map(|s| amp(6.0) * s.signum()).collect();
    v.push(("square 997 Hz +6 dBFS".into(), sq));
    let soft: Vec<f32> = sine_phase(sr, 220.0, 1.0, 1.0, 0.0).iter().map(|s| amp(9.0) * (4.0 * s).tanh()).collect();
    v.push(("tanh-square 220 Hz +9 dBFS".into(), soft));
    for seed in 7..11 {
        v.push((format!("pink noise +6 dBFS RMS #{seed}"), pink(&mut Rng::new(seed), (sr * 2.0) as usize, amp(6.0))));
    }
    v.push(("speech_like peak +10 dBFS".into(), scale_to_peak_db(speech_like(sr, 3.0, 11), 10.0)));
    for seed in [12, 14] {
        v.push((
            format!("music_like (drums) peak +10 dBFS #{seed}"),
            scale_to_peak_db(music_like(sr, 3.0, seed), 10.0),
        ));
    }
    let mut drums = vec![0.0f32; (sr * 2.0) as usize];
    let mut rng = Rng::new(13);
    for k in 0..8 {
        let hit = drum_hit(sr, &mut rng, amp(12.0));
        hearo_testkit::signals::add_at(&mut drums, &hit, k * (sr * 0.25) as usize);
    }
    v.push(("drum hits +12 dBFS".into(), drums));
    v
}

/// §7.5: no sample and no 4× true peak above the ceiling, and the limiter doesn't over-squash.
/// Every signal is measured and printed before asserting, so a failure shows the whole table.
fn check_ceiling(sr: f32) {
    let ceiling = 10f32.powf(CEILING_DB / 20.0);
    let mut worst_tp = f32::MIN;
    let mut worst_name = String::new();
    let mut failures = Vec::new();
    for (name, x) in hot_signals(sr) {
        let y = run_mono(&x, sr);
        let tp_in = db(true_peak(&x));
        let sp = peak(&y);
        let tp = db(true_peak(&y));
        let (ji, jo) = (max_jump(&x), max_jump(&y));
        eprintln!("  sr {sr} {name:<36} in {tp_in:+6.2} dBTP -> out {tp:+7.3} dBTP, jump {jo:.3} (in {ji:.3})");
        if !y.iter().all(|v| v.is_finite()) {
            failures.push(format!("{name}: non-finite output"));
        }
        if sp > ceiling {
            failures.push(format!("{name}: sample peak {:.3} dBFS > ceiling", db(sp)));
        }
        if tp > CEILING_DB {
            failures.push(format!("{name}: true peak {tp:.3} dBTP > {CEILING_DB} dBTP"));
        }
        // Not over-limiting: hot material ends up within 1.5 dB of the ceiling. Content near
        // Nyquist is limited conservatively (the HF guard; see limiter.rs), by up to ~5.5 dB.
        let floor = if name.starts_with(NEAR_NYQUIST) { CEILING_DB - 6.0 } else { CEILING_DB - 1.5 };
        if tp < floor {
            failures.push(format!("{name}: over-limited to {tp:.2} dBTP"));
        }
        // Never makes the signal jumpier than it was (no clicks from the gain).
        if jo > ji {
            failures.push(format!("{name}: max jump {jo:.3} > input {ji:.3}"));
        }
        if tp > worst_tp {
            worst_tp = tp;
            worst_name = name;
        }
    }
    eprintln!(
        "MARGIN sr {sr}: max true peak {worst_tp:.3} dBTP ({worst_name}), {:.3} dB under the {CEILING_DB} dBTP ceiling",
        CEILING_DB - worst_tp
    );
    assert!(failures.is_empty(), "sr {sr}: {failures:#?}");
}

#[test]
fn reference_meter_reads_intersample_peaks() {
    let x = sine_phase(48_000.0, 12_000.0, 0.05, 1.0, PI / 4.0);
    assert!((db(peak(&x)) + 3.01).abs() < 0.02, "samples sit at −3 dB, got {:.3}", db(peak(&x)));
    // Skip the abrupt start/end (their Gibbs ringing is real true-peak content, not the sine's).
    let tp = db(true_peak_inner(&x, 64));
    assert!(tp.abs() < 0.02, "meter must read the 0 dB true peak, got {tp:.3}");
}

#[test]
fn never_exceeds_ceiling_at_48k() {
    check_ceiling(48_000.0);
}

#[test]
fn never_exceeds_ceiling_at_44k1() {
    check_ceiling(44_100.0);
}

#[test]
fn never_exceeds_ceiling_at_96k() {
    check_ceiling(96_000.0);
}

/// Below the threshold the limiter is a bit-exact delay (no gain applied at all).
#[test]
fn transparent_below_threshold() {
    for sr in RATES {
        let speech = speech_like(sr, 5.0, 21);
        let tp_in = db(true_peak(&speech));
        assert!(tp_in < CEILING_DB - 1.0, "test signal itself is too hot ({tp_in:.2} dBTP)");
        let out = run_mono(&speech, sr);
        assert_eq!(out, speech, "sr {sr}: speech_like at −20 dBFS RMS must pass bit-exact");
        // Just under the ceiling (−1.5 dBTP) is still untouched.
        let near = scale_to_peak_db(speech.clone(), 0.0);
        let near = {
            let g = 10f32.powf((CEILING_DB - 0.5) / 20.0) / true_peak(&near);
            near.iter().map(|v| v * g).collect::<Vec<f32>>()
        };
        let out = run_mono(&near, sr);
        let changed = out.iter().zip(&near).filter(|(a, b)| a != b).count();
        let diff_db = db(out.iter().zip(&near).fold(0.0f32, |m, (a, b)| m.max((a - b).abs())) / peak(&near));
        assert!(changed == 0, "sr {sr}: {changed} samples changed at −1.5 dBTP (max diff {diff_db:.1} dB re peak)");
        eprintln!("MARGIN transparency sr {sr}: speech at {tp_in:.1} dBTP and −1.5 dBTP bit-exact (0 samples changed)");
    }
}

/// Output (and the gain curve itself, read from a constant probe on the linked channel) is
/// identical for every block size, and the gain never steps faster than the attack ramp.
#[test]
fn block_size_invariant_and_smooth() {
    for sr in RATES {
        let hot = scale_to_peak_db(music_like(sr, 4.0, 31), 12.0);
        let probe = vec![1e-4f32; hot.len()];
        let (ref_l, ref_r, _) = run_stereo(&hot, &probe, sr, 512);
        let gain: Vec<f32> = ref_r.iter().map(|v| v / 1e-4).collect();
        let step = gain.windows(2).fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()));
        let max_step = 1.0 / (0.25e-3 * sr);
        assert!(step <= max_step, "sr {sr}: gain stepped {step:.4}/sample (max {max_step:.4})");
        assert!(gain.iter().all(|g| *g <= 1.0 + 1e-6), "gain above unity");
        for block in [1, 64, 333, 2048] {
            let (l, r, _) = run_stereo(&hot, &probe, sr, block);
            assert!(l == ref_l && r == ref_r, "sr {sr}: block {block} differs from block 512");
        }
        let gr = -db(gain.iter().fold(1.0f32, |m, g| m.min(*g)));
        eprintln!("MARGIN smooth sr {sr}: max gain step {step:.4}/sample (limit {max_step:.4}), max GR {gr:.1} dB");
    }
}

/// Stereo-linked: a peak in one channel turns both down by the same gain (image stays put).
#[test]
fn stereo_linked() {
    let sr = 48_000.0;
    let hot = sine_phase(sr, 440.0, 1.0, 4.0, 0.0);
    let quiet = sine_phase(sr, 660.0, 1.0, 0.1, 0.0);
    let (l, r, _) = run_stereo(&hot, &quiet, sr, 512);
    let tail = (sr * 0.5) as usize;
    let gl = peak(&l[tail..]) / 4.0;
    let gr = peak(&r[tail..]) / 0.1;
    assert!((db(gl) - db(gr)).abs() < 0.2, "L gain {:.2} dB vs R gain {:.2} dB", db(gl), db(gr));
}

/// Latency is exact, constant per sample rate, and within the 0.5 ms budget.
#[test]
fn latency_is_exact_and_within_budget() {
    for (sr, expect) in [(44_100.0, 22), (48_000.0, 24), (96_000.0, 48)] {
        let mut m = Limiter::new();
        m.prepare(sr, 512, 1);
        assert_eq!(m.latency_samples(), expect, "sr {sr}");
        assert!(m.latency_samples() as f32 / sr <= 0.5e-3 + 1e-9);
        let mut x = vec![0.0f32; 200];
        x[3] = 0.5;
        let mut t = hearo_dsp::TelemetryWriter::new();
        m.process(&mut AudioBlock::from_planar(&mut x, 1), &mut t);
        assert_eq!(x.iter().position(|v| *v != 0.0), Some(3 + expect), "sr {sr}: impulse delay");
        assert_eq!(x[3 + expect], 0.5, "impulse passes bit-exact");
    }
}

/// Telemetry: gain-reduction meter and one `Limit` event per engagement.
#[test]
fn one_event_per_engagement() {
    let sr = 48_000.0;
    let mut x = vec![0.0f32; (sr * 4.0) as usize];
    let burst = sine_phase(sr, 1000.0, 0.2, 2.0, 0.0); // +6 dBFS
    for k in 0..3 {
        hearo_testkit::signals::add_at(&mut x, &burst, (k as f32 * sr) as usize + 1000);
    }
    let (_, _, events) = run_stereo(&x, &x, sr, 256);
    let lim: Vec<&Event> = events.iter().filter(|e| e.module == ModuleId::Limiter).collect();
    assert_eq!(lim.len(), 3, "events: {lim:?}");
    for (k, e) in lim.iter().enumerate() {
        assert_eq!(e.kind, EventKind::Limit);
        let onset = (k as f32 * sr) as i64 + 1000;
        assert!((e.start_sample as i64 - onset).abs() < (sr * 0.002) as i64, "event {k} at {}", e.start_sample);
        assert!((6.0..8.5).contains(&e.energy_removed_db), "GR {:.2} dB", e.energy_removed_db);
    }
    // Meter shows block-max GR during a burst.
    let mut chain = Chain::new(vec![Box::new(Limiter::new()) as Box<dyn Module>]);
    chain.prepare(sr, 512, 1);
    let mut b = sine_phase(sr, 1000.0, 0.05, 2.0, 0.0);
    chain.process(&mut AudioBlock::from_planar(&mut b, 1));
    let gr = chain.telemetry().meters(ModuleId::Limiter).gain_reduction_db;
    assert!((6.0..8.5).contains(&gr), "meter GR {gr:.2} dB");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Random hot input at random block sizes and ceilings: never above the ceiling (sample
    /// peak), always finite.
    #[test]
    fn random_hot_input_never_exceeds_ceiling(
        seed in any::<u64>(),
        amp_db in -10.0f32..30.0,
        ceiling_db in -6.0f32..0.0,
        sizes in prop::collection::vec(1usize..700, 1..8),
        sr_idx in 0usize..3,
        tone in any::<bool>(),
    ) {
        let sr = RATES[sr_idx];
        let n = (sr * 0.15) as usize;
        let mut rng = Rng::new(seed);
        let a = 10f32.powf(amp_db / 20.0);
        let freq = rng.range(30.0, 0.49 * sr);
        let x: Vec<f32> = (0..n)
            .map(|i| {
                let t = if tone { (2.0 * PI * freq * i as f32 / sr).sin() } else { 0.0 };
                a * (0.5 * t + rng.gaussian())
            })
            .collect();
        let mut chain = Chain::new(vec![Box::new(Limiter::new()) as Box<dyn Module>]);
        chain.prepare(sr, 1024, 2);
        chain.set_param(0, 0, ceiling_db);
        let c = 10f32.powf(ceiling_db / 20.0);
        let mut buf = vec![0.0f32; 2 * 700];
        let (mut pos, mut k) = (0, 0);
        while pos < n {
            let len = sizes[k % sizes.len()].min(n - pos);
            k += 1;
            buf[..len].copy_from_slice(&x[pos..pos + len]);
            for (j, v) in buf[len..2 * len].iter_mut().enumerate() {
                *v = -0.7 * x[pos + j];
            }
            chain.process(&mut AudioBlock::from_planar(&mut buf[..2 * len], 2));
            for &v in &buf[..2 * len] {
                prop_assert!(v.is_finite());
                prop_assert!(v.abs() <= c, "{} > ceiling {}", v.abs(), c);
            }
            pos += len;
        }
    }
}

/// Absurd input (±3e38, ±inf, NaN) never produces non-finite output or samples above the ceiling,
/// and the limiter recovers to transparent once the input is normal again (rt-safety review).
#[test]
fn survives_non_finite_input_and_recovers() {
    use hearo_dsp::modules::limiter::Limiter;
    use hearo_dsp::{AudioBlock, Module};
    let sr = 48_000.0;
    // Feed the module directly (the chain would zero non-finite input before it).
    let mut m = Limiter::new();
    m.prepare(sr, 512, 1);
    let mut t = hearo_dsp::TelemetryWriter::new();
    let ceil = 10f32.powf(-1.0 / 20.0);
    let mut bad: Vec<f32> = (0..512)
        .map(|i| match i % 4 {
            0 => 3e38,
            1 => f32::INFINITY,
            2 => f32::NAN,
            _ => -3e38,
        })
        .collect();
    m.process(&mut AudioBlock::from_planar(&mut bad, 1), &mut t);
    assert!(bad.iter().all(|x| x.is_finite() && x.abs() <= ceil + 1e-6), "non-finite or over-ceiling output");
    // One second of quiet speech-level sine afterwards: back to unity gain (release ≤ 1 s).
    let mut quiet: Vec<f32> = (0..48_000).map(|i| 0.1 * (i as f32 * 0.05).sin()).collect();
    for c in quiet.chunks_mut(512) {
        m.process(&mut AudioBlock::from_planar(c, 1), &mut t);
    }
    let tail = &quiet[40_000..];
    assert!(tail.iter().any(|x| x.abs() > 0.09), "limiter stuck reducing after absurd input");
}
