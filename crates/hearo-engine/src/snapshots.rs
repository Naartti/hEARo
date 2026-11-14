//! Labeling snapshots (§13.5): the last seconds of unprocessed audio + a JSON sidecar in
//! `~/Library/Application Support/hEARo/snapshots/`. Local only — they may contain other
//! people's voices. Shared by the CLI and the app.

use std::path::{Path, PathBuf};

/// Seconds saved per snapshot (the event is usually 0.5–2 s before the key press).
pub const SNAPSHOT_SECS: f64 = 8.0;

/// What kind of snapshot this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Label {
    /// "Something annoying just happened that hEARo missed."
    Missed,
    /// "hEARo removed something it shouldn't have."
    FalseHit,
    /// Saved automatically around de-smack catches (opt-in, ADR 0018); unverified until reviewed.
    Auto,
}

impl Label {
    /// Stable string used in file names and sidecars.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Missed => "missed",
            Self::FalseHit => "false_hit",
            Self::Auto => "auto",
        }
    }
}

/// One de-smack catch inside an auto-saved clip, relative to the clip start.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Catch {
    /// Start, seconds.
    pub start_s: f64,
    /// End, seconds.
    pub end_s: f64,
    /// Gain reduction applied, dB.
    pub db_removed: f32,
}

/// Default snapshot directory.
pub fn default_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/hEARo/snapshots"))
}

/// A random, persistent id for this installation (`<data dir>/device-id`), recorded in every
/// snapshot so datasets merged from several Macs keep their provenance.
pub fn device_id(data_dir: &Path) -> String {
    let path = data_dir.join("device-id");
    if let Ok(id) = std::fs::read_to_string(&path) {
        let id = id.trim().to_string();
        if !id.is_empty() {
            return id;
        }
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let id = format!("{:x}{:08x}", now.as_nanos() as u64, std::process::id());
    let _ = std::fs::create_dir_all(data_dir);
    let _ = std::fs::write(&path, &id);
    id
}

/// Counts from an import.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// Snapshots copied in.
    pub added: u32,
    /// Already present (same name and size) — skipped.
    pub skipped: u32,
    /// Same name, different content — imported under a new name.
    pub renamed: u32,
}

/// Record of clips deleted in the review window, in the snapshots folder: one `<stem>\t<wav
/// bytes>` line each (`#` comments allowed). The app appends to it when it moves a clip to the
/// Trash; [`import`] skips matching clips so they don't come back from another Mac's export.
pub const DELETED_FILE: &str = "deleted.txt";

fn deleted_here(dir: &Path) -> Vec<(String, u64)> {
    let text = std::fs::read_to_string(dir.join(DELETED_FILE)).unwrap_or_default();
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let (stem, size) = l.trim().split_once('\t')?;
            Some((stem.to_owned(), size.trim().parse().ok()?))
        })
        .collect()
}

/// `snapshot-1-missed-imported2` → `snapshot-1-missed` (import renames clashing copies).
fn base_stem(stem: &str) -> &str {
    match stem.rsplit_once("-imported") {
        Some((base, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => base,
        _ => stem,
    }
}

/// A fresh temp staging folder per call (two exports/imports at once must not share one).
fn stage_dir(kind: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("hearo-{kind}-{}-{n}", std::process::id()))
}

/// Snapshot stems (`snapshot-…`) in `dir` that have a WAV.
pub fn list(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".wav").filter(|s| s.starts_with("snapshot-")).map(str::to_string)
        })
        .collect();
    v.sort();
    v
}

type DynErr = Box<dyn std::error::Error + Send + Sync>;

