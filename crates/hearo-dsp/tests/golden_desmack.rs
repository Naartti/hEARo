#![allow(clippy::print_stderr)] // margins are printed for the PROGRESS log

//! Golden tests for the pause-aware lip-smack remover (human H2 feedback, 2026-11-13; spec in
//! docs/deep-dives/06-click-detection-and-repair.md). Measurable outcomes, not exact samples.

use hearo_dsp::modules::desmack::DeSmack;
use hearo_dsp::{AudioBlock, Chain, Event, Module, ModuleId};
use hearo_testkit::Rng;
use hearo_testkit::metrics::{energy_db, max_jump, rms_db};
use hearo_testkit::signals::{
    Injected, add_at, music_like, pink, speech_like, speech_with_long_smacks, speech_with_pause_smacks,
    speech_with_stop_bursts,
};

const FLOOR_DB: f32 = -55.0;

/// Run through a chain, latency-compensated (output aligned with input), collecting events.
fn process(input: &[f32], sr: f32) -> (Vec<f32>, Vec<Event>, Chain) {
    let mut chain = Chain::new(vec![Box::new(DeSmack::new()) as Box<dyn Module>]);
    chain.prepare(sr, 512, 1);
    let lat = chain.latency_samples();
    let mut buf: Vec<f32> = input.iter().copied().chain(std::iter::repeat_n(0.0, lat)).collect();
    let mut events = Vec::new();
    for chunk in buf.chunks_mut(512) {
        chain.process(&mut AudioBlock::from_planar(chunk, 1));
        events.extend_from_slice(chain.telemetry().events());
        chain.telemetry_mut().clear_events();
    }
    buf.drain(..lat);
    (buf, events, chain)
}

fn ms(sr: f32, m: f32) -> usize {
    (sr * m / 1000.0) as usize
}

fn matched(ev: &Event, s: &Injected, sr: f32) -> bool {
    let t = ev.start_sample as usize;
    t + ms(sr, 10.0) >= s.start && t <= s.end() + ms(sr, 5.0)
}

fn window_energy(x: &[f32], wins: &[(usize, usize)]) -> f32 {
    let v: Vec<f32> = wins.iter().flat_map(|&(a, b)| x[a..b.min(x.len())].iter().copied()).collect();
    energy_db(&v)
}

fn check_pause_smacks(sr: f32, secs: f32, seed: u64) {
    let (input, smacks) = speech_with_pause_smacks(sr, secs, seed, (secs * 0.7) as usize, (-22.0, -10.0), FLOOR_DB);
    let (out, events, _) = process(&input, sr);
    let smack_events: Vec<&Event> = events.iter().filter(|e| e.module == ModuleId::DeSmack).collect();

    let hits = smacks.iter().filter(|s| smack_events.iter().any(|e| matched(e, s, sr))).count();
    let recall = hits as f32 / smacks.len() as f32;
    assert!(recall >= 0.9, "sr {sr}: recall {recall:.2} ({hits}/{})", smacks.len());

    let wins: Vec<(usize, usize)> = smacks.iter().map(|s| (s.start, s.end())).collect();
    let reduction = window_energy(&input, &wins) - window_energy(&out, &wins);
    assert!(reduction >= 12.0, "sr {sr}: smack energy reduced only {reduction:.1} dB");

    // Outside the smacks (±50 ms) speech is untouched.
    let mut mask = vec![true; input.len()];
    for s in &smacks {
        let a = s.start.saturating_sub(ms(sr, 50.0));
        let b = (s.end() + ms(sr, 50.0)).min(input.len());
        mask[a..b].iter_mut().for_each(|m| *m = false);
    }
    let (inside_in, inside_out): (Vec<f32>, Vec<f32>) =
        input.iter().zip(&out).zip(&mask).filter(|(_, m)| **m).map(|((a, b), _)| (*a, *b)).unzip();
    let change = (energy_db(&inside_out) - energy_db(&inside_in)).abs();
    assert!(change < 0.2, "sr {sr}: speech outside smacks changed {change:.2} dB");
    eprintln!(
        "MARGIN sr {sr}: recall {recall:.2} ({hits}/{}), smack energy −{reduction:.1} dB, outside change {change:.3} dB",
        smacks.len()
    );

    // Ducking is smooth: no new discontinuities.
    assert!(max_jump(&out) <= max_jump(&input) * 1.05 + 1e-4, "sr {sr}: ducking clicked");
}

