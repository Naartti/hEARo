//! `hearo dataset eval`: score the detectors against the human's reviewed snapshots.
//!
//! A *reviewed* clip is exhaustively labelled (ADR 0017): every mouth noise in it is a region,
//! so a detection outside every region is a false positive, and a region nothing hit is a miss.
//! Auto-saved clips also carry `rejected` spans — live detections the human marked as false
//! positives (hard negatives); they are reported separately so a change can be judged on them.
//!
//! Snapshots never leave this Mac (they can contain other people's voices), so this is a local
//! tuning tool, not a CI test; the scoring itself is unit-tested below.

use anyhow::{Context, Result};
use hearo_dsp::{Chain, ModuleId, modules};
use hearo_engine::snapshots;

/// A labelled or detected span, in seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub start: f64,
    pub end: f64,
}

impl Span {
    fn overlaps(&self, o: &Span, tol: f64) -> bool {
        self.start <= o.end + tol && o.start <= self.end + tol
    }
}

/// Region classes the mouth-noise detectors should catch. Others (breath, filled pause, …) are
/// neutral: a detection there is neither a hit nor a false positive.
const TARGET_CLASSES: [&str; 2] = ["lip_smack", "mouth_click"];
/// Detections this close to a span still count as on it (labels are hand-drawn).
const TOLERANCE_S: f64 = 0.02;
/// Detections before this point are the cold-started detector settling, not the live behaviour.
const WARMUP_S: f64 = 0.5;

/// What happened to one detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// On a target region.
    Hit,
    /// On a neutral region (breath, …).
    Neutral,
    /// On a span the human rejected in review (a hard negative).
    Rejected,
    /// Anywhere else in the (exhaustively labelled) clip.
    FalsePositive,
}

/// Counts for one clip or a whole set.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Score {
    pub regions: usize,
    pub hit_regions: usize,
    pub detections: usize,
    pub false_positives: usize,
    /// Subset of `false_positives` on human-rejected spans.
    pub rejected_hits: usize,
}

impl Score {
    fn add(&mut self, o: &Score) {
        self.regions += o.regions;
        self.hit_regions += o.hit_regions;
        self.detections += o.detections;
        self.false_positives += o.false_positives;
        self.rejected_hits += o.rejected_hits;
    }
}

/// Labels of one reviewed clip.
#[derive(Debug, Default, Clone)]
pub struct Labels {
    pub targets: Vec<Span>,
    pub neutral: Vec<Span>,
    pub rejected: Vec<Span>,
}

/// Classify each detection and count regions hit.
pub fn score(detections: &[Span], labels: &Labels) -> (Score, Vec<Verdict>) {
    let verdicts: Vec<Verdict> = detections
        .iter()
        .map(|d| {
            if labels.targets.iter().any(|t| d.overlaps(t, TOLERANCE_S)) {
                Verdict::Hit
            } else if labels.neutral.iter().any(|t| d.overlaps(t, TOLERANCE_S)) {
                Verdict::Neutral
            } else if labels.rejected.iter().any(|t| d.overlaps(t, TOLERANCE_S)) {
                Verdict::Rejected
            } else {
                Verdict::FalsePositive
            }
        })
        .collect();
    let hit_regions = labels.targets.iter().filter(|t| detections.iter().any(|d| d.overlaps(t, TOLERANCE_S))).count();
    let s = Score {
        regions: labels.targets.len(),
        hit_regions,
        detections: detections.len(),
        false_positives: verdicts.iter().filter(|v| matches!(v, Verdict::Rejected | Verdict::FalsePositive)).count(),
        rejected_hits: verdicts.iter().filter(|v| **v == Verdict::Rejected).count(),
    };
    (s, verdicts)
}

fn spans(v: &serde_json::Value, filter: impl Fn(&str) -> bool) -> Vec<Span> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter(|r| filter(r["class"].as_str().unwrap_or("")))
                .filter_map(|r| Some(Span { start: r["start_s"].as_f64()?, end: r["end_s"].as_f64()? }))
                .collect()
        })
        .unwrap_or_default()
}

/// Labels from a sidecar, or `None` if the clip isn't reviewed yet.
pub fn labels_from_sidecar(side: &serde_json::Value) -> Option<Labels> {
    if side["reviewed"] != true {
        return None;
    }
    Some(Labels {
        targets: spans(&side["regions"], |c| TARGET_CLASSES.contains(&c)),
        neutral: spans(&side["regions"], |c| !TARGET_CLASSES.contains(&c)),
        rejected: spans(&side["rejected"], |_| true),
    })
}