/// Zip every snapshot in `dir` (+ a manifest) into `out` using macOS `ditto`. Returns the count.
pub fn export(dir: &Path, out: &Path) -> Result<usize, DynErr> {
    let stems = list(dir);
    let stage = stage_dir("export");
    let root = stage.join("hearo-snapshots");
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&root)?;
    for stem in &stems {
        for ext in ["wav", "json"] {
            let src = dir.join(format!("{stem}.{ext}"));
            if src.exists() {
                std::fs::copy(&src, root.join(format!("{stem}.{ext}")))?;
            }
        }
    }
    let manifest = serde_json::json!({
        "format": "hearo-snapshots/1",
        "count": stems.len(),
        "exported_unix": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs(),
        "hearo_version": env!("CARGO_PKG_VERSION"),
        "device": dir.parent().map(device_id).unwrap_or_default(),
    });
    std::fs::write(root.join("manifest.json"), serde_json::to_string_pretty(&manifest)?)?;
    let _ = std::fs::remove_file(out);
    let st =
        std::process::Command::new("/usr/bin/ditto").args(["-c", "-k", "--keepParent"]).arg(&root).arg(out).status()?;
    let _ = std::fs::remove_dir_all(&stage);
    if !st.success() {
        return Err("ditto failed to create the archive".into());
    }
    Ok(stems.len())
}

/// Merge a snapshot archive (from [`export`]) into `dir`. Never overwrites.
pub fn import(archive: &Path, dir: &Path) -> Result<ImportReport, DynErr> {
    let stage = stage_dir("import");
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage)?;
    let st = std::process::Command::new("/usr/bin/ditto").args(["-x", "-k"]).arg(archive).arg(&stage).status()?;
    if !st.success() {
        let _ = std::fs::remove_dir_all(&stage);
        return Err("not a hEARo snapshot archive (ditto could not unpack it)".into());
    }
    let root = stage.join("hearo-snapshots");
    let src_dir = if root.is_dir() { root } else { stage.clone() };
    std::fs::create_dir_all(dir)?;
    let deleted = deleted_here(dir);
    let mut report = ImportReport::default();
    for stem in list(&src_dir) {
        let wav = src_dir.join(format!("{stem}.wav"));
        let size = std::fs::metadata(&wav)?.len();
        // Deleted on this Mac (review window): don't bring it back.
        if deleted.iter().any(|(s, n)| *n == size && base_stem(s) == base_stem(&stem)) {
            report.skipped += 1;
            continue;
        }
        // Identical copy already here (under its own name or an earlier `-importedN`)? Skip.
        let same = |name: &str| std::fs::metadata(dir.join(format!("{name}.wav"))).is_ok_and(|m| m.len() == size);
        if same(&stem)
            || (2..1000)
                .map(|n| format!("{stem}-imported{n}"))
                .take_while(|c| dir.join(format!("{c}.wav")).exists())
                .any(|c| same(&c))
        {
            report.skipped += 1;
            continue;
        }
        let target = if dir.join(format!("{stem}.wav")).exists() {
            let mut n = 2;
            while dir.join(format!("{stem}-imported{n}.wav")).exists() {
                n += 1;
            }
            report.renamed += 1;
            format!("{stem}-imported{n}")
        } else {
            report.added += 1;
            stem.clone()
        };
        std::fs::copy(&wav, dir.join(format!("{target}.wav")))?;
        let json = src_dir.join(format!("{stem}.json"));
        if json.exists() {
            std::fs::copy(&json, dir.join(format!("{target}.json")))?;
        }
    }
    let _ = std::fs::remove_dir_all(&stage);
    Ok(report)
}

/// Context recorded in the sidecar.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// Output device name.
    pub output_device: String,
    /// Module keys in the chain.
    pub modules: Vec<String>,
    /// Who saved it ("cli" / "app").
    pub source: String,
    /// Installation id ([`device_id`]); filled in by [`save`] if empty.
    pub device: String,
}

/// Write `left`/`right` (unprocessed, `sample_rate`) as `<dir>/snapshot-<unix>-<label>.wav` +
/// `.json`. Returns the WAV path.
pub fn save(
    dir: &Path,
    left: &[f32],
    right: &[f32],
    sample_rate: f64,
    label: Label,
    ctx: &Context,
) -> Result<PathBuf, DynErr> {
    let (stem, now, wav) = write_wav(dir, left, right, sample_rate, label)?;
    let mut sidecar = base_sidecar(dir, left.len() as f64 / sample_rate, sample_rate, label, now, ctx);
    let secs = left.len() as f64 / sample_rate;
    // The key was pressed at the end of the clip; people react ~0.5–2 s after the event.
    sidecar["press_at_s"] = serde_json::json!(secs);
    sidecar["event_hint_s"] = serde_json::json!([(secs - 2.5).max(0.0), (secs - 0.3).max(0.0)]);
    std::fs::write(dir.join(format!("{stem}.json")), serde_json::to_string_pretty(&sidecar)?)?;
    Ok(wav)
}

