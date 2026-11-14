//! CPU budget enforcement (§2): the whole chain must use ≤ 5% of one core at 48 kHz stereo.
//!
//! Ignored by default because timing is only meaningful in an optimized, quiet build.
//! Run with `just bench` (release profile). Set HEARO_BUDGET_SCALE to loosen in CI.

#![allow(clippy::print_stdout)] // Reports timings to the developer.

use std::time::Instant;

use hearo_dsp::{AudioBlock, Chain, modules};
use hearo_testkit::signals::{SR, speech_like};

const BLOCK: usize = 512;
/// Fraction of real time the whole chain may use.
const CHAIN_BUDGET: f64 = 0.05;

fn p99_block_seconds(chain: &mut Chain, input: &[f32]) -> f64 {
    let mut buf = vec![0.0f32; 2 * BLOCK];
    let mut times = Vec::new();
    for chunk in input.as_chunks::<BLOCK>().0 {
        buf[..BLOCK].copy_from_slice(chunk);
        buf[BLOCK..].copy_from_slice(chunk);
        let t = Instant::now();
        let mut b = AudioBlock::from_planar(&mut buf, 2);
        chain.process(&mut b);
        times.push(t.elapsed().as_secs_f64());
    }
    times.sort_by(f64::total_cmp);
    times[times.len() * 99 / 100]
}

#[test]
#[ignore = "timing test; run via `just bench`"]
fn chain_within_cpu_budget() {
    let scale: f64 = std::env::var("HEARO_BUDGET_SCALE").ok().and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let input = speech_like(SR, 20.0, 1);
    let block_secs = BLOCK as f64 / SR as f64;
    let mut chain = Chain::new(modules::KEYS.iter().map(|k| modules::create(k).unwrap()).collect());
    chain.prepare(SR, BLOCK, 2);
    let p99 = p99_block_seconds(&mut chain, &input);
    let used = p99 / block_secs;
    println!("chain p99 per {BLOCK}-frame block: {:.1} µs = {:.3}% of real time", p99 * 1e6, used * 100.0);
    assert!(used <= CHAIN_BUDGET * scale, "chain uses {:.2}% > {:.2}%", used * 100.0, CHAIN_BUDGET * scale * 100.0);
}

/// Worst-case callback with dense clicks (de-click repairs: Burg + QR on the audio thread) at the
/// engine's IO buffer (64 frames at 48 kHz, scaled with the rate: 128 at 96 kHz) must stay under
/// half the callback period (rt-safety review found 766 µs vs a 667 µs budget at 96 kHz).
#[test]
#[ignore = "timing test; run via `just bench` / `just ci`"]
fn dense_clicks_fit_a_64_frame_callback() {
    use hearo_testkit::Rng;
    use hearo_testkit::signals::inject_clicks;
    for sr in [48_000.0f32, 96_000.0] {
        let mut sig = speech_like(sr, 10.0, 3);
        let mut rng = Rng::new(4);
        inject_clicks(&mut sig, sr, &mut rng, 80, (0.1, 0.4), (0.2, 1.5), 0.04);
        let frames = if sr > 60_000.0 { 128 } else { 64 };
        // The input is deterministic, so a slow callback (an algorithmic worst case) is slow at
        // the same position in every run; an OS preemption isn't. Time each callback position
        // in 3 runs and keep its fastest, then take the worst position (2026-11-14: single
        // runs failed ~1 in 3 under background load, 1457 µs vs a typical 535–587 µs).
        let mut best = vec![f64::MAX; sig.len() / frames];
        for _ in 0..3 {
            let mut chain = Chain::new(modules::KEYS.iter().map(|k| modules::create(k).unwrap()).collect());
            chain.prepare(sr, frames, 2);
            let mut buf = vec![0.0f32; 2 * frames];
            for (i, c) in sig.chunks(frames).filter(|c| c.len() == frames).enumerate() {
                buf[..frames].copy_from_slice(c);
                buf[frames..].copy_from_slice(c);
                let t = Instant::now();
                chain.process(&mut AudioBlock::from_planar(&mut buf, 2));
                best[i] = best[i].min(t.elapsed().as_secs_f64());
            }
        }
        let worst = best.iter().copied().fold(0.0f64, f64::max);
        let period = frames as f64 / sr as f64;
        println!(
            "dense clicks @ {sr} Hz: worst {frames}-frame callback {:.0} µs of {:.0} µs",
            worst * 1e6,
            period * 1e6
        );
        assert!(worst < 0.5 * period, "sr {sr}: worst callback {:.0} µs ≥ half of {:.0} µs", worst * 1e6, period * 1e6);
    }
}
