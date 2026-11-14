//! Property tests for the chain (§10.1): bit-exact bypass, block-size invariance, finiteness.

use hearo_dsp::modules::utility::{Gain, Lookahead};
use hearo_dsp::{AudioBlock, Chain, Event, EventKind, Module, ModuleId, TelemetryWriter};
use hearo_testkit::Rng;
use proptest::prelude::*;

/// Planar noise, `channels × frames`.
fn noise(seed: u64, channels: usize, frames: usize, amp: f32) -> Vec<f32> {
    let mut rng = Rng::new(seed);
    (0..channels * frames).map(|_| amp * rng.gaussian()).collect()
}

/// Feed `input` (planar) through the chain in the given block sizes (cycled).
fn run(chain: &mut Chain, input: &[f32], channels: usize, sizes: &[usize]) -> Vec<f32> {
    let frames = input.len() / channels;
    let mut out = vec![0.0; input.len()];
    let mut start = 0;
    let mut k = 0;
    let mut buf = vec![0.0; input.len()];
    while start < frames {
        let n = sizes[k % sizes.len()].min(frames - start);
        k += 1;
        for c in 0..channels {
            buf[c * n..(c + 1) * n].copy_from_slice(&input[c * frames + start..c * frames + start + n]);
        }
        let mut b = AudioBlock::from_planar(&mut buf[..channels * n], channels);
        chain.process(&mut b);
        for c in 0..channels {
            out[c * frames + start..c * frames + start + n].copy_from_slice(&buf[c * n..(c + 1) * n]);
        }
        start += n;
    }
    out
}

/// Like [`run`], but calls `at_sample(chain)` exactly before input sample `at` (splitting a
/// block if needed), so block-size comparisons stay aligned.
fn run_with_event(
    chain: &mut Chain,
    input: &[f32],
    channels: usize,
    sizes: &[usize],
    at: usize,
    at_sample: impl Fn(&mut Chain),
) -> Vec<f32> {
    let frames = input.len() / channels;
    let mut out = vec![0.0; input.len()];
    let mut buf = vec![0.0; input.len()];
    let (mut start, mut k, mut fired) = (0, 0, false);
    while start < frames {
        if !fired && start == at {
            at_sample(chain);
            fired = true;
        }
        let mut n = sizes[k % sizes.len()].min(frames - start);
        if !fired && start < at && start + n > at {
            n = at - start;
        } else {
            k += 1;
        }
        for c in 0..channels {
            buf[c * n..(c + 1) * n].copy_from_slice(&input[c * frames + start..c * frames + start + n]);
        }
        chain.process(&mut AudioBlock::from_planar(&mut buf[..channels * n], channels));
        for c in 0..channels {
            out[c * frames + start..c * frames + start + n].copy_from_slice(&buf[c * n..(c + 1) * n]);
        }
        start += n;
    }
    out
}

fn channel_tail(x: &[f32], channels: usize, from: usize) -> Vec<f32> {
    let frames = x.len() / channels;
    (0..channels).flat_map(|c| x[c * frames + from..(c + 1) * frames].to_vec()).collect()
}

const FADE: usize = 480;

fn delayed(input: &[f32], channels: usize, delay: usize) -> Vec<f32> {
    let frames = input.len() / channels;
    let mut out = vec![0.0; input.len()];
    for c in 0..channels {
        for i in delay..frames {
            out[c * frames + i] = input[c * frames + i - delay];
        }
    }
    out
}

fn modules(spec: &[(usize, f32)]) -> Vec<Box<dyn Module>> {
    spec.iter()
        .map(|&(lat, g)| -> Box<dyn Module> {
            if lat == 0 { Box::new(Gain::new(g)) } else { Box::new(Lookahead::new(lat, g)) }
        })
        .collect()
}

