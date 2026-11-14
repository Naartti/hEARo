//! `hearo procs` and `hearo run`: the live engine from the terminal (M2).

use std::io::{BufRead, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use hearo_dsp::{Module, amp_to_db, modules};
use hearo_engine::ca::procs;
use hearo_engine::{DEFAULT_EXCLUDES, EngineConfig, LiveEngine, StatsSnapshot, is_excluded};

use crate::parse_param;

#[derive(clap::Args, Debug)]
pub struct RunArgs {
    /// Comma-separated module keys in chain order (e.g. `dehum,gain`). Default: none = passthrough.
    #[arg(long, value_delimiter = ',')]
    modules: Vec<String>,
    /// Parameter override, `module.param=value` (repeatable).
    #[arg(long = "param")]
    params: Vec<String>,
    /// Extra bundle-ID prefixes to leave untouched (repeatable / comma-separated).
    #[arg(long, value_delimiter = ',')]
    exclude: Vec<String>,
    /// Don't exclude the default music apps and DAWs.
    #[arg(long)]
    no_default_excludes: bool,
    /// Stop after this many seconds (default: run until `q`).
    #[arg(long)]
    seconds: Option<f64>,
    /// Start in bypass.
    #[arg(long)]
    bypass: bool,
    /// Don't register the global § hotkey for snapshots.
    #[arg(long)]
    no_hotkey: bool,
}

pub fn procs(watch: bool, all: bool) -> Result<()> {
    loop {
        let mut list = procs::list().context("listing Core Audio processes")?;
        list.sort_by(|a, b| b.running_output.cmp(&a.running_output).then(a.bundle_id.cmp(&b.bundle_id)));
        if watch {
            print!("\x1b[2J\x1b[H");
        }
        println!("{:>7}  {:<5} {:<9} {:<46} App", "PID", "Audio", "Default", "Bundle ID");
        for p in list.iter().filter(|p| all || p.running_output || !p.bundle_id.is_empty()) {
            let excluded = is_excluded(&p.bundle_id, DEFAULT_EXCLUDES);
            println!(
                "{:>7}  {:<5} {:<9} {:<46} {}",
                p.pid,
                if p.running_output { "▶" } else { "" },
                if excluded { "excluded" } else { "processed" },
                p.bundle_id,
                procs::display_name(&p.bundle_id)
            );
        }
        let playing = list.iter().filter(|p| p.running_output).count();
        println!("\n{} processes, {playing} playing audio.", list.len());
        if !watch {
            return Ok(());
        }
        std::io::stdout().flush()?;
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn build_modules(args: &RunArgs) -> Result<Vec<Box<dyn Module>>> {
    let mut list = Vec::new();
    for key in &args.modules {
        list.push(
            modules::create(key).with_context(|| format!("unknown module '{key}' (known: {:?})", modules::KEYS))?,
        );
    }
    for p in &args.params {
        let (m, param, value) = parse_param(p)?;
        let idx =
            args.modules.iter().position(|k| *k == m).with_context(|| format!("module '{m}' not in --modules"))?;
        let pidx = list[idx]
            .params()
            .iter()
            .position(|d| d.key == param)
            .with_context(|| format!("module '{m}' has no parameter '{param}'"))?;
        list[idx].set_param(pidx, value);
    }
    Ok(list)
}

fn db(x: f32) -> String {
    if x <= 0.0 { "  -inf".into() } else { format!("{:>6.1}", amp_to_db(x)) }
}

fn status_line(s: &StatsSnapshot, prev: &StatsSnapshot, sr: f64, bypass: bool, enabled: &[(String, bool)]) -> String {
    let cbs = (s.callbacks - prev.callbacks).max(1);
    let silent = 100.0 * (s.silent_callbacks - prev.silent_callbacks) as f64 / cbs as f64;
    let budget_us = s.last_frames as f64 / sr * 1e6;
    let mods: Vec<String> = enabled
        .iter()
        .enumerate()
        .map(|(i, (k, on))| format!("{}:{}{}", i + 1, k, if *on { "" } else { "(off)" }))
        .collect();
    format!(
        "in {} dB | out {} dB | cycle {:>5.0} µs / {:.0} µs | overruns {} | silent {:>3.0}% | faults {} | {}{}",
        db(s.in_peak),
        db(s.out_peak),
        s.max_cycle_ns as f64 / 1000.0,
        budget_us,
        s.overruns,
        silent,
        s.faults,
        if bypass { "BYPASS " } else { "" },
        if mods.is_empty() { "passthrough".into() } else { mods.join(" ") },
    )
}

fn start_engine(args: &RunArgs) -> Result<LiveEngine> {
    let modules = build_modules(args)?;
    let mut exclude: Vec<String> = args.exclude.clone();
    if !args.no_default_excludes {
        exclude.extend(DEFAULT_EXCLUDES.iter().map(|s| s.to_string()));
    }
    LiveEngine::start(EngineConfig { exclude, modules, max_frames: 4096, capture_seconds: 20.0, buffer_frames: 64 })
        .context("starting the live engine")
}

pub fn run(args: &RunArgs) -> Result<()> {
    let mut engine = start_engine(args)?;
    let mut info = engine.info().clone();
    let mut controls = engine.controls().clone();
    controls.set_bypass(args.bypass);

    println!(
        "hEARo live — output: {} ({} Hz, {} ch)",
        info.output.name, info.output.sample_rate, info.output.output_channels
    );
    println!(
        "  aggregate {} Hz, buffer {} frames · added latency {:.1} ms (+ device {:.1} ms)",
        info.aggregate.sample_rate,
        info.aggregate.buffer_frames,
        info.added_latency_ms(),
        info.device_latency_ms()
    );
    let names: Vec<String> = info.excluded.iter().map(|p| procs::display_name(&p.bundle_id)).collect();
    println!("  excluded: {}", if names.is_empty() { "(none running)".into() } else { names.join(", ") });
    println!(
        "  System Audio Recording: {:?}{}",
        info.permission,
        match info.permission {
            hearo_engine::ca::permission::Permission::NotDetermined =>
                " — macOS will ask when audio starts; click Allow",
            _ => "",
        }
    );
    let hotkey = if args.no_hotkey { None } else { crate::hotkey::HotKey::register(crate::hotkey::KEY_SECTION).ok() };
    if hotkey.is_some() {
        println!("  § (anywhere, no Enter) = save snapshot \"missed it\" — the last 8 s of unprocessed audio");
    } else {
        println!("  (global § hotkey unavailable; use m + Enter)");
    }
    println!("  in this terminal (then Enter): b = bypass A/B · 1..9 = toggle module · m/f = snapshot · q = quit\n");

    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if tx.send(line.trim().to_string()).is_err() {
                break;
            }
        }
    });

    let mut enabled: Vec<(String, bool)> = args.modules.iter().map(|k| (k.clone(), true)).collect();
    let started = Instant::now();
    let mut prev = engine.stats().snapshot();
    let mut warned_silence = false;
    let (mut first_callbacks, mut first_silent) = (0u64, 0u64);
    let mut saved = 0usize;
    let mut seen_presses = crate::hotkey::presses();
    let mut last_status = Instant::now();
    loop {
        // Pump the main run loop (delivers § hotkey presses) in short slices so a press is
        // handled within ~50 ms, and refresh the status line every 0.5 s.
        crate::hotkey::pump(0.05);
        let presses = crate::hotkey::presses();
        if presses != seen_presses {
            seen_presses = presses;
            snapshot_and_confirm(&engine, "missed", &args.modules, &mut saved);
        }
        if last_status.elapsed() < Duration::from_millis(500) {
            continue;
        }
        last_status = Instant::now();
        // Follow the system output (AirPods connected, another device picked in Control Center):
        // tap + aggregate are bound to one device, so rebuild them on the new one.
        if engine.output_changed() {
            let bypass = controls.bypass();
            drop(engine); // old tap/aggregate gone before new ones are made
            std::thread::sleep(Duration::from_millis(250)); // let the device settle
            engine = start_engine(args)?;
            info = engine.info().clone();
            controls = engine.controls().clone();
            controls.set_bypass(bypass);
            for (i, (_, on)) in enabled.iter().enumerate() {
                controls.set_enabled(i, *on);
            }
            prev = engine.stats().snapshot();
            (first_callbacks, first_silent) = (0, 0);
            println!(
                "\n↪ output is now {} ({} Hz) — rebuilt, added latency {:.1} ms",
                info.output.name,
                info.output.sample_rate,
                info.added_latency_ms()
            );
            continue;
        }
        while let Ok(cmd) = rx.try_recv() {
            match cmd.as_str() {
                "q" => {
                    drop(hotkey);
                    return finish(engine, info.aggregate.sample_rate);
                }
                "b" => controls.set_bypass(!controls.bypass()),
                "m" | "f" => {
                    let label = if cmd == "m" { "missed" } else { "false_hit" };
                    snapshot_and_confirm(&engine, label, &args.modules, &mut saved);
                }
                n => {
                    if let Ok(i) = n.parse::<usize>()
                        && (1..=enabled.len()).contains(&i)
                    {
                        enabled[i - 1].1 = !enabled[i - 1].1;
                        controls.set_enabled(i - 1, enabled[i - 1].1);
                    }
                }
            }
        }
        let s = engine.stats().snapshot();
        if prev.callbacks == 0 && s.callbacks > 0 {
            (first_callbacks, first_silent) = (s.callbacks, s.silent_callbacks);
        }
        if s.callbacks == prev.callbacks {
            // The tap only runs while some processed app is playing audio.
            print!("\r\x1b[2Kidle — waiting for an app to play audio (excluded apps don't count)");
        } else {
            print!("\r\x1b[2K{}", status_line(&s, &prev, info.aggregate.sample_rate, controls.bypass(), &enabled));
        }
        std::io::stdout().flush()?;

        // A permission problem shows as digital silence while apps are playing.
        if !warned_silence
            && started.elapsed() > Duration::from_secs(2)
            && s.callbacks.saturating_sub(first_callbacks) as f64 * s.last_frames as f64
                > 3.0 * info.aggregate.sample_rate
        {
            // Sustained digital silence (≈3 s of callbacks, all silent) while apps play.
            let silent = s.silent_callbacks - first_silent == s.callbacks - first_callbacks;
            let playing = procs::list()
                .map(|l| l.iter().any(|p| p.running_output && !info.excluded.iter().any(|e| e.object == p.object)))
                .unwrap_or(false);
            if silent && playing {
                println!(
                    "\n⚠ ~3 s of pure digital silence while apps are playing. If you hear nothing, check System Audio Recording for your terminal (System Settings → Privacy & Security → Screen & System Audio Recording) and restart it."
                );
                warned_silence = true;
            }
        }
        if s.panics > 0 {
            println!("\n⚠ processing panicked {} time(s); fell back to passthrough", s.panics);
        }
        prev = s;
        if args.seconds.is_some_and(|secs| started.elapsed().as_secs_f64() >= secs) {
            drop(hotkey);
            return finish(engine, info.aggregate.sample_rate);
        }
    }
}

