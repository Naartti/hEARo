#![allow(clippy::print_stderr)] // margins are printed for the PROGRESS log

//! Golden tests for de-plosive (HANDOVER §7.4; deep dive 04). Measurable outcomes, not exact
//! samples. Zero lookahead, so outputs need no latency compensation (asserted).

use std::f32::consts::PI;

use hearo_dsp::modules::deplosive::DePlosive;
use hearo_dsp::{AudioBlock, Chain, Event, Module, ModuleId};
use hearo_testkit::Rng;
use hearo_testkit::metrics::{band_energy_db, energy_db, max_jump};
use hearo_testkit::signals::{Injected, add_at, music_like, pink, plosive, pluck, sine, speech_like};

const FLOOR_DB: f32 = -60.0;

fn ms(sr: f32, m: f32) -> usize {
    (sr * m / 1000.0) as usize
}

fn chain() -> Chain {
    Chain::new(vec![Box::new(DePlosive::new()) as Box<dyn Module>])
}

/// Mono run in `block`-sized chunks, collecting events.
fn process_block(input: &[f32], sr: f32, block: usize) -> (Vec<f32>, Vec<Event>, Chain) {
    let mut chain = chain();
    chain.prepare(sr, block, 1);
    assert_eq!(chain.latency_samples(), 0, "de-plosive must not add latency");
    let mut buf = input.to_vec();
    let mut events = Vec::new();
    for chunk in buf.chunks_mut(block) {
        chain.process(&mut AudioBlock::from_planar(chunk, 1));
        events.extend_from_slice(chain.telemetry().events());
        chain.telemetry_mut().clear_events();
    }
    (buf, events, chain)
}

fn process(input: &[f32], sr: f32) -> (Vec<f32>, Vec<Event>, Chain) {
    process_block(input, sr, 512)
}

fn with_floor(mut x: Vec<f32>, seed: u64) -> Vec<f32> {
    let floor = pink(&mut Rng::new(seed), x.len(), 10f32.powf(FLOOR_DB / 20.0));
    add_at(&mut x, &floor, 0);
    x
}

/// Speech-like signal with one LF plosive pop (30–80 Hz, 40–80 ms) per second at a random
/// offset — some land in pauses, most on top of syllables — peaks `level_db` dBFS.
fn speech_with_plosives(sr: f32, secs: f32, seed: u64, level_db: (f32, f32)) -> (Vec<f32>, Vec<Injected>) {
    let mut s = with_floor(speech_like(sr, secs, seed), seed ^ 0xF1);
    let mut rng = Rng::new(seed ^ 0xB0B);
    let mut pops = Vec::new();
    let mut t = ms(sr, 500.0);
    while t + sr as usize <= s.len() {
        let start = t + rng.range_usize(0, ms(sr, 500.0));
        let amp = 10f32.powf(rng.range(level_db.0, level_db.1) / 20.0);
        let p = plosive(sr, &mut rng, amp);
        add_at(&mut s, &p, start);
        pops.push(Injected { start, len: p.len() });
        t += sr as usize;
    }
    (s, pops)
}

/// Sum of the < 150 Hz energy of `x` over the windows (per-window FFT), in dB.
fn lf_energy_db(x: &[f32], sr: f32, wins: &[(usize, usize)]) -> f32 {
    let e: f64 = wins
        .iter()
        .map(|&(a, b)| 10f64.powf(band_energy_db(&x[a..b.min(x.len())], sr, 0.0, 150.0) as f64 / 10.0))
        .sum();
    (10.0 * e.max(1e-30).log10()) as f32
}

fn energy_outside(input: &[f32], out: &[f32], wins: &[(usize, usize)]) -> f32 {
    let mut mask = vec![true; input.len()];
    for &(a, b) in wins {
        mask[a..b.min(input.len())].iter_mut().for_each(|m| *m = false);
    }
    let pick = |x: &[f32]| x.iter().zip(&mask).filter(|(_, m)| **m).map(|(v, _)| *v).collect::<Vec<f32>>();
    (energy_db(&pick(out)) - energy_db(&pick(input))).abs()
}

fn matched(ev: &Event, p: &Injected) -> bool {
    let t = ev.start_sample as usize;
    t >= p.start && t <= p.end()
}