/// Write an auto-saved clip (label `auto`) with its de-smack catches as `proposals`, in the same
/// format as `hearo dataset propose`, so the review window pre-marks them. Returns the WAV path.
pub fn save_auto(
    dir: &Path,
    left: &[f32],
    right: &[f32],
    sample_rate: f64,
    ctx: &Context,
    catches: &[Catch],
) -> Result<PathBuf, DynErr> {
    let (stem, now, wav) = write_wav(dir, left, right, sample_rate, Label::Auto)?;
    let secs = left.len() as f64 / sample_rate;
    let mut sidecar = base_sidecar(dir, secs, sample_rate, Label::Auto, now, ctx);
    let ms = |s: f64| (s * 1000.0).round() / 1000.0;
    let proposals: Vec<serde_json::Value> = catches
        .iter()
        .map(|c| {
            serde_json::json!({
                "start_s": ms(c.start_s),
                "end_s": ms(c.end_s),
                "class": "lip_smack",
                "module": "desmack",
                "db_removed": (f64::from(c.db_removed) * 10.0).round() / 10.0,
                "in_press_window": true,
            })
        })
        .collect();
    let lo = catches.iter().map(|c| c.start_s).fold(f64::INFINITY, f64::min);
    let hi = catches.iter().map(|c| c.end_s).fold(0.0, f64::max);
    if lo.is_finite() {
        sidecar["event_hint_s"] = serde_json::json!([ms((lo - 0.1).max(0.0)), ms((hi + 0.1).min(secs))]);
    }
    sidecar["proposals"] = serde_json::Value::Array(proposals);
    sidecar["proposals_by"] = serde_json::json!(format!("hearo {} live [\"desmack\"]", env!("CARGO_PKG_VERSION")));
    std::fs::write(dir.join(format!("{stem}.json")), serde_json::to_string_pretty(&sidecar)?)?;
    Ok(wav)
}

/// Unix times of the auto-saved clips in `dir` (from their file names; no file reads).
/// Copies imported from another Mac (`-importedN`) aren't this Mac's saves and don't count.
pub fn auto_save_times(dir: &Path) -> Vec<u64> {
    list(dir).iter().filter(|s| !s.contains("-imported")).filter_map(|s| auto_save_time(s)).collect()
}

/// Whether a snapshot stem is an auto-saved clip (`snapshot-<unix>-auto[-n][-importedN]`).
pub fn is_auto(stem: &str) -> bool {
    auto_save_time(stem).is_some()
}

/// Delete auto-saved clips older than `max_age_days` that were never touched in the review window
/// (not reviewed, no regions, nothing rejected) — clips you worked on are kept, and so are
/// hotkey snapshots. Deletions are recorded in [`DELETED_FILE`] so an import doesn't bring them
/// back. Returns how many were deleted.
pub fn expire_auto(dir: &Path, max_age_days: u32, now_unix: u64) -> usize {
    let cutoff = now_unix.saturating_sub(u64::from(max_age_days) * 86_400);
    let mut deleted = Vec::new();
    for stem in list(dir) {
        let Some(unix) = auto_save_time(&stem) else { continue };
        if unix >= cutoff {
            continue;
        }
        let json = dir.join(format!("{stem}.json"));
        // A missing or unreadable sidecar might hold labels we can't see: keep the clip.
        let Some(side) = std::fs::read_to_string(&json)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .filter(serde_json::Value::is_object)
        else {
            continue;
        };
        let nonempty = |k: &str| side[k].as_array().is_some_and(|a| !a.is_empty());
        if side["reviewed"] != false && !side["reviewed"].is_null() || nonempty("regions") || nonempty("rejected") {
            continue;
        }
        let wav = dir.join(format!("{stem}.wav"));
        let size = std::fs::metadata(&wav).map(|m| m.len()).unwrap_or(0);
        if std::fs::remove_file(&wav).is_ok() {
            let _ = std::fs::remove_file(&json);
            deleted.push(format!("{stem}\t{size}\n"));
        }
    }
    if !deleted.is_empty() {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join(DELETED_FILE)) {
            let _ = f.write_all(deleted.concat().as_bytes());
        }
    }
    deleted.len()
}

