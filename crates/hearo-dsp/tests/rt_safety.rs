//! Real-time safety (§10.2): no allocation in any processing path, plus a randomized
//! 10-minute stress test (set HEARO_STRESS_SECS to change the length).

use hearo_dsp::analysis::{LogBands, StftAnalyzer};
use hearo_dsp::modules::{self, utility::Lookahead};
use hearo_dsp::{AudioBlock, Chain, Event, EventKind, Module, ModuleId, TelemetryWriter};
use hearo_testkit::Rng;
use hearo_testkit::rt::{CountingAllocator, count_allocs};

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

/// Pushes more events per block than the telemetry buffer holds (exercises overflow).
struct EventFlood;
impl Module for EventFlood {
    fn id(&self) -> ModuleId {
        ModuleId::DeClick
    }
    fn prepare(&mut self, _: f32, _: usize, _: usize) {}
    fn reset(&mut self) {}
    fn process(&mut self, _: &mut AudioBlock<'_>, t: &mut TelemetryWriter) {
        for _ in 0..300 {
            t.push_event(Event {
                module: ModuleId::DeClick,
                kind: EventKind::Click,
                start_sample: t.block_start(),
                duration_samples: 1,
                energy_removed_db: 1.0,
            });
        }
        t.set_gain_reduction(ModuleId::DeClick, 3.0);
    }
}

/// Emits a NaN every 50th block (drives the per-slot fault path under the allocation counter).
struct Poison(u32);
impl Module for Poison {
    fn id(&self) -> ModuleId {
        ModuleId::Utility
    }
    fn prepare(&mut self, _: f32, _: usize, _: usize) {}
    fn reset(&mut self) {}
    fn process(&mut self, block: &mut AudioBlock<'_>, _: &mut TelemetryWriter) {
        self.0 += 1;
        if self.0.is_multiple_of(50) && block.frames() > 0 {
            block.channel_mut(0)[0] = f32::NAN;
        }
    }
}

fn full_chain() -> Chain {
    let mut m: Vec<Box<dyn Module>> = modules::KEYS.iter().map(|k| modules::create(k).unwrap()).collect();
    // -6 dB so toggling it is audible to the discontinuity check.
    m.push(Box::new(Lookahead::new(240, -6.0)));
    m.push(Box::new(EventFlood));
    Chain::new(m)
}

fn chain_with_poison() -> Chain {
    let mut m: Vec<Box<dyn Module>> = modules::KEYS.iter().map(|k| modules::create(k).unwrap()).collect();
    m.push(Box::new(Poison(0)));
    Chain::new(m)
}

#[test]
fn harness_detects_allocations() {
    let (_, n) = count_allocs(|| std::hint::black_box(vec![1u8; 16]));
    assert!(n >= 1, "the counting allocator must see allocations");
    let (_, n) = count_allocs(|| std::hint::black_box(1 + 1));
    assert_eq!(n, 0);
}

#[test]
fn chain_process_does_not_allocate() {
    let mut chain = full_chain();
    chain.prepare(48_000.0, 1024, 2);
    let mut buf = vec![0.1f32; 2 * 3000];
    buf[5] = f32::NAN; // exercise the fault path too
    let (_, n) = count_allocs(|| {
        // Every slot goes off and back on (Off → WarmingUp → FadingIn → On), with bypass flips.
        for round in 0..2 {
            for (i, size) in [1usize, 32, 512, 1024, 3000, 7, 0].into_iter().enumerate() {
                let mut b = AudioBlock::from_planar(&mut buf[..2 * size], 2);
                for slot in 0..chain.len() {
                    chain.set_enabled(slot, round == 1);
                }
                chain.set_bypass(i == 3);
                chain.set_param(0, 0, -3.0);
                chain.set_param(99, 0, -3.0);
                chain.set_enabled(99, true);
                chain.process(&mut b);
                chain.telemetry_mut().clear_events();
            }
        }
        // Unprepared / mismatched chains pass through without allocating.
        let mut mono = [0.1f32; 16];
        chain.process(&mut AudioBlock::from_planar(&mut mono, 1));
        chain.reset();
    });
    assert_eq!(n, 0, "Chain::process allocated");
}

#[test]
fn module_fault_path_does_not_allocate() {
    let mut chain = chain_with_poison();
    chain.prepare(48_000.0, 256, 2);
    let mut buf = vec![0.1f32; 2 * 256];
    let (_, n) = count_allocs(|| {
        for _ in 0..200 {
            chain.process(&mut AudioBlock::from_planar(&mut buf, 2));
        }
    });
    assert_eq!(n, 0, "fault path allocated");
    assert!(chain.faults() >= 3, "poison should have fired");
    assert!(buf.iter().all(|x| x.is_finite()));
}

