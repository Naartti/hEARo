//! `hearo dataset …`: snapshot statistics and moving snapshots between Macs.

use std::path::PathBuf;

use anyhow::{Context, Result};
use hearo_engine::snapshots;

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// Count snapshots by label and device.
    Stats,
    /// Zip all snapshots (+ manifest) into a file.
    Export {
        /// Output .zip.
        out: PathBuf,
    },
    /// Merge a snapshot zip (from another Mac) into this Mac's snapshots. Never overwrites.
    Import {
        /// Input .zip.
        archive: PathBuf,
    },
    /// Run the detectors offline over every snapshot and store where they think the annoying
    /// sound is as `proposals` in each sidecar (your own `regions` are never touched). The review
    /// window pre-marks these; training can use them as weak labels. Auto-saved clips are skipped
    /// (they already have the live detector's proposals).
    Propose {
        /// Only print, don't write sidecars.
        #[arg(long)]
        dry_run: bool,
        /// Detectors to run (comma-separated). Default: de-smack only — what the human labels;
        /// all modules buried the smacks under ~300 de-ess/de-click proposals (2026-11-14).
        #[arg(long, value_delimiter = ',', default_value = "desmack")]
        modules: Vec<String>,
    },
}

pub fn run(cmd: Cmd) -> Result<()> {
    let dir = snapshots::default_dir().context("HOME not set")?;
    match cmd {
        Cmd::Stats => {
            let stems = snapshots::list(&dir);
            let mut by: std::collections::BTreeMap<(String, String), usize> = Default::default();
            for s in &stems {
                let side: serde_json::Value = std::fs::read_to_string(dir.join(format!("{s}.json")))
                    .ok()
                    .and_then(|t| serde_json::from_str(&t).ok())
                    .unwrap_or_default();
                let label = side["label"].as_str().unwrap_or("?").to_string();
                let device = side["device"].as_str().unwrap_or("(this Mac, before device ids)").to_string();
                *by.entry((device, label)).or_default() += 1;
            }
            println!("{} snapshots in {}", stems.len(), dir.display());
            for ((device, label), n) in by {
                println!("  {n:>4}  {label:<10} device {device}");
            }
        }
        Cmd::Export { out } => {
            let n = snapshots::export(&dir, &out).map_err(|e| anyhow::anyhow!("{e}"))?;
            println!("✔ exported {n} snapshots to {}", out.display());
        }
        Cmd::Propose { dry_run, modules } => propose(&dir, dry_run, &modules)?,
        Cmd::Import { archive } => {
            let r = snapshots::import(&archive, &dir).map_err(|e| anyhow::anyhow!("{e}"))?;
            println!(
                "✔ imported: {} added, {} already present, {} renamed (name clash)",
                r.added, r.skipped, r.renamed
            );
        }
    }
    Ok(())
}

fn propose(dir: &std::path::Path, dry_run: bool, keys: &[String]) -> Result<()> {
    use hearo_dsp::{Chain, modules};
    let (mut total, mut in_window, mut clips_hit) = (0usize, 0usize, 0usize);
    let stems = snapshots::list(dir);
    for stem in &stems {
        // Auto-saved clips already carry the live detector's proposals, and the review window's
        // accept/false-positive decisions match their exact spans: never overwrite them.
        if stem.split('-').nth(2) == Some("auto") {
            continue;
        }
        let wav = hearo_testkit::wav::read(dir.join(format!("{stem}.wav")))?;
        let sr = wav.sample_rate as f32;
        // Detectors only (no repair needed): run the user-facing modules and collect events.
        // In chain order, whatever order they were given in.
        let mods: Vec<_> =
            modules::KEYS.iter().filter(|k| keys.iter().any(|m| m == *k)).filter_map(|k| modules::create(k)).collect();
        anyhow::ensure!(!mods.is_empty(), "no known modules in {keys:?} (known: {:?})", modules::KEYS);
        let mut chain = Chain::new(mods);
        chain.prepare(sr, 512, wav.channels.len());
        let (_, events) = crate::process::render(&mut chain, &wav, 512);
        let secs = wav.frames() as f64 / sr as f64;
        let props: Vec<serde_json::Value> = events
            .iter()
            .map(|e| {
                let start = e.start_sample as f64 / sr as f64;
                let end = start + e.duration_samples as f64 / sr as f64;
                let before_press = secs - start;
                serde_json::json!({
                    "start_s": (start * 1000.0).round() / 1000.0,
                    "end_s": (end * 1000.0).round() / 1000.0,
                    "class": match e.kind {
                        hearo_dsp::EventKind::Smack => "lip_smack",
                        hearo_dsp::EventKind::Click => "mouth_click",
                        hearo_dsp::EventKind::Plosive => "plosive",
                        hearo_dsp::EventKind::Sibilance => "sibilance",
                        _ => "other",
                    },
                    "module": e.module.key(),
                    "db_removed": (e.energy_removed_db * 10.0).round() / 10.0,
                    "in_press_window": (0.3..=2.5).contains(&before_press),
                })
            })
            .collect();
        let hits = props.iter().filter(|p| p["in_press_window"] == true).count();
        total += props.len();
        in_window += hits;
        clips_hit += usize::from(hits > 0);
        println!("{stem}: {} proposals ({hits} in the press window)", props.len());
        if dry_run {
            continue;
        }
        let side_path = dir.join(format!("{stem}.json"));
        let mut side: serde_json::Value = std::fs::read_to_string(&side_path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        side["proposals"] = serde_json::Value::Array(props);
        side["proposals_by"] = serde_json::json!(format!("hearo {} {:?}", env!("CARGO_PKG_VERSION"), keys));
        std::fs::write(&side_path, serde_json::to_string_pretty(&side)?)?;
    }
    println!(
        "\n{} clips: {total} proposals, {in_window} in the 0.3–2.5 s window before the press; {clips_hit}/{} clips have one there",
        stems.len(),
        stems.len()
    );
    Ok(())
}
