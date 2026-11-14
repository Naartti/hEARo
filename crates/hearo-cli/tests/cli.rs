//! End-to-end CLI tests: gen-signals → process → inspect.

use std::process::Command;

fn hearo() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hearo"))
}

#[test]
fn gen_process_inspect_roundtrip() {
    let dir = std::env::temp_dir().join(format!("hearo-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let st = hearo().args(["gen-signals"]).arg(&dir).args(["--secs", "1"]).status().unwrap();
    assert!(st.success());
    assert!(dir.join("manifest.json").exists());

    // Bypass must be bit-exact passthrough, even with a module that changes the signal.
    let input = dir.join("speech.wav");
    let out = dir.join("out.wav");
    let st = hearo()
        .args(["process"])
        .arg(&input)
        .arg(&out)
        .args(["--modules", "gain", "--param", "gain.gain=-6", "--bypass", "--report", "--block", "333"])
        .status()
        .unwrap();
    assert!(st.success());
    let a = hearo_testkit::wav::read(&input).unwrap();
    let b = hearo_testkit::wav::read(&out).unwrap();
    assert_eq!(a, b, "bypass output must equal input bit-for-bit");
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("out.wav.report.json")).unwrap()).unwrap();
    assert_eq!(report["latency_samples"], 0);
    assert_eq!(report["faults"], 0);
    assert_eq!(report["channels"], 1);
    let rms_before = report["before"][0]["rms_db"].as_f64().unwrap();
    assert!((rms_before + 20.0).abs() < 0.1, "speech-like is -20 dBFS RMS, got {rms_before}");

    // Gain actually applies when not bypassed.
    let st = hearo()
        .args(["process"])
        .arg(&input)
        .arg(&out)
        .args(["--modules", "gain", "--param", "gain.gain=-6"])
        .status()
        .unwrap();
    assert!(st.success());
    let b = hearo_testkit::wav::read(&out).unwrap();
    let delta = hearo_testkit::metrics::rms_db(&b.channels[0]) - hearo_testkit::metrics::rms_db(&a.channels[0]);
    assert!((delta + 6.0).abs() < 0.05, "{delta}");

    let o = hearo().args(["inspect", "--json"]).arg(&input).output().unwrap();
    assert!(o.status.success());
    assert!(String::from_utf8_lossy(&o.stdout).contains("octave_bands_db"));

    // Error paths are clear.
    for (args, msg) in [
        (vec!["--modules", "nope"], "unknown module"),
        (vec!["--block", "0"], "--block must be > 0"),
        (vec!["--param", "gain.gain=-6"], "not in --modules"),
        (vec!["--modules", "gain", "--param", "gain.nope=1"], "has no parameter"),
    ] {
        let o = hearo().args(["process"]).arg(&input).arg(&out).args(&args).output().unwrap();
        assert!(!o.status.success(), "{args:?} should fail");
        assert!(String::from_utf8_lossy(&o.stderr).contains(msg), "{args:?}: {}", String::from_utf8_lossy(&o.stderr));
    }

    // Unknown module is a clear error.
    let o = hearo().args(["process"]).arg(&input).arg(&out).args(["--modules", "nope"]).output().unwrap();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("unknown module"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn stereo_bypass_is_bit_exact_with_odd_blocks() {
    let dir = std::env::temp_dir().join(format!("hearo-cli-st-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut rng = hearo_testkit::Rng::new(4);
    let wav = hearo_testkit::wav::Wav {
        sample_rate: 44_100,
        channels: vec![
            hearo_testkit::signals::white(&mut rng, 3000, 0.2),
            hearo_testkit::signals::pink(&mut rng, 3000, 0.1),
        ],
    };
    let input = dir.join("st.wav");
    let out = dir.join("st-out.wav");
    hearo_testkit::wav::write(&input, &wav).unwrap();
    let st = hearo()
        .args(["process"])
        .arg(&input)
        .arg(&out)
        .args(["--modules", "gain", "--bypass", "--block", "7"])
        .status()
        .unwrap();
    assert!(st.success());
    assert_eq!(hearo_testkit::wav::read(&out).unwrap(), wav);
    std::fs::remove_dir_all(dir).unwrap();
}