/// Real smacks through the full chain (stereo, random block sizes): the detect → confirm →
/// duck → report paths must not allocate either. Long multi-tick smacks also take the
/// provisional-duck and release paths.
#[test]
fn smack_ducking_path_does_not_allocate() {
    use hearo_testkit::signals::{speech_with_long_smacks, speech_with_pause_smacks};
    let sr = 48_000.0;
    let signals = [
        speech_with_pause_smacks(sr, 20.0, 5, 12, (-18.0, -10.0), -55.0),
        speech_with_long_smacks(sr, 20.0, 6, (-20.0, -12.0), -55.0),
    ];
    for (sig, smacks) in &signals {
        // Only the real modules: full_chain's EventFlood fills the telemetry buffer on purpose.
        let mut chain = Chain::new(modules::KEYS.iter().map(|k| modules::create(k).unwrap()).collect());
        chain.prepare(sr, 1024, 2);
        let mut rng = Rng::new(9);
        let mut buf = vec![0.0f32; 2 * 3000];
        let mut pos = 0;
        let mut events = 0usize;
        let mut allocs = 0;
        let slot = chain.find(ModuleId::DeSmack).expect("desmack in chain");
        let below_speech = chain.module(slot).unwrap().params().iter().position(|p| p.key == "below_speech").unwrap();
        let mut step = 0u32;
        while pos < sig.len() {
            let n = rng.range_usize(1, 3000).min(sig.len() - pos);
            buf[..n].copy_from_slice(&sig[pos..pos + n]);
            buf[n..2 * n].copy_from_slice(&sig[pos..pos + n]);
            step += 1;
            let (_, a) = count_allocs(|| {
                if step.is_multiple_of(7) {
                    // "Below speech" changes mid-stream (sometimes past its 48 dB maximum).
                    chain.set_param(slot, below_speech, rng.range(17.0, 60.0));
                }
                chain.process(&mut AudioBlock::from_planar(&mut buf[..2 * n], 2));
                events += chain.telemetry().events().iter().filter(|e| e.module == ModuleId::DeSmack).count();
                chain.telemetry_mut().clear_events();
            });
            allocs += a;
            pos += n;
        }
        assert_eq!(allocs, 0, "smack path allocated");
        assert!(events >= smacks.len() * 7 / 10, "only {events} smack events for {} smacks", smacks.len());
    }
}

/// Plosive pops through the full chain (stereo, random block sizes, parameter changes while
/// the cut is engaged): detect → cut → release → report must not allocate.
#[test]
fn plosive_cut_path_does_not_allocate() {
    use hearo_testkit::signals::{add_at, plosive, speech_like};
    let sr = 48_000.0;
    let mut sig = speech_like(sr, 20.0, 6);
    let mut prng = Rng::new(7);
    for k in 0..18 {
        let p = plosive(sr, &mut prng, 0.4);
        add_at(&mut sig, &p, (sr * (k as f32 + 1.3)) as usize);
    }
    let mut chain = Chain::new(modules::KEYS.iter().map(|k| modules::create(k).unwrap()).collect());
    chain.prepare(sr, 1024, 2);
    let slot = modules::KEYS.iter().position(|k| *k == "deplosive").unwrap();
    let mut rng = Rng::new(10);
    let mut buf = vec![0.0f32; 2 * 3000];
    let (mut pos, mut events, mut allocs, mut step) = (0, 0usize, 0, 0u32);
    while pos < sig.len() {
        let n = rng.range_usize(1, 3000).min(sig.len() - pos);
        buf[..n].copy_from_slice(&sig[pos..pos + n]);
        buf[n..2 * n].copy_from_slice(&sig[pos..pos + n]);
        step += 1;
        let (_, a) = count_allocs(|| {
            if step.is_multiple_of(7) {
                // Cutoff, max cut and release all change mid-stream (sometimes mid-pop).
                let ranges = [(6.0, 14.0), (60.0, 300.0), (0.0, 40.0), (5.0, 400.0)]; // incl. out of range
                for (p, (lo, hi)) in ranges.into_iter().enumerate() {
                    chain.set_param(slot, p, rng.range(lo, hi));
                }
            }
            chain.process(&mut AudioBlock::from_planar(&mut buf[..2 * n], 2));
            events += chain.telemetry().events().iter().filter(|e| e.module == ModuleId::DePlosive).count();
            chain.telemetry_mut().clear_events();
        });
        allocs += a;
        pos += n;
    }
    assert_eq!(allocs, 0, "plosive path allocated");
    assert!(events >= 9, "only {events} plosive events for 18 pops");
}