#[test]
fn removes_pause_smacks_at_48k() {
    check_pause_smacks(48_000.0, 60.0, 21);
}

#[test]
fn removes_pause_smacks_at_44k1_and_96k() {
    check_pause_smacks(44_100.0, 30.0, 22);
    check_pause_smacks(96_000.0, 30.0, 23);
}

fn with_floor(mut x: Vec<f32>, seed: u64) -> Vec<f32> {
    let floor = pink(&mut Rng::new(seed), x.len(), 10f32.powf(FLOOR_DB / 20.0));
    add_at(&mut x, &floor, 0);
    x
}

#[test]
fn few_false_positives_on_clean_speech() {
    let sr = 48_000.0;
    let mins = 2.0;
    let input = with_floor(speech_like(sr, 60.0 * mins, 31), 1);
    let (out, events, _) = process(&input, sr);
    let per_min = events.len() as f32 / mins;
    assert!(per_min <= 1.0, "{per_min:.1} false positives/min on clean speech");
    eprintln!("MARGIN clean speech: {per_min:.1} FP/min");
    let diff: Vec<f32> = input.iter().zip(&out).map(|(a, b)| a - b).collect();
    assert!(rms_db(&input) - rms_db(&diff) > 40.0, "clean speech altered");
}

#[test]
fn few_false_positives_on_music() {
    let sr = 48_000.0;
    let mins = 2.0;
    let input = with_floor(music_like(sr, 60.0 * mins, 32), 2);
    let (_, events, _) = process(&input, sr);
    let per_min = events.len() as f32 / mins;
    assert!(per_min <= 2.0, "{per_min:.1} false positives/min on music");
    eprintln!("MARGIN music: {per_min:.1} FP/min");
}

#[test]
fn reports_gain_reduction_and_triggers() {
    let sr = 48_000.0;
    let (input, smacks) = speech_with_pause_smacks(sr, 20.0, 41, 10, (-18.0, -12.0), FLOOR_DB);
    let (_, events, chain) = process(&input, sr);
    let m = chain.telemetry().meters(ModuleId::DeSmack);
    assert_eq!(m.triggers as usize, events.len());
    assert!(m.triggers as usize >= smacks.len() * 9 / 10);
    for e in &events {
        assert!(e.energy_removed_db > 3.0, "event reports {} dB removed", e.energy_removed_db);
        assert!(e.duration_samples as usize <= ms(sr, 45.0));
    }
}

/// Stereo run: ch0 = `left`, ch1 = `right`; returns both outputs (latency-compensated).
fn process_stereo(left: &[f32], right: &[f32], sr: f32, block: usize) -> (Vec<f32>, Vec<f32>, Vec<Event>) {
    let mut chain = Chain::new(vec![Box::new(DeSmack::new()) as Box<dyn Module>]);
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
    ol.drain(..lat);
    or.drain(..lat);
    (ol, or, events)
}

/// The duck itself, measured on a probe channel (constant input ⇒ output = gain curve):
/// identical across block sizes, and never steps faster than a 1 ms fade allows (DSP review).
#[test]
fn gain_curve_is_smooth_and_block_size_invariant() {
    let sr = 48_000.0;
    let (sig, smacks) = speech_with_pause_smacks(sr, 30.0, 51, 20, (-20.0, -10.0), FLOOR_DB);
    check_block_size_invariance(sr, &sig, &smacks, 12.0);
    // Long smacks take the provisional-duck path (and its release when speech follows).
    let (sig, smacks) = speech_with_long_smacks(sr, 30.0, 52, (-20.0, -12.0), FLOOR_DB);
    check_block_size_invariance(sr, &sig, &smacks, 3.0);
}