/// `snapshot-<unix>-auto[-n][-importedN]` → unix.
fn auto_save_time(stem: &str) -> Option<u64> {
    let (unix, label) = stem.strip_prefix("snapshot-")?.split_once('-')?;
    (label == "auto" || label.starts_with("auto-")).then(|| unix.parse().ok())?
}

/// Write the WAV under a fresh `snapshot-<unix>-<label>[-n]` stem.
fn write_wav(
    dir: &Path,
    left: &[f32],
    right: &[f32],
    sample_rate: f64,
    label: Label,
) -> Result<(String, std::time::Duration, PathBuf), DynErr> {
    std::fs::create_dir_all(dir)?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?;
    let mut stem = format!("snapshot-{}-{}", now.as_secs(), label.as_str());
    // Two presses within a second must not overwrite each other.
    let mut n = 1;
    while dir.join(format!("{stem}.wav")).exists() {
        n += 1;
        stem = format!("snapshot-{}-{}-{n}", now.as_secs(), label.as_str());
    }
    let wav = dir.join(format!("{stem}.wav"));
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: sample_rate.round() as u32,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(&wav, spec)?;
    for (l, r) in left.iter().zip(right) {
        w.write_sample(*l)?;
        w.write_sample(*r)?;
    }
    w.finalize()?;
    Ok((stem, now, wav))
}