fn finish(engine: LiveEngine, sample_rate: f64) -> Result<()> {
    let s = engine.stats().snapshot();
    drop(engine);
    println!(
        "\nstopped: {} callbacks, {:.1} s audio, {} overruns, {} short-input, {} silent callbacks, {} faults, {} panics",
        s.callbacks,
        s.frames as f64 / sample_rate.max(1.0),
        s.overruns,
        s.short_input,
        s.silent_callbacks,
        s.faults,
        s.panics
    );
    if s.panics > 0 {
        bail!("processing panicked");
    }
    Ok(())
}

fn snapshot_and_confirm(engine: &LiveEngine, label: &str, modules: &[String], saved: &mut usize) {
    match save_snapshot(engine, label, modules) {
        Ok(path) => {
            *saved += 1;
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            // Bold green on its own line so it stands out from the status line.
            println!("\r\x1b[2K\x1b[1;32m✔ snapshot #{} saved ({label}): {name} — thanks, hero\x1b[0m", *saved);
            crate::hotkey::notify("hEARo", &format!("Snapshot #{} saved — thanks, hero", *saved));
        }
        Err(e) => {
            println!("\r\x1b[2K\x1b[1;31m✘ snapshot failed: {e:#}\x1b[0m");
            crate::hotkey::notify("hEARo", "Snapshot failed — see terminal");
        }
    }
}

fn save_snapshot(engine: &LiveEngine, label: &str, modules: &[String]) -> Result<std::path::PathBuf> {
    use hearo_engine::snapshots::{self, Context, Label};
    let label = if label == "missed" { Label::Missed } else { Label::FalseHit };
    let dir = snapshots::default_dir().context("HOME not set")?;
    let ctx = Context { modules: modules.to_vec(), source: "cli".into(), ..Context::default() };
    engine.save_snapshot(&dir, label, ctx).map_err(|e| anyhow::anyhow!("{e}"))
}
