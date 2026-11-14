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
}

impl Label {
    /// Stable string used in file names and sidecars.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Missed => "missed",
            Self::FalseHit => "false_hit",
        }
    }
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
) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
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
    let secs = left.len() as f64 / sample_rate;
    let sidecar = serde_json::json!({
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
        // The key was pressed at the end of the clip; people react ~0.5–2 s after the event.
        "press_at_s": secs,
        "event_hint_s": [(secs - 2.5).max(0.0), (secs - 0.3).max(0.0)],
        "regions": [],
    });
    std::fs::write(dir.join(format!("{stem}.json")), serde_json::to_string_pretty(&sidecar)?)?;
    Ok(wav)
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