/// Clicky input through the full chain at 64-frame blocks: the de-click repair path (Burg + QR)
/// must not allocate (rt-safety review). Its timing is checked in `budgets.rs` (release build;
/// wall-clock asserts are flaky under parallel test load).
#[test]
fn declick_repair_path_is_alloc_free() {
    use hearo_testkit::signals::{inject_clicks, speech_like};
    for sr in [48_000.0f32, 96_000.0] {
        let mut sig = speech_like(sr, 10.0, 3);
        let mut rng = Rng::new(4);
        let clicks = inject_clicks(&mut sig, sr, &mut rng, 60, (0.1, 0.4), (0.2, 1.5), 0.05);
        let mut chain = Chain::new(modules::KEYS.iter().map(|k| modules::create(k).unwrap()).collect());
        chain.prepare(sr, 64, 2);
        let mut buf = vec![0.0f32; 128];
        let (mut events, mut allocs) = (0usize, 0usize);
        for (k, c) in sig.chunks(64).enumerate() {
            let n = c.len();
            buf[..n].copy_from_slice(c);
            buf[n..2 * n].copy_from_slice(c);
            if k == 300 {
                chain.set_param(2, 0, 0.7); // a parameter change while regions may be open
            }
            let (_, a) = count_allocs(|| chain.process(&mut AudioBlock::from_planar(&mut buf[..2 * n], 2)));
            allocs += a;
            events += chain.telemetry().events().iter().filter(|e| e.module == ModuleId::DeClick).count();
            chain.telemetry_mut().clear_events();
        }
        assert_eq!(allocs, 0, "sr {sr}: repair path allocated");
        assert!(events >= clicks.len() / 2, "sr {sr}: only {events} click repairs for {} clicks", clicks.len());
    }
}

#[test]
fn analysis_does_not_allocate() {
    let mut stft = StftAnalyzer::new(2048, 512);
    let bands = LogBands::new(48_000.0, 2048, 20.0, 20_000.0, 12);
    let mut out = vec![0.0; bands.len()];
    let input = vec![0.1f32; 4096];
    let (_, n) = count_allocs(|| stft.push(&input, |p| bands.compute(p, &mut out)));
    assert_eq!(n, 0, "analysis allocated");
}

/// Randomized stress: random block sizes (32–1024), sample rates, toggles, bypass and gain
/// changes. Checks: zero allocations, finite output, no discontinuities.
#[test]
fn stress_random_blocks_and_toggles() {
    let total_secs: f32 = std::env::var("HEARO_STRESS_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(600.0);
    let mut rng = Rng::new(0x5EED);
    let mut chain = full_chain();
    let mut buf = vec![0.0f32; 2 * 1024];
    let mut simulated = 0.0f32;
    let mut allocs = 0;
    let mut processed_secs = 0.0f64;
    while simulated < total_secs {
        // A "format change" segment of 30 s.
        let sr = [44_100.0, 48_000.0, 96_000.0][rng.range_usize(0, 3)];
        chain.prepare(sr, 1024, 2);
        let seg_frames = (sr * 30.0) as usize;
        let freq = rng.range(80.0, 2000.0);
        // Sine slope bounds the input jump; allow 2x gain (+6 dB) plus ramps.
        let input_jump = 0.5 * 2.0 * std::f32::consts::PI * freq / sr;
        let bound = input_jump * 2.0 + 0.06;
        let mut t = 0usize;
        let mut prev = 0.0f32;
        let mut first = true;
        while t < seg_frames {
            let n = rng.range_usize(32, 1025).min(seg_frames - t);
            for i in 0..n {
                let s = 0.5 * (2.0 * std::f32::consts::PI * freq * (t + i) as f32 / sr).sin();
                buf[i] = s;
                buf[n + i] = s;
            }
            let toggle = rng.unit() < 0.05;
            let idx = rng.range_usize(0, chain.len());
            let on = rng.unit() < 0.5;
            let bypass = rng.unit() < 0.02;
            let gain = rng.range(-6.0, 6.0);
            let (_, a) = count_allocs(|| {
                if toggle {
                    chain.set_enabled(idx, on);
                }
                if bypass {
                    let b = chain.is_bypassed();
                    chain.set_bypass(!b);
                }
                chain.set_param(0, 0, gain);
                let mut b = AudioBlock::from_planar(&mut buf[..2 * n], 2);
                chain.process(&mut b);
                chain.telemetry_mut().clear_events();
            });
            allocs += a;
            assert!(buf[..2 * n].iter().all(|x| x.is_finite()), "non-finite output");
            // Both channels carry the same signal through the same chain: they must stay identical.
            assert_eq!(buf[..n], buf[n..2 * n], "channels diverged");
            for &x in &buf[..n] {
                if !first {
                    assert!((x - prev).abs() <= bound, "discontinuity {} > {bound} at sr {sr}", (x - prev).abs());
                }
                first = false;
                prev = x;
            }
            t += n;
            processed_secs += n as f64 / sr as f64;
        }
        simulated += 30.0;
    }
    assert!(processed_secs >= total_secs as f64 - 1e-3, "processed only {processed_secs} s");
    assert_eq!(allocs, 0, "allocations on the audio path during stress");
    assert_eq!(chain.faults(), 0);
}
