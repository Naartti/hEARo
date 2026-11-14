//! `hearo process`: run a WAV through the chain offline and report what changed.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use hearo_dsp::{Chain, Event, ModuleId, modules};
use hearo_testkit::wav::{self, Wav};
use serde::Serialize;

use crate::inspect::ChannelStats;
use crate::parse_param;

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Input WAV.
    input: PathBuf,
    /// Output WAV (32-bit float, latency-compensated so it lines up with the input).
    output: PathBuf,
    /// Comma-separated module keys, in chain order. Default: none (empty chain).
    #[arg(long, value_delimiter = ',')]
    modules: Vec<String>,
    /// Parameter override, `module.param=value` (repeatable).
    #[arg(long = "param")]
    params: Vec<String>,
    /// Block size used to feed the chain.
    #[arg(long, default_value_t = 512)]
    block: usize,
    /// Global bypass (output should equal input).
    #[arg(long)]
    bypass: bool,
    /// Print a Markdown report and write `<output>.report.json`.
    #[arg(long)]
    report: bool,
}

#[derive(Serialize)]
struct Report {
    input: String,
    output: String,
    modules: Vec<String>,
    sample_rate: u32,
    channels: usize,
    seconds: f32,
    latency_samples: usize,
    latency_ms: f32,
    realtime_factor: f32,
    faults: u64,
    triggers: Vec<(String, u64)>,
    events: Vec<EventOut>,
    dropped_events: u64,
    before: Vec<ChannelStats>,
    after: Vec<ChannelStats>,
}

#[derive(Serialize)]
struct EventOut {
    module: &'static str,
    kind: String,
    start_s: f64,
    duration_ms: f32,
    energy_removed_db: f32,
}

impl EventOut {
    fn from(e: &Event, sr: f32) -> Self {
        Self {
            module: e.module.key(),
            kind: format!("{:?}", e.kind),
            start_s: e.start_sample as f64 / sr as f64,
            duration_ms: e.duration_samples as f32 * 1000.0 / sr,
            energy_removed_db: e.energy_removed_db,
        }
    }
}

/// Run `input` through a prepared chain in `block`-sized pieces (shared with the app's review
/// render: [`hearo_dsp::offline::render`]). Output is latency-compensated: it lines up with the
/// input sample for sample and has the same length.
pub fn render(chain: &mut Chain, input: &Wav, block: usize) -> (Wav, Vec<Event>) {
    let r = hearo_dsp::offline::render(chain, &input.channels, block, None);
    (Wav { sample_rate: input.sample_rate, channels: r.channels }, r.events)
}

pub fn run(args: &Args) -> Result<()> {
    let input = wav::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let channels = input.channels.len();
    let sr = input.sample_rate as f32;
    let frames = input.frames();
    if channels == 0 || channels > hearo_dsp::MAX_CHANNELS {
        bail!("unsupported channel count {channels}");
    }
    if args.block == 0 {
        bail!("--block must be > 0");
    }

    let mut list = Vec::new();
    for key in &args.modules {
        list.push(
            modules::create(key).with_context(|| format!("unknown module '{key}' (known: {:?})", modules::KEYS))?,
        );
    }
    let mut chain = Chain::new(list);
    chain.set_bypass(args.bypass);
    chain.prepare(sr, args.block, channels);
    for p in &args.params {
        let (m, param, value) = parse_param(p)?;
        let idx =
            args.modules.iter().position(|k| *k == m).with_context(|| format!("module '{m}' not in --modules"))?;
        let pidx = chain
            .module(idx)
            .context("module index out of range")?
            .params()
            .iter()
            .position(|d| d.key == param)
            .with_context(|| format!("module '{m}' has no parameter '{param}'"))?;
        chain.set_param(idx, pidx, value);
    }

    let started = Instant::now();
    let (output, events) = render(&mut chain, &input, args.block);
    let elapsed = started.elapsed().as_secs_f32();
    let events: Vec<EventOut> = events.iter().map(|e| EventOut::from(e, sr)).collect();
    wav::write(&args.output, &output).with_context(|| format!("writing {}", args.output.display()))?;

    if !args.report {
        return Ok(());
    }
    let seconds = frames as f32 / sr;
    let report = Report {
        input: args.input.display().to_string(),
        output: args.output.display().to_string(),
        modules: args.modules.clone(),
        sample_rate: input.sample_rate,
        channels,
        seconds,
        latency_samples: chain.latency_samples(),
        latency_ms: chain.latency_samples() as f32 * 1000.0 / sr,
        realtime_factor: if elapsed > 0.0 { seconds / elapsed } else { f32::INFINITY },
        faults: chain.faults(),
        triggers: ModuleId::ALL
            .iter()
            .map(|id| (id.key().to_string(), chain.telemetry().meters(*id).triggers))
            .filter(|(_, n)| *n > 0)
            .collect(),
        dropped_events: chain.telemetry().dropped(),
        events,
        before: input.channels.iter().map(|c| ChannelStats::of(c, sr)).collect(),
        after: output.channels.iter().map(|c| ChannelStats::of(c, sr)).collect(),
    };
    let json_path = PathBuf::from(format!("{}.report.json", args.output.display()));
    std::fs::write(&json_path, serde_json::to_string_pretty(&report)?)?;
    print_markdown(&report);
    println!("\nJSON report: {}", json_path.display());
    Ok(())
}

fn print_markdown(r: &Report) {
    println!("## hearo process report\n");
    println!("- Input: `{}` ({} Hz, {} ch, {:.2} s)", r.input, r.sample_rate, r.channels, r.seconds);
    println!("- Modules: {}", if r.modules.is_empty() { "(none)".into() } else { r.modules.join(" → ") });
    println!("- Latency: {} samples ({:.2} ms)", r.latency_samples, r.latency_ms);
    println!("- Speed: {:.0}× real time; faults: {}", r.realtime_factor, r.faults);
    println!("- Events: {} (dropped {})", r.events.len(), r.dropped_events);
    for (m, n) in &r.triggers {
        println!("  - {m}: {n} triggers");
    }
    println!("\n| Ch | RMS before | RMS after | Peak before | Peak after |");
    println!("|---:|---:|---:|---:|---:|");
    for (i, (b, a)) in r.before.iter().zip(&r.after).enumerate() {
        println!("| {i} | {:.1} | {:.1} | {:.1} | {:.1} |", b.rms_db, a.rms_db, b.peak_db, a.peak_db);
    }
    println!("\n| Octave (Hz) | Δ dB (ch0) |");
    println!("|---:|---:|");
    if let (Some(b), Some(a)) = (r.before.first(), r.after.first()) {
        for ((f, eb), (_, ea)) in b.octave_bands_db.iter().zip(&a.octave_bands_db) {
            println!("| {f:.1} | {:+.2} |", ea - eb);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hearo_dsp::Module;
    use hearo_dsp::modules::utility::Lookahead;
    use hearo_testkit::{Rng, signals};

    #[test]
    fn render_compensates_latency_exactly() {
        let mut rng = Rng::new(3);
        let input = Wav {
            sample_rate: 48_000,
            channels: vec![signals::white(&mut rng, 5000, 0.3), signals::white(&mut rng, 5000, 0.3)],
        };
        let mut chain = Chain::new(vec![Box::new(Lookahead::new(240, 0.0)) as Box<dyn Module>]);
        chain.prepare(48_000.0, 333, 2);
        let (out, _) = render(&mut chain, &input, 333);
        assert_eq!(out, input, "a pure-latency module must render bit-exact after compensation");
    }
}