/// Run `module` (the detector being scored) over every reviewed snapshot, after the modules the
/// live chain ran before it, with `params` applied (`module.param=value`).
pub fn run(module: &str, params: &[String], verbose: bool) -> Result<()> {
    let id = ModuleId::from_key(module).with_context(|| format!("unknown module '{module}'"))?;
    let dir = snapshots::default_dir().context("HOME not set")?;
    let mut total = Score::default();
    let mut auto = Score::default();
    let mut clips = 0;
    for stem in snapshots::list(&dir) {
        let side: serde_json::Value = std::fs::read_to_string(dir.join(format!("{stem}.json")))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let Some(labels) = labels_from_sidecar(&side) else { continue };
        let wav = hearo_testkit::wav::read(dir.join(format!("{stem}.wav")))?;
        let sr = wav.sample_rate as f32;
        // The live chain up to and including the scored module (what fed it live), in chain order.
        let live: Vec<String> = side["modules"]
            .as_array()
            .map(|a| a.iter().filter_map(|m| m.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let keys: Vec<&str> = modules::KEYS
            .iter()
            .copied()
            .take_while(|k| *k != module)
            .filter(|k| live.iter().any(|m| m == k))
            .chain(std::iter::once(module))
            .collect();
        let mut chain = Chain::new(keys.iter().filter_map(|k| modules::create(k)).collect());
        chain.prepare(sr, 512, wav.channels.len());
        for p in params {
            let (m, param, value) = crate::parse_param(p)?;
            let idx = keys.iter().position(|k| *k == m).with_context(|| format!("module '{m}' not in {keys:?}"))?;
            let pidx = chain
                .module(idx)
                .context("module index out of range")?
                .params()
                .iter()
                .position(|d| d.key == param)
                .with_context(|| format!("module '{m}' has no parameter '{param}'"))?;
            chain.set_param(idx, pidx, value);
        }
        let (_, events) = crate::process::render(&mut chain, &wav, 512);
        let dets: Vec<(Span, f32)> = events
            .iter()
            .filter(|e| e.module == id)
            .map(|e| {
                let start = e.start_sample as f64 / sr as f64;
                (Span { start, end: start + e.duration_samples as f64 / sr as f64 }, e.energy_removed_db)
            })
            .filter(|(s, _)| s.start >= WARMUP_S)
            .collect();
        let spans: Vec<Span> = dets.iter().map(|d| d.0).collect();
        let (s, verdicts) = score(&spans, &labels);
        clips += 1;
        total.add(&s);
        if snapshots::is_auto(&stem) {
            auto.add(&s);
        }
        if verbose {
            println!(
                "{stem}: {}/{} regions hit, {} false positives ({} rejected)",
                s.hit_regions, s.regions, s.false_positives, s.rejected_hits
            );
            for ((d, db), v) in dets.iter().zip(&verdicts) {
                println!("    {:7.3}–{:7.3} s  {:5.1} dB  {v:?}", d.start, d.end, db);
            }
            for t in labels.targets.iter().filter(|t| !spans.iter().any(|d| d.overlaps(t, TOLERANCE_S))) {
                println!("    {:7.3}–{:7.3} s  MISSED", t.start, t.end);
            }
        }
    }
    let pct = |a: usize, b: usize| if b == 0 { 0.0 } else { 100.0 * a as f64 / b as f64 };
    println!(
        "{clips} reviewed clips: recall {}/{} ({:.0} %), {} detections, {} false positives \
         (precision {:.0} %), {} on human-rejected spans; auto-saved clips: {} false positives, {} rejected",
        total.hit_regions,
        total.regions,
        pct(total.hit_regions, total.regions),
        total.detections,
        total.false_positives,
        pct(total.detections - total.false_positives, total.detections),
        total.rejected_hits,
        auto.false_positives,
        auto.rejected_hits,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sp(start: f64, end: f64) -> Span {
        Span { start, end }
    }

    #[test]
    fn classifies_detections_and_counts_regions() {
        let labels = Labels {
            targets: vec![sp(1.0, 1.02), sp(3.0, 3.03)],
            neutral: vec![sp(5.0, 5.4)],
            rejected: vec![sp(6.0, 6.03)],
        };
        let dets = [sp(1.01, 1.03), sp(1.015, 1.02), sp(5.1, 5.12), sp(6.0, 6.02), sp(7.0, 7.01)];
        let (s, v) = score(&dets, &labels);
        assert_eq!(v, [Verdict::Hit, Verdict::Hit, Verdict::Neutral, Verdict::Rejected, Verdict::FalsePositive]);
        assert_eq!(s, Score { regions: 2, hit_regions: 1, detections: 5, false_positives: 2, rejected_hits: 1 });
    }

    #[test]
    fn tolerance_is_applied_both_ways() {
        let labels = Labels { targets: vec![sp(2.0, 2.01)], ..Default::default() };
        assert_eq!(score(&[sp(2.025, 2.03)], &labels).1, [Verdict::Hit]);
        assert_eq!(score(&[sp(1.97, 1.985)], &labels).1, [Verdict::Hit]);
        assert_eq!(score(&[sp(2.04, 2.05)], &labels).1, [Verdict::FalsePositive]);
    }

    #[test]
    fn unreviewed_clips_have_no_labels_and_classes_split() {
        let side = serde_json::json!({
            "reviewed": true,
            "regions": [
                {"class": "lip_smack", "start_s": 1.0, "end_s": 1.1},
                {"class": "breath", "start_s": 2.0, "end_s": 2.5},
                {"class": "mouth_click", "start_s": 3.0, "end_s": 3.01}
            ],
            "rejected": [{"class": "lip_smack", "start_s": 4.0, "end_s": 4.02}]
        });
        let l = labels_from_sidecar(&side).unwrap();
        assert_eq!(l.targets, [sp(1.0, 1.1), sp(3.0, 3.01)]);
        assert_eq!(l.neutral, [sp(2.0, 2.5)]);
        assert_eq!(l.rejected, [sp(4.0, 4.02)]);
        assert!(labels_from_sidecar(&serde_json::json!({"reviewed": false})).is_none());
        assert!(labels_from_sidecar(&serde_json::json!({})).is_none());
    }
}
