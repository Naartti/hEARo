//! `hearo`: the developer CLI. Offline DSP iteration without the app (§8.3).

// Printing is this binary's job; the workspace-wide print lints target library/RT code.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

mod dataset;
mod eval;
mod gen_signals;
mod hotkey;
mod inspect;
mod live;
mod process;

#[derive(Parser, Debug)]
#[command(name = "hearo", version, about = "hEARo developer CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Generate the deterministic reference test signals (WAV + ground-truth JSON).
    GenSignals {
        /// Output directory.
        out_dir: PathBuf,
        /// Sample rate.
        #[arg(long, default_value_t = 48_000)]
        sr: u32,
        /// Length of each signal in seconds.
        #[arg(long, default_value_t = 10.0)]
        secs: f32,
        /// Random seed.
        #[arg(long, default_value_t = 1)]
        seed: u64,
    },
    /// Process a WAV file offline through the chain.
    Process(process::Args),
    /// List processes Core Audio knows about and whether they're playing audio.
    Procs {
        /// Refresh every second until Ctrl-C.
        #[arg(long)]
        watch: bool,
        /// Show every process, not just ones with a bundle ID or playing audio.
        #[arg(long)]
        all: bool,
    },
    /// Process live system audio (Core Audio tap → chain → current output device).
    Run(live::RunArgs),
    /// Labeling snapshots: stats, export to a zip, import/merge a zip from another Mac.
    #[command(subcommand)]
    Dataset(dataset::Cmd),
    /// Print level and spectrum statistics for a WAV file.
    Inspect {
        /// WAV file.
        input: PathBuf,
        /// Emit JSON instead of a table.
        #[arg(long)]
        json: bool,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::GenSignals { out_dir, sr, secs, seed } => gen_signals::run(&out_dir, sr, secs, seed),
        Command::Process(args) => process::run(&args),
        Command::Inspect { input, json } => inspect::run(&input, json),
        Command::Procs { watch, all } => live::procs(watch, all),
        Command::Dataset(cmd) => dataset::run(cmd),
        Command::Run(args) => live::run(&args),
    }
}

/// Parse `module.param=value`.
pub(crate) fn parse_param(s: &str) -> Result<(String, String, f32)> {
    let (lhs, value) = s.split_once('=').context("expected module.param=value")?;
    let (module, param) = lhs.split_once('.').context("expected module.param=value")?;
    let value: f32 = value.trim().parse().with_context(|| format!("bad number in {s}"))?;
    if !value.is_finite() {
        bail!("{s}: the value must be a finite number");
    }
    if module.is_empty() || param.is_empty() {
        bail!("expected module.param=value");
    }
    Ok((module.to_string(), param.to_string(), value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_params() {
        assert_eq!(parse_param("gain.gain=-6").unwrap(), ("gain".into(), "gain".into(), -6.0));
        assert!(parse_param("gain=-6").is_err());
        assert!(parse_param("gain.gain=x").is_err());
        // NaN/inf would reach a module and be written to a sidecar as `null`.
        assert!(parse_param("gain.gain=NaN").is_err());
        assert!(parse_param("gain.gain=inf").is_err());
    }

    #[test]
    fn cli_is_well_formed() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
