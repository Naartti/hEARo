//! `hearo inspect`: level and octave-band statistics.

use std::path::Path;

use anyhow::{Context, Result};
use hearo_testkit::{metrics, wav};
use serde::Serialize;

#[derive(Serialize)]
pub struct ChannelStats {
    pub rms_db: f32,
    pub peak_db: f32,
    pub octave_bands_db: Vec<(f32, f32)>,
}

impl ChannelStats {
    pub fn of(x: &[f32], sr: f32) -> Self {
        Self {
            rms_db: metrics::rms_db(x),
            peak_db: metrics::peak_db(x),
            octave_bands_db: metrics::octave_bands_db(x, sr),
        }
    }
}

#[derive(Serialize)]
struct Report {
    sample_rate: u32,
    channels: usize,
    seconds: f32,
    stats: Vec<ChannelStats>,
}

pub fn run(path: &Path, json: bool) -> Result<()> {
    let w = wav::read(path).with_context(|| format!("reading {}", path.display()))?;
    let sr = w.sample_rate as f32;
    let report = Report {
        sample_rate: w.sample_rate,
        channels: w.channels.len(),
        seconds: w.frames() as f32 / sr,
        stats: w.channels.iter().map(|c| ChannelStats::of(c, sr)).collect(),
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    println!("{}: {} Hz, {} ch, {:.2} s", path.display(), report.sample_rate, report.channels, report.seconds);
    for (i, s) in report.stats.iter().enumerate() {
        println!("ch{i}: rms {:.1} dBFS, peak {:.1} dBFS", s.rms_db, s.peak_db);
        for (f, db) in &s.octave_bands_db {
            println!("  {:>7.1} Hz {:>7.1} dB", f, db);
        }
    }
    Ok(())
}