fn module_spec() -> impl Strategy<Value = Vec<(usize, f32)>> {
    prop::collection::vec((0usize..300, -12.0f32..6.0), 0..5)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn all_disabled_is_bit_exact_delay(
        spec in module_spec(),
        sizes in prop::collection::vec(1usize..1024, 1..8),
        channels in 1usize..=2,
        seed in any::<u64>(),
    ) {
        let mut chain = Chain::new(modules(&spec));
        for i in 0..chain.len() { chain.set_enabled(i, false); }
        chain.prepare(48_000.0, 512, channels);
        let input = noise(seed, channels, 6000, 0.3);
        let out = run(&mut chain, &input, channels, &sizes);
        prop_assert_eq!(out, delayed(&input, channels, chain.latency_samples()));
    }

    #[test]
    fn bypass_is_bit_exact_delay(
        spec in module_spec(),
        sizes in prop::collection::vec(1usize..1024, 1..8),
        channels in 1usize..=2,
        seed in any::<u64>(),
    ) {
        let mut chain = Chain::new(modules(&spec));
        chain.set_bypass(true);
        chain.prepare(48_000.0, 512, channels);
        let input = noise(seed, channels, 6000, 0.3);
        let out = run(&mut chain, &input, channels, &sizes);
        prop_assert_eq!(out, delayed(&input, channels, chain.latency_samples()));
    }

    #[test]
    fn block_size_invariance(
        spec in module_spec(),
        sizes in prop::collection::vec(1usize..1024, 1..8),
        channels in 1usize..=2,
        seed in any::<u64>(),
    ) {
        let input = noise(seed, channels, 8000, 0.3);
        let mut a = Chain::new(modules(&spec));
        a.prepare(48_000.0, 256, channels);
        let mut b = Chain::new(modules(&spec));
        b.prepare(48_000.0, 256, channels);
        let one_block = run(&mut a, &input, channels, &[usize::MAX]);
        let random_blocks = run(&mut b, &input, channels, &sizes);
        prop_assert_eq!(one_block, random_blocks);
    }

    #[test]
    fn block_size_invariance_with_param_change(
        spec in module_spec(),
        sizes in prop::collection::vec(1usize..1024, 1..8),
        channels in 1usize..=2,
        seed in any::<u64>(),
        at in 0usize..8000,
        gain in -24.0f32..12.0,
    ) {
        let mut spec = spec;
        spec.insert(0, (0, 0.0)); // slot 0 is always a Gain
        let input = noise(seed, channels, 8000, 0.3);
        let change = |c: &mut Chain| c.set_param(0, 0, gain);
        let mut a = Chain::new(modules(&spec));
        a.prepare(48_000.0, 256, channels);
        let mut b = Chain::new(modules(&spec));
        b.prepare(48_000.0, 256, channels);
        let one = run_with_event(&mut a, &input, channels, &[usize::MAX], at, change);
        let many = run_with_event(&mut b, &input, channels, &sizes, at, change);
        prop_assert_eq!(one, many);
    }

    #[test]
    fn disabling_midstream_ends_bit_exact(
        spec in module_spec(),
        sizes in prop::collection::vec(1usize..1024, 1..8),
        channels in 1usize..=2,
        seed in any::<u64>(),
        via_bypass in any::<bool>(),
    ) {
        let input = noise(seed, channels, 9000, 0.3);
        let mut chain = Chain::new(modules(&spec));
        chain.prepare(48_000.0, 512, channels);
        let out = run_with_event(&mut chain, &input, channels, &sizes, 2000, |c| {
            if via_bypass {
                c.set_bypass(true);
            } else {
                for i in 0..c.len() { c.set_enabled(i, false); }
            }
        });
        let latency = chain.latency_samples();
        let expect = delayed(&input, channels, latency);
        // Fade-period samples still sit in downstream delay lines for up to `latency` samples.
        let from = 2000 + FADE + latency;
        prop_assert_eq!(channel_tail(&out, channels, from), channel_tail(&expect, channels, from));
    }

    #[test]
    fn output_always_finite(
        spec in module_spec(),
        seed in any::<u64>(),
        poison in prop::sample::select(vec![f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e30, -1e38]),
        at in 0usize..2000,
    ) {
        let mut chain = Chain::new(modules(&spec));
        chain.prepare(48_000.0, 128, 1);
        let mut input = noise(seed, 1, 2000, 0.3);
        input[at] = poison;
        let out = run(&mut chain, &input, 1, &[100]);
        prop_assert!(out.iter().all(|x| x.is_finite()));
    }
}