fn check_block_size_invariance(sr: f32, sig: &[f32], smacks: &[Injected], min_red: f32) {
    let probe = vec![1e-4f32; sig.len()];
    let wins: Vec<(usize, usize)> = smacks.iter().map(|s| (s.start, s.end())).collect();
    let max_step = 1.0 / ms(sr, 1.0) as f32 + 1e-3;
    let mut reductions = Vec::new();
    for block in [1, 64, 333, 512, 2048] {
        let (out, gain, _) = process_stereo(sig, &probe, sr, block);
        let g: Vec<f32> = gain.iter().map(|x| x / 1e-4).collect();
        let step = g.windows(2).fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()));
        assert!(step <= max_step, "block {block}: gain stepped {step:.3} in one sample (max {max_step:.3})");
        let red = window_energy(sig, &wins) - window_energy(&out, &wins);
        reductions.push(red);
    }
    let (lo, hi) = reductions.iter().fold((f32::MAX, f32::MIN), |(a, b), &r| (a.min(r), b.max(r)));
    assert!(lo >= min_red, "reductions by block size: {reductions:?}");
    assert!(hi - lo < 0.5, "reduction depends on block size: {reductions:?}");
}

/// A smack in one channel only is detected, and both channels are ducked together (linked).
#[test]
fn smack_in_one_channel_is_found_and_ducks_both() {
    let sr = 48_000.0;
    let (sig, smacks) = speech_with_pause_smacks(sr, 20.0, 52, 12, (-20.0, -12.0), FLOOR_DB);
    let (clean, _) = speech_with_pause_smacks(sr, 20.0, 52, 0, (-20.0, -12.0), FLOOR_DB);
    let (ol, or, events) = process_stereo(&sig, &clean, sr, 512);
    let hits = smacks.iter().filter(|s| events.iter().any(|e| matched(e, s, sr))).count();
    assert!(hits as f32 >= 0.9 * smacks.len() as f32, "{hits}/{} one-channel smacks found", smacks.len());
    let wins: Vec<(usize, usize)> = smacks.iter().map(|s| (s.start, s.end())).collect();
    let red_l = window_energy(&sig, &wins) - window_energy(&ol, &wins);
    let red_r = window_energy(&clean, &wins) - window_energy(&or, &wins);
    // A smack in one channel has half the mean power, so it clears the detector later and is
    // reduced less than a centred one (measured −8.6 dB); what matters is found + linked.
    assert!(red_l >= 8.0 && red_r >= 6.0, "L −{red_l:.1} dB, R −{red_r:.1} dB (duck must be linked)");
    assert!((red_l - red_r).abs() < 3.0, "channels ducked differently: L −{red_l:.1}, R −{red_r:.1} dB");
}

/// Long multi-tick smacks with a room tail (human-labelled miss, 2026-11-14). Burst + tail +
/// stay-quiet takes longer than the 40 ms lookahead, so they are ducked provisionally once the
/// tail is back at the floor. Known limit: the first ~8 ms (first tick) can play before that.
#[test]
fn long_multi_tick_smacks_are_ducked() {
    for sr in [44_100.0, 48_000.0, 96_000.0] {
        let (input, smacks) = speech_with_long_smacks(sr, 30.0, 71, (-20.0, -12.0), FLOOR_DB);
        let (out, events, _) = process(&input, sr);
        let hits = smacks.iter().filter(|s| events.iter().any(|e| matched(e, s, sr))).count();
        let recall = hits as f32 / smacks.len() as f32;
        assert!(recall >= 0.8, "sr {sr}: recall {recall:.2} ({hits}/{})", smacks.len());
        // Per detected smack: everything from 8 ms after the onset through the tail.
        let mut rest: Vec<f32> = smacks
            .iter()
            .filter(|s| events.iter().any(|e| matched(e, s, sr)))
            .map(|s| {
                let w = [(s.start + ms(sr, 8.0), s.end() + ms(sr, 20.0))];
                window_energy(&input, &w) - window_energy(&out, &w)
            })
            .collect();
        rest.sort_by(f32::total_cmp);
        let median = rest[rest.len() / 2];
        let first: Vec<(usize, usize)> = smacks.iter().map(|s| (s.start, s.start + ms(sr, 8.0))).collect();
        let first_red = window_energy(&input, &first) - window_energy(&out, &first);
        eprintln!(
            "MARGIN long smacks sr {sr}: recall {recall:.2} ({hits}/{}), after 8 ms median −{median:.1} dB (worst −{:.1}), first 8 ms −{first_red:.1} dB",
            smacks.len(),
            rest[0]
        );
        assert!(median >= 15.0, "sr {sr}: long smacks reduced only {median:.1} dB (median) after the first 8 ms");
        assert!(max_jump(&out) <= max_jump(&input) * 1.05 + 1e-4, "sr {sr}: ducking clicked");
    }
}