/// Pops at `level` dBFS peak must lose ≥ `min_reduction` dB of LF energy, with ≥ `min_recall`.
fn check_plosives(sr: f32, secs: f32, seed: u64, level: (f32, f32), min_reduction: f32, min_recall: f32) {
    let (input, pops) = speech_with_plosives(sr, secs, seed, level);
    let (out, events, _) = process(&input, sr);
    let ev: Vec<&Event> = events.iter().filter(|e| e.module == ModuleId::DePlosive).collect();
    let hits = pops.iter().filter(|p| ev.iter().any(|e| matched(e, p))).count();
    let recall = hits as f32 / pops.len() as f32;

    let wins: Vec<(usize, usize)> = pops.iter().map(|p| (p.start, p.end())).collect();
    let reduction = lf_energy_db(&input, sr, &wins) - lf_energy_db(&out, sr, &wins);
    // Outside the pops plus their release tail, speech is untouched.
    let padded: Vec<(usize, usize)> = pops.iter().map(|p| (p.start, p.end() + ms(sr, 100.0))).collect();
    let outside = energy_outside(&input, &out, &padded);
    let extra = ev.len() - ev.iter().filter(|e| pops.iter().any(|p| matched(e, p))).count();
    eprintln!(
        "MARGIN sr {sr}, pops {level:?} dBFS: LF burst energy −{reduction:.1} dB (need ≥ {min_reduction}), recall {recall:.2} ({hits}/{}), \
         {extra} unmatched events, outside change {outside:.3} dB",
        pops.len()
    );
    assert!(reduction >= min_reduction, "sr {sr}: LF burst energy reduced only {reduction:.1} dB");
    assert!(recall >= min_recall, "sr {sr}: recall {recall:.2} ({hits}/{})", pops.len());
    assert!(outside < 0.3, "sr {sr}: audio outside the pops changed {outside:.2} dB");
    assert!(max_jump(&out) <= max_jump(&input) * 1.05 + 1e-4, "sr {sr}: the cut clicked");
}

#[test]
fn removes_plosives_at_48k() {
    check_plosives(48_000.0, 60.0, 11, (-12.0, -4.0), 10.0, 0.95);
}

/// Known limit (zero lookahead + speech guard): pops *quieter* than the speech peaks are caught
/// later and less often. Locked in so a regression shows; see deep dive 04.
#[test]
fn quiet_plosives_are_partly_reduced() {
    check_plosives(48_000.0, 60.0, 14, (-22.0, -16.0), 3.0, 0.7);
}

#[test]
fn removes_plosives_at_44k1_and_96k() {
    check_plosives(44_100.0, 30.0, 12, (-12.0, -4.0), 10.0, 0.95);
    check_plosives(96_000.0, 30.0, 13, (-12.0, -4.0), 10.0, 0.95);
}

/// No burst → no change (spec §7.4). Speech-like has syllable onsets and a 85–160 Hz
/// fundamental; none of it may trigger.
#[test]
fn speech_without_plosives_is_untouched() {
    for (sr, seed) in [(48_000.0, 21), (44_100.0, 22), (96_000.0, 23)] {
        let mins = if sr == 48_000.0 { 2.0 } else { 1.0 };
        let input = with_floor(speech_like(sr, 60.0 * mins, seed), seed);
        let (out, events, _) = process(&input, sr);
        let change = (energy_db(&out) - energy_db(&input)).abs();
        let per_min = events.len() as f32 / mins;
        eprintln!("MARGIN clean speech sr {sr}: change {change:.4} dB (< 0.3), {per_min:.1} events/min");
        assert!(change < 0.3, "sr {sr}: clean speech changed {change:.2} dB");
        assert!(per_min <= 1.0, "sr {sr}: {per_min:.1} false triggers/min on clean speech");
    }
}

/// Bass-heavy music with a kick every 0.5 s must not pump (≤ 2 events/min, < 1 dB).
#[test]
fn music_with_kicks_does_not_pump() {
    let sr = 48_000.0;
    let mins = 2.0;
    let input = with_floor(music_like(sr, 60.0 * mins, 31), 3);
    let (out, events, _) = process(&input, sr);
    let per_min = events.len() as f32 / mins;
    let change = (energy_db(&out) - energy_db(&input)).abs();
    let lf_change = band_energy_db(&input, sr, 0.0, 150.0) - band_energy_db(&out, sr, 0.0, 150.0);
    eprintln!("MARGIN music: {per_min:.1} events/min (≤ 2), change {change:.3} dB (< 1), LF change {lf_change:.3} dB");
    assert!(per_min <= 2.0, "{per_min:.1} events/min on music");
    assert!(change < 1.0, "music changed {change:.2} dB");
}