#[test]
fn enabling_lookahead_midstream_settles_aligned() {
    use hearo_dsp::modules::utility::Lookahead;
    let mut chain = Chain::new(vec![Box::new(Lookahead::new(100, -6.0)) as Box<dyn Module>]);
    chain.set_enabled(0, false);
    chain.prepare(48_000.0, 64, 1);
    let input = noise(9, 1, 5000, 0.3);
    let out = run_with_event(&mut chain, &input, 1, &[64], 1000, |c| c.set_enabled(0, true));
    let expect: Vec<f32> = delayed(&input, 1, 100).iter().map(|x| x * 0.501_187_2).collect();
    // Before the toggle (and during warm-up) the output is the aligned dry signal.
    assert_eq!(out[..1100], delayed(&input, 1, 100)[..1100]);
    // Warm-up (100) + fade (480) + one block of slack later, fully processed and aligned.
    for i in 1000 + 100 + FADE + 64..5000 {
        assert!((out[i] - expect[i]).abs() < 1e-6, "sample {i}");
    }
}

#[test]
fn bypass_fade_lasts_exactly_10ms() {
    let mut chain = Chain::new(vec![Box::new(Gain::new(-6.0)) as Box<dyn Module>]);
    chain.prepare(48_000.0, 1024, 1);
    chain.set_bypass(true);
    let out = run(&mut chain, &[0.5f32; 1024], 1, &[1024]);
    assert!(out[FADE - 1] < 0.5, "still fading at {}", FADE - 1);
    assert_eq!(out[FADE], 0.5, "fully bypassed at exactly {FADE} samples");
}

/// Records the telemetry block start of every call.
struct StampProbe;
impl Module for StampProbe {
    fn id(&self) -> ModuleId {
        ModuleId::DeClick
    }
    fn prepare(&mut self, _: f32, _: usize, _: usize) {}
    fn reset(&mut self) {}
    fn process(&mut self, _: &mut AudioBlock<'_>, t: &mut TelemetryWriter) {
        t.push_event(Event {
            module: ModuleId::DeClick,
            kind: EventKind::Click,
            start_sample: t.block_start(),
            duration_samples: 1,
            energy_removed_db: 0.0,
        });
    }
}

#[test]
fn telemetry_block_start_tracks_sub_chunks() {
    let mut chain = Chain::new(vec![Box::new(StampProbe) as Box<dyn Module>]);
    chain.prepare(48_000.0, 256, 1);
    let _ = run(&mut chain, &[0.0f32; 1000], 1, &[1000]);
    let starts: Vec<u64> = chain.telemetry().events().iter().map(|e| e.start_sample).collect();
    assert_eq!(starts, vec![0, 256, 512, 768]);
}

#[test]
fn bypass_toggle_crossfades_back_to_processed() {
    let mut chain = Chain::new(vec![Box::new(Gain::new(-6.0))]);
    chain.prepare(48_000.0, 64, 1);
    let input = vec![0.5f32; 4800];
    chain.set_bypass(true);
    let out = run(&mut chain, &input, 1, &[64]);
    assert_eq!(*out.last().unwrap(), 0.5, "fully bypassed");
    chain.set_bypass(false);
    let out = run(&mut chain, &input, 1, &[64]);
    assert!((out.last().unwrap() - 0.5 * 0.501_187).abs() < 1e-5, "back to processed");
    assert!(out.windows(2).all(|w| w[1] <= w[0] + 1e-7), "monotonic fade, no clicks");
}