/// A long smack ducked provisionally, then speech starts before the stay-quiet check finishes:
/// the duck is released smoothly before the speech, and no smack event is reported.
#[test]
fn provisional_duck_releases_smoothly_when_speech_follows() {
    let sr = 48_000.0;
    let mut rng = Rng::new(9);
    let speech = speech_like(sr, 4.0, 9);
    let floor = pink(&mut rng, speech.len(), 10f32.powf(FLOOR_DB / 20.0));
    let mut sig = floor;
    // 1 s of speech, 0.5 s pause with a long smack, then speech again right after its tail.
    sig[..ms(sr, 1000.0)].iter_mut().zip(&speech).for_each(|(s, v)| *s += v);
    let (sm, ticks) = hearo_testkit::signals::multi_tick_smack(sr, &mut rng, 0.15);
    let at = ms(sr, 1300.0);
    add_at(&mut sig, &sm, at);
    // Speech resumes 60 ms after the onset: after the provisional duck starts (~start + 45 ms),
    // before the stay-quiet check could confirm.
    let resume = at + ms(sr, 60.0);
    let tail = &speech[ms(sr, 1000.0)..ms(sr, 2500.0)];
    add_at(&mut sig, tail, resume);
    let (out, events, _) = process(&sig, sr);
    let ducked = window_energy(&sig, &[(at, at + ticks)]) - window_energy(&out, &[(at, at + ticks)]);
    let after = [(resume, resume + tail.len())];
    let change = (window_energy(&sig, &after) - window_energy(&out, &after)).abs();
    eprintln!("MARGIN release: ticks −{ducked:.1} dB, speech after changed {change:.3} dB, events {}", events.len());
    assert!(change < 0.1, "speech after the released duck changed {change:.2} dB");
    assert!(max_jump(&out) <= max_jump(&sig) * 1.05 + 1e-4, "release clicked");
    assert!(ducked > 3.0, "provisional duck never engaged (−{ducked:.1} dB): the test doesn't exercise the release");
    assert_eq!(events.len(), 0, "a released provisional duck was reported as a smack");
}

/// Stop consonants ("t", "k", "p") after a closure must not be ducked (DSP review: the
/// realistic false-positive risk — a closure looks like a pause, the burst decays fast).
#[test]
fn stop_consonants_are_not_ducked() {
    let sr = 48_000.0;
    let (sig, bursts) = speech_with_stop_bursts(sr, 60.0, 61, FLOOR_DB);
    let (out, events, _) = process(&sig, sr);
    let ducked = bursts.iter().filter(|b| events.iter().any(|e| matched(e, b, sr))).count();
    let frac = ducked as f32 / bursts.len() as f32;
    let wins: Vec<(usize, usize)> = bursts.iter().map(|b| (b.start, b.end())).collect();
    let change = window_energy(&sig, &wins) - window_energy(&out, &wins);
    eprintln!(
        "MARGIN stop consonants: {ducked}/{} ducked ({:.0}%), burst energy change {change:.1} dB",
        bursts.len(),
        frac * 100.0
    );
    assert!(frac <= 0.05, "{ducked}/{} stop bursts ducked ({:.0}%)", bursts.len(), frac * 100.0);
    assert!(change < 1.0, "stop bursts lost {change:.1} dB");
}

/// Events from a mono run through one configured de-smack (latency-compensated clock).
fn events_with(input: &[f32], sr: f32, m: DeSmack) -> Vec<Event> {
    let mut chain = Chain::new(vec![Box::new(m) as Box<dyn Module>]);
    chain.prepare(sr, 512, 1);
    let lat = chain.latency_samples();
    let mut buf: Vec<f32> = input.iter().copied().chain(std::iter::repeat_n(0.0, lat)).collect();
    let mut events = Vec::new();
    for chunk in buf.chunks_mut(512) {
        chain.process(&mut AudioBlock::from_planar(chunk, 1));
        events.extend_from_slice(chain.telemetry().events());
        chain.telemetry_mut().clear_events();
    }
    events
}

fn found(events: &[Event], smacks: &[Injected], sr: f32) -> usize {
    smacks.iter().filter(|s| events.iter().any(|e| matched(e, s, sr))).count()
}