/// A low male voice: 90 Hz harmonic source with a strong fundamental, in syllables with
/// abrupt-ish onsets. Its LF/HF balance is high but steady, so it must never be cut.
fn low_voice(sr: f32, secs: f32, seed: u64) -> Vec<f32> {
    let mut rng = Rng::new(seed);
    let n = (sr * secs) as usize;
    let mut out = vec![0.0f32; n];
    let mut t = 0usize;
    let mut phase = 0.0f32;
    while t < n {
        let len = ms(sr, rng.range(150.0, 350.0)).min(n - t);
        let f0 = rng.range(80.0, 100.0);
        for i in 0..len {
            phase += f0 / sr;
            let env = (PI * i as f32 / len as f32).sin().powf(0.6);
            let v: f32 = (1..=20).map(|k| (2.0 * PI * k as f32 * phase).sin() / k as f32).sum();
            out[t + i] = 0.1 * env * v;
        }
        t += len + if rng.unit() < 0.4 { ms(sr, rng.range(30.0, 150.0)) } else { 0 };
    }
    out
}

#[test]
fn low_male_voice_is_not_cut() {
    let sr = 48_000.0;
    let mins = 1.0;
    let input = with_floor(low_voice(sr, 60.0 * mins, 41), 4);
    let (out, events, _) = process(&input, sr);
    let change = (energy_db(&out) - energy_db(&input)).abs();
    let per_min = events.len() as f32 / mins;
    eprintln!("MARGIN low voice: {per_min:.1} events/min (≤ 1), change {change:.4} dB (< 0.3)");
    assert!(per_min <= 1.0, "{per_min:.1} events/min on a low voice");
    assert!(change < 0.3, "low voice changed {change:.2} dB");
}

/// A steady low sine is never cut — neither under speech nor, once it has settled, alone. A
/// tone swelling in from silence looks like a pop for its first moments (zero lookahead), so
/// one timed-out event at its start is allowed; the steady part is bit-exact.
#[test]
fn sustained_low_sine_is_never_cut() {
    let sr = 48_000.0;
    let mut tone = sine(sr, 90.0, 10.0, 0.3);
    let fade = ms(sr, 200.0);
    for (i, v) in tone.iter_mut().take(fade).enumerate() {
        *v *= 0.5 - 0.5 * (PI * i as f32 / fade as f32).cos();
    }
    let alone = with_floor(tone.clone(), 5);
    let (out, events, _) = process(&alone, sr);
    let steady = ms(sr, 500.0);
    let diff = out[steady..].iter().zip(&alone[steady..]).fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
    eprintln!("MARGIN 90 Hz sine alone: {} events (≤ 1), steady max |diff| {diff:.2e}", events.len());
    assert!(events.len() <= 1, "a faded-in 90 Hz sine was cut {} times", events.len());
    assert_eq!(diff, 0.0, "idle output must be the input, bit for bit");

    let mut under = with_floor(speech_like(sr, 10.0, 51), 6);
    add_at(&mut under, &tone, 0);
    let (out, events, _) = process(&under, sr);
    let change = (energy_db(&out) - energy_db(&under)).abs();
    eprintln!("MARGIN 90 Hz sine under speech: {} events, change {change:.4} dB", events.len());
    assert!(events.is_empty(), "a 90 Hz drone under speech was cut {} times", events.len());
}