/// The sidecar keys every snapshot has.
fn base_sidecar(
    dir: &Path,
    secs: f64,
    sample_rate: f64,
    label: Label,
    now: std::time::Duration,
    ctx: &Context,
) -> serde_json::Value {
    serde_json::json!({
        "label": label.as_str(),
        "unix_time": now.as_secs(),
        "seconds": secs,
        "sample_rate": sample_rate,
        "unprocessed": true,
        "output_device": ctx.output_device,
        "modules": ctx.modules,
        "source": ctx.source,
        "device": if ctx.device.is_empty() { dir.parent().map(device_id).unwrap_or_default() } else { ctx.device.clone() },
        "hearo_version": env!("CARGO_PKG_VERSION"),
        "regions": [],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_wav_and_sidecar_without_clobbering() {
        let dir = std::env::temp_dir().join(format!("hearo-snap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ctx = Context {
            output_device: "Test".into(),
            modules: vec!["gain".into()],
            source: "test".into(),
            device: "d1".into(),
        };
        let a = save(&dir, &[0.1; 480], &[0.2; 480], 48_000.0, Label::Missed, &ctx).unwrap();
        let b = save(&dir, &[0.1; 480], &[0.2; 480], 48_000.0, Label::Missed, &ctx).unwrap();
        assert_ne!(a, b);
        let r = hound::WavReader::open(&a).unwrap();
        assert_eq!((r.spec().channels, r.spec().sample_rate, r.len()), (2, 48_000, 960));
        let side: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(a.with_extension("json")).unwrap()).unwrap();
        assert_eq!(side["label"], "missed");
        assert!((side["press_at_s"].as_f64().unwrap() - 0.01).abs() < 1e-9);
        assert_eq!(side["device"], "d1");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn auto_clip_carries_its_catches_as_proposals() {
        let dir = std::env::temp_dir().join(format!("hearo-auto-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let catches = [Catch { start_s: 4.98, end_s: 5.0004, db_removed: 17.96 }];
        let wav = save_auto(&dir, &[0.0; 4800], &[0.0; 4800], 48_000.0, &Context::default(), &catches).unwrap();
        assert!(wav.file_name().unwrap().to_string_lossy().contains("-auto"));
        let side: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(wav.with_extension("json")).unwrap()).unwrap();
        assert_eq!(side["label"], "auto");
        assert!(side["press_at_s"].is_null(), "nobody pressed a key");
        let p = &side["proposals"][0];
        assert_eq!((p["start_s"].as_f64(), p["end_s"].as_f64()), (Some(4.98), Some(5.0)));
        assert_eq!((p["class"].as_str(), p["module"].as_str()), (Some("lip_smack"), Some("desmack")));
        assert_eq!(p["db_removed"].as_f64(), Some(18.0));
        assert_eq!(side["event_hint_s"][0].as_f64(), Some(4.88));
        assert_eq!(side["regions"].as_array().map(Vec::len), Some(0));
        assert_eq!(auto_save_times(&dir).len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Only old, untouched auto clips go; reviewed or edited ones and hotkey snapshots stay.
    #[test]
    fn expiry_deletes_only_old_untouched_auto_clips() {
        let dir = std::env::temp_dir().join(format!("hearo-expire-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let day = 86_400;
        let now = 100 * day;
        let make = |stem: &str, extra: serde_json::Value| {
            std::fs::write(dir.join(format!("{stem}.wav")), b"RIFF").unwrap();
            let mut side = serde_json::json!({"label": "auto", "regions": [], "proposals": [{}]});
            if let (Some(s), Some(e)) = (side.as_object_mut(), extra.as_object()) {
                s.extend(e.clone());
            }
            std::fs::write(dir.join(format!("{stem}.json")), side.to_string()).unwrap();
        };
        let old = now - 40 * day;
        make(&format!("snapshot-{old}-auto"), serde_json::json!({}));
        make(&format!("snapshot-{old}-auto-2"), serde_json::json!({}));
        make(&format!("snapshot-{}-auto", old + 1), serde_json::json!({"reviewed": true}));
        make(&format!("snapshot-{}-auto", old + 2), serde_json::json!({"rejected": [{"start_s": 1.0}]}));
        make(&format!("snapshot-{}-auto", old + 3), serde_json::json!({"regions": [{"start_s": 1.0}]}));
        make(&format!("snapshot-{}-auto", now - 10 * day), serde_json::json!({}));
        make(&format!("snapshot-{old}-missed"), serde_json::json!({"label": "missed"}));
        assert_eq!(expire_auto(&dir, 31, now), 2);
        let left = list(&dir);
        assert_eq!(left.len(), 5, "{left:?}");
        assert!(!left.contains(&format!("snapshot-{old}-auto")));
        let deleted = std::fs::read_to_string(dir.join(DELETED_FILE)).unwrap();
        assert_eq!(deleted.lines().count(), 2, "recorded so an import won't resurrect them");
        assert_eq!(expire_auto(&dir, 31, now), 0, "idempotent");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn auto_stems_and_cap_seeding() {
        for (stem, auto) in [
            ("snapshot-5-auto", true),
            ("snapshot-5-auto-2", true),
            ("snapshot-5-auto-imported3", true),
            ("snapshot-5-automatic", false),
            ("snapshot-x-auto", false),
            ("snapshot-5-missed", false),
        ] {
            assert_eq!(is_auto(stem), auto, "{stem}");
        }
        let dir = std::env::temp_dir().join(format!("hearo-seed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for s in ["snapshot-5-auto", "snapshot-6-auto-2", "snapshot-7-auto-imported2", "snapshot-8-missed"] {
            std::fs::write(dir.join(format!("{s}.wav")), b"x").unwrap();
        }
        let mut t = auto_save_times(&dir);
        t.sort_unstable();
        assert_eq!(t, vec![5, 6], "imported copies don't use up this Mac's daily limit");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A clip whose sidecar is missing, corrupt or not an object might hold labels: kept.
    #[test]
    fn expiry_keeps_clips_with_unreadable_sidecars() {
        let dir = std::env::temp_dir().join(format!("hearo-expire-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (i, json) in [None, Some("{"), Some("[]"), Some(r#"{"reviewed":"yes"}"#)].iter().enumerate() {
            let stem = format!("snapshot-{}-auto", 100 + i);
            std::fs::write(dir.join(format!("{stem}.wav")), b"RIFF").unwrap();
            if let Some(j) = json {
                std::fs::write(dir.join(format!("{stem}.json")), j).unwrap();
            }
        }
        assert_eq!(expire_auto(&dir, 1, 100 * 86_400), 0);
        assert_eq!(list(&dir).len(), 4);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn device_id_is_stable() {
        let dir = std::env::temp_dir().join(format!("hearo-dev-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let a = device_id(&dir);
        assert!(!a.is_empty());
        assert_eq!(device_id(&dir), a);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn export_import_merges_without_overwriting() {
        let base = std::env::temp_dir().join(format!("hearo-xfer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (a, b) = (base.join("macA/snapshots"), base.join("macB/snapshots"));
        let ctx = Context::default();
        save(&a, &[0.1; 480], &[0.1; 480], 48_000.0, Label::Missed, &ctx).unwrap();
        save(&a, &[0.2; 960], &[0.2; 960], 48_000.0, Label::FalseHit, &ctx).unwrap();
        // Mac B already has one of them (identical) and a different file with a clashing name.
        std::fs::create_dir_all(&b).unwrap();
        let stems = list(&a);
        std::fs::copy(a.join(format!("{}.wav", stems[0])), b.join(format!("{}.wav", stems[0]))).unwrap();
        std::fs::write(b.join(format!("{}.wav", stems[1])), b"different").unwrap();
        let zip = base.join("export.zip");
        assert_eq!(export(&a, &zip).unwrap(), 2);
        let r = import(&zip, &b).unwrap();
        assert_eq!(r, ImportReport { added: 0, skipped: 1, renamed: 1 });
        assert_eq!(list(&b).len(), 3, "nothing overwritten: {:?}", list(&b));
        // Importing again changes nothing.
        let r = import(&zip, &b).unwrap();
        assert_eq!((r.added, r.skipped), (0, 2));
        assert!(import(&base.join("missing.zip"), &b).is_err());
        std::fs::remove_dir_all(base).unwrap();
    }

    /// A clip deleted in the review window (moved to the Trash and recorded in `deleted.txt`)
    /// must not come back when the other Mac's export is imported again.
    #[test]
    fn import_skips_clips_deleted_here() {
        let base = std::env::temp_dir().join(format!("hearo-del-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (a, b) = (base.join("macA"), base.join("macB"));
        let ctx = Context::default();
        save(&a, &[0.1; 480], &[0.1; 480], 48_000.0, Label::Missed, &ctx).unwrap();
        save(&a, &[0.2; 960], &[0.2; 960], 48_000.0, Label::Missed, &ctx).unwrap();
        let zip = base.join("export.zip");
        export(&a, &zip).unwrap();
        assert_eq!(import(&zip, &b).unwrap().added, 2);
        // Delete one on Mac B the way the app does: remove the files, record name + size.
        let gone = list(&b)[0].clone();
        let size = std::fs::metadata(b.join(format!("{gone}.wav"))).unwrap().len();
        std::fs::remove_file(b.join(format!("{gone}.wav"))).unwrap();
        std::fs::remove_file(b.join(format!("{gone}.json"))).unwrap();
        std::fs::write(b.join(DELETED_FILE), format!("# comment\n{gone}\t{size}\n")).unwrap();
        let r = import(&zip, &b).unwrap();
        assert_eq!(r, ImportReport { added: 0, skipped: 2, renamed: 0 });
        assert!(!list(&b).contains(&gone), "deleted clip came back");
        // A renamed copy (`-imported2`) of a deleted clip is matched by its base name too.
        std::fs::write(b.join(DELETED_FILE), format!("{gone}-imported2\t{size}\n")).unwrap();
        assert_eq!(import(&zip, &b).unwrap().added, 0);
        std::fs::remove_dir_all(base).unwrap();
    }
}