/// Real catches the human rejected (2026-11-15) were mostly bursts far quieter than the speech
/// around them; "Below speech" sets how far below the speech level a smack may peak and still be
/// ducked. Smacks 22–30 dB under the speech reference are left alone by default and found once
/// the gate is opened (so the gate, not the detector, is what rejects them). The gate is a ratio
/// of levels measured over the same span, so it is sample-rate independent; checked at all three.
#[test]
fn smacks_far_below_speech_are_left_alone() {
    for (sr, seed) in [(44_100.0, 71), (48_000.0, 72), (96_000.0, 73)] {
        // −22…−10 dBFS smacks sit 2–18 dB under this speech reference: these sit ≈ 22–30 dB under
        // (a quieter room, −75 dB, so the burst detector itself still sees them clearly).
        let (input, smacks) = speech_with_pause_smacks(sr, 30.0, seed, 20, (-34.0, -26.0), -75.0);
        let with = |db: f32| {
            let mut m = DeSmack::new();
            let key = m.params().iter().position(|p| p.key == "below_speech").expect("below_speech param");
            m.set_param(key, db);
            found(&events_with(&input, sr, m), &smacks, sr)
        };
        let (default, wide) = (found(&events_with(&input, sr, DeSmack::new()), &smacks, sr), with(48.0));
        eprintln!("MARGIN quiet smacks sr {sr}: {default}/{n} default, {wide} at 48 dB", n = smacks.len());
        assert!(default * 10 <= smacks.len(), "sr {sr}: {default}/{} quiet smacks ducked", smacks.len());
        // A control, not a recall target: quiet smacks are harder for the burst detector too.
        assert!(wide * 2 >= smacks.len(), "sr {sr}: only {wide}/{} found with the gate open", smacks.len());
    }
}

/// At 0 dB nothing quieter than the speech itself is ducked (the direction of the parameter).
#[test]
fn below_speech_zero_leaves_every_quieter_smack_alone() {
    let sr = 48_000.0;
    let (input, smacks) = speech_with_pause_smacks(sr, 30.0, 74, 20, (-22.0, -10.0), FLOOR_DB);
    let mut m = DeSmack::new();
    let key = m.params().iter().position(|p| p.key == "below_speech").expect("below_speech param");
    m.set_param(key, 0.0);
    let n = found(&events_with(&input, sr, m), &smacks, sr);
    assert!(n * 10 <= smacks.len(), "{n}/{} smacks ducked although quieter than speech", smacks.len());
}

/// Dull bursts in pauses (thumps, low mouth noise: most of their energy below 1.5 kHz) were the
/// other common false positive; a smack is bright.
#[test]
fn dull_bursts_in_pauses_are_left_alone() {
    const DULL_HF_MIX: f32 = 0.03;
    let sr = 48_000.0;
    let (mut input, gaps) = hearo_testkit::signals::speech_like_with_gaps(sr, 30.0, 72);
    let floor = pink(&mut Rng::new(73), input.len(), 10f32.powf(-80.0 / 20.0));
    add_at(&mut input, &floor, 0);
    let mut rng = Rng::new(74);
    let mut bursts = Vec::new();
    for g in gaps.iter().filter(|g| g.len >= ms(sr, 75.0) && g.start > ms(sr, 500.0)).step_by(2).take(20) {
        // A loud smack-shaped burst low-passed at 500 Hz, plus a faint bright copy so its HF still
        // jumps far above the pause floor: it triggers the detector, but most energy is dull.
        let mut lp = [hearo_dsp::filters::Biquad::lowpass(sr as f64, 500.0, 0.707); 2];
        let raw = hearo_testkit::signals::smack_len(sr, &mut rng, 0.3, 0.012);
        let dull: Vec<f32> =
            raw.iter().map(|&v| lp.iter_mut().fold(v as f64, |x, f| f.tick(x)) as f32 + DULL_HF_MIX * v).collect();
        let at = g.start + ms(sr, 20.0);
        add_at(&mut input, &dull, at);
        bursts.push(Injected { start: at, len: dull.len() });
    }
    let n = found(&events_with(&input, sr, DeSmack::new()), &bursts, sr);
    eprintln!("MARGIN dull bursts: {n}/{} ducked", bursts.len());
    assert!(n * 10 <= bursts.len(), "{n}/{} dull bursts ducked", bursts.len());
}