/// The first word after a long silence often starts with a pop ("Pause … Peter"): the
/// background balance must not adopt the pop within milliseconds just because the room was quiet.
#[test]
fn pop_after_long_silence_is_cut() {
    let sr = 48_000.0;
    let mut rng = Rng::new(91);
    let mut x = with_floor(vec![0.0; (sr * 30.0) as usize], 10);
    let mut pops = Vec::new();
    // Every 3 s: 2.5 s of room tone, then a pop and 0.5 s of speech starting 15 ms into it.
    for k in 0..10 {
        let start = (k as f32 * 3.0 * sr) as usize + ms(sr, 2500.0);
        let amp = 10f32.powf(rng.range(-12.0, -4.0) / 20.0);
        let p = plosive(sr, &mut rng, amp);
        add_at(&mut x, &p, start);
        add_at(&mut x, &speech_like(sr, 0.45, 100 + k), start + ms(sr, 15.0));
        pops.push(Injected { start, len: p.len() });
    }
    let (out, events, _) = process(&x, sr);
    let hits = pops.iter().filter(|p| events.iter().any(|e| matched(e, p))).count();
    let wins: Vec<(usize, usize)> = pops.iter().map(|p| (p.start, p.end())).collect();
    let reduction = lf_energy_db(&x, sr, &wins) - lf_energy_db(&out, sr, &wins);
    eprintln!("MARGIN pops after silence: {hits}/{} cut, LF −{reduction:.1} dB (≥ 10)", pops.len());
    assert!(hits >= 9, "{hits}/{} pops after silence found", pops.len());
    assert!(reduction >= 10.0, "LF reduced only {reduction:.1} dB");
}

/// A low tone starting abruptly from silence looks exactly like a pop for its first
/// milliseconds (zero lookahead). It may be cut once, but the timeout must give it back
/// within 150 ms + release, and the steady part after that is untouched.
#[test]
fn abrupt_low_tone_is_released_by_the_timeout() {
    let sr = 48_000.0;
    let x = with_floor(sine(sr, 90.0, 5.0, 0.3), 7);
    let (out, events, _) = process(&x, sr);
    let from = ms(sr, 150.0 + 60.0 + 5.0);
    let diff = out[from..].iter().zip(&x[from..]).fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
    eprintln!("MARGIN abrupt 90 Hz sine: {} events, steady max |diff| {diff:.2e}", events.len());
    assert!(events.len() <= 1, "{} events", events.len());
    assert_eq!(diff, 0.0, "the steady tone was changed after the timeout");
}

/// Identical output at every block size (all state is per sample) and a smooth cut.
#[test]
fn block_size_invariant_and_smooth() {
    let sr = 48_000.0;
    let (input, pops) = speech_with_plosives(sr, 20.0, 61, (-10.0, -4.0));
    let (reference, ref_events, _) = process_block(&input, sr, 4096);
    assert!(ref_events.len() >= pops.len() * 9 / 10);
    for block in [1, 64, 333, 512, 2048] {
        let (out, events, _) = process_block(&input, sr, block);
        let diff = out.iter().zip(&reference).fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(diff <= 1e-6, "block {block}: differs by {diff:e}");
        assert_eq!(events, ref_events, "block {block}: events differ");
    }
    // The gain path itself, probed with a constant-plus-LF signal: the cut never steps.
    assert!(max_jump(&reference) <= max_jump(&input) * 1.05 + 1e-4);
}

#[test]
fn reports_events_and_gain_reduction() {
    let sr = 48_000.0;
    let (input, pops) = speech_with_plosives(sr, 20.0, 71, (-10.0, -4.0));
    let mut chain = chain();
    chain.prepare(sr, 256, 1);
    let mut buf = input.clone();
    let (mut events, mut max_gr) = (Vec::new(), 0.0f32);
    for chunk in buf.chunks_mut(256) {
        chain.process(&mut AudioBlock::from_planar(chunk, 1));
        events.extend_from_slice(chain.telemetry().events());
        chain.telemetry_mut().clear_events();
        max_gr = max_gr.max(chain.telemetry().meters(ModuleId::DePlosive).gain_reduction_db);
    }
    let m = chain.telemetry().meters(ModuleId::DePlosive);
    assert_eq!(m.triggers as usize, events.len());
    assert!((max_gr - 24.0).abs() < 0.1, "meter peaked at {max_gr:.1} dB (max cut 24)");
    for e in &events {
        assert_eq!(e.kind, hearo_dsp::EventKind::Plosive);
        assert!(e.energy_removed_db > 0.5, "event reports {} dB removed", e.energy_removed_db);
        assert!(e.duration_samples as usize <= ms(sr, 150.0 + 60.0 + 2.0));
    }
    let hits = pops.iter().filter(|p| events.iter().any(|e| matched(e, p))).count();
    assert!(hits >= pops.len() * 9 / 10, "{hits}/{}", pops.len());
}

