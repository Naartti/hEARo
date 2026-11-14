//! Micro-benchmarks (divan). Budgets are enforced separately by `tests/budgets.rs`.

use divan::Bencher;
use hearo_dsp::analysis::{LogBands, StftAnalyzer};
use hearo_dsp::{AudioBlock, Chain, modules};

fn main() {
    divan::main();
}

fn full_chain(block: usize) -> Chain {
    let mut chain = Chain::new(modules::KEYS.iter().map(|k| modules::create(k).unwrap()).collect());
    chain.prepare(48_000.0, block, 2);
    chain
}

#[divan::bench(args = [64, 256, 512, 1024])]
fn chain_all_modules(bencher: Bencher, block: usize) {
    let mut chain = full_chain(block);
    let mut buf = vec![0.1f32; 2 * block];
    bencher.bench_local(|| {
        let mut b = AudioBlock::from_planar(&mut buf, 2);
        chain.process(&mut b);
    });
}

#[divan::bench(args = [1024, 2048])]
fn stft_and_bands_10ms(bencher: Bencher, size: usize) {
    let mut stft = StftAnalyzer::new(size, size / 4);
    let bands = LogBands::new(48_000.0, size, 20.0, 20_000.0, 12);
    let mut out = vec![0.0; bands.len()];
    let input = vec![0.1f32; 480];
    bencher.bench_local(|| stft.push(&input, |p| bands.compute(p, &mut out)));
}

/// One instance of each registered module, measured alone on a 512-frame stereo block.
#[divan::bench(args = modules::KEYS)]
fn module_alone(bencher: Bencher, key: &str) {
    let mut chain = Chain::new(vec![modules::create(key).unwrap()]);
    chain.prepare(48_000.0, 512, 2);
    let mut buf = vec![0.1f32; 2 * 512];
    bencher.bench_local(|| {
        let mut b = AudioBlock::from_planar(&mut buf, 2);
        chain.process(&mut b);
    });
}