/// Stereo run: ch0 = `left`, ch1 = `right`, in `block`-sized chunks.
fn process_stereo(left: &[f32], right: &[f32], sr: f32, block: usize) -> (Vec<f32>, Vec<f32>, Vec<Event>) {
    let mut chain = chain();
    chain.prepare(sr, block, 2);
    let n = left.len();
    let (mut ol, mut or) = (Vec::with_capacity(n), Vec::with_capacity(n));
    let mut events = Vec::new();
    let mut buf = vec![0.0f32; 2 * block];
    for pos in (0..n).step_by(block) {
        let k = block.min(n - pos);
        buf[..k].copy_from_slice(&left[pos..pos + k]);
        buf[k..2 * k].copy_from_slice(&right[pos..pos + k]);
        chain.process(&mut AudioBlock::from_planar(&mut buf[..2 * k], 2));
        ol.extend_from_slice(&buf[..k]);
        or.extend_from_slice(&buf[k..2 * k]);
        events.extend_from_slice(chain.telemetry().events());
        chain.telemetry_mut().clear_events();
    }
    (ol, or, events)
}

/// The cut itself, measured on a probe channel: DC is fully removed by the high-pass, so with a
/// constant right channel the output is `(1 − d)·DC` — the gain curve. Pops in the left channel
/// only must cut the right one too (linked), the curve must never step faster than the 1 ms
/// attack ramp, and it must be identical at every block size.
#[test]
fn stereo_cut_is_linked_smooth_and_block_size_invariant() {
    let sr = 48_000.0;
    let (left, pops) = speech_with_plosives(sr, 20.0, 81, (-10.0, -4.0));
    let dc = 1e-3f32;
    let probe = vec![dc; left.len()];
    let settle = ms(sr, 100.0); // the high-pass's own response to the DC step
    let max_step = (1.0 - 10f32.powf(-24.0 / 20.0)) / ms(sr, 1.0) as f32 + 1e-3;
    let mut curves: Vec<Vec<f32>> = Vec::new();
    for block in [1, 64, 333, 512, 2048] {
        let (_, or, events) = process_stereo(&left, &probe, sr, block);
        let hits = pops.iter().filter(|p| events.iter().any(|e| matched(e, p))).count();
        assert!(hits >= pops.len() * 9 / 10, "block {block}: {hits}/{} one-channel pops found", pops.len());
        let g: Vec<f32> = or[settle..].iter().map(|v| v / dc).collect();
        let step = g.windows(2).fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()));
        assert!(step <= max_step, "block {block}: gain stepped {step:.4} in one sample (max {max_step:.4})");
        let deepest = g.iter().fold(1.0f32, |m, v| m.min(*v));
        assert!(deepest < 0.07, "block {block}: right channel cut only to {deepest:.3} (must be linked)");
        if block == 1 {
            eprintln!(
                "MARGIN stereo: linked cut reaches {:.1} dB, max step {step:.4} (≤ {max_step:.4})",
                20.0 * deepest.log10()
            );
        }
        curves.push(or);
    }
    for (c, block) in curves.iter().zip([1, 64, 333, 512, 2048]) {
        let diff = c.iter().zip(&curves[0]).fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(diff <= 1e-6, "block {block}: differs from block 1 by {diff:e}");
    }
}

/// Bass-heavy music: the kick pattern plus a plucked bass line (41–110 Hz) on every beat. Bass
/// notes start suddenly, but with their own harmonics, so the spectral balance doesn't jump.
#[test]
fn bass_line_does_not_pump() {
    let sr = 48_000.0;
    let mins = 2.0;
    let mut x = with_floor(music_like(sr, 60.0 * mins, 33), 8);
    let mut rng = Rng::new(9);
    let notes = [41.2, 55.0, 61.7, 73.4, 82.4, 98.0, 110.0];
    let beat = ms(sr, 500.0);
    let mut t = beat / 4;
    while t < x.len() {
        let f = notes[rng.range_usize(0, notes.len())];
        let note = pluck(sr, &mut rng, f, 0.5, 0.35);
        add_at(&mut x, &note, t);
        t += beat;
    }
    let (out, events, _) = process(&x, sr);
    let per_min = events.len() as f32 / mins;
    let change = (energy_db(&out) - energy_db(&x)).abs();
    eprintln!("MARGIN bass line: {per_min:.1} events/min (≤ 2), change {change:.3} dB (< 1)");
    assert!(per_min <= 2.0, "{per_min:.1} events/min on a bass line");
    assert!(change < 1.0, "bass line changed {change:.2} dB");
}
