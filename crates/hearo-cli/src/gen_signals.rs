//! `hearo gen-signals`: write the reference signal set.

use std::path::Path;

use anyhow::Result;
use hearo_testkit::signals::{self, Injected};
use hearo_testkit::wav::{self, Wav};
use hearo_testkit::{Rng, metrics};
use serde::Serialize;

#[derive(Serialize)]
struct Manifest {
    sample_rate: u32,
    secs: f32,
    seed: u64,
    signals: Vec<Entry>,
}

#[derive(Serialize)]
struct Entry {
    file: String,
    description: &'static str,
    rms_db: f32,
    peak_db: f32,
    /// Ground-truth event positions, when the signal has injected events.
    events: Vec<Injected>,
}

fn sprinkle(
    base: &mut [f32],
    sr: f32,
    rng: &mut Rng,
    every_s: f32,
    make: impl Fn(&mut Rng) -> Vec<f32>,
) -> Vec<Injected> {
    let step = (every_s * sr) as usize;
    let mut out = Vec::new();
    let mut t = step / 2;
    while t + step / 2 < base.len() {
        let ev = make(rng);
        signals::add_at(base, &ev, t);
        out.push(Injected { start: t, len: ev.len() });
        t += step;
    }
    out
}

pub fn run(dir: &Path, sr_u: u32, secs: f32, seed: u64) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let sr = sr_u as f32;
    let mut rng = Rng::new(seed);
    let speech = signals::speech_like(sr, secs, seed);
    let n = speech.len();
    let mut set: Vec<(&str, &'static str, Vec<f32>, Vec<Injected>)> = Vec::new();

    set.push(("speech.wav", "Speech-like signal (glottal pulses + formants), -20 dBFS RMS", speech.clone(), vec![]));

    let mut s = speech.clone();
    let clicks = signals::inject_clicks(&mut s, sr, &mut rng, (secs * 2.0) as usize, (0.05, 0.3), (0.2, 5.0), 0.05);
    set.push(("speech_clicks.wav", "Speech-like + mouth clicks (2/s)", s, clicks));

    let mut s = speech.clone();
    let hum = signals::hum(sr, 50.0, 5, secs, 0.05);
    signals::add_at(&mut s, &hum, 0);
    set.push(("speech_hum50.wav", "Speech-like + 50 Hz hum with 5 harmonics", s, vec![]));

    let mut s = speech.clone();
    let ev = sprinkle(&mut s, sr, &mut rng, 1.0, |r| signals::sibilance(sr, r, 0.12, 0.3));
    set.push(("speech_sibilance.wav", "Speech-like + 120 ms 6-8 kHz 's' bursts (1/s)", s, ev));

    let mut s = speech.clone();
    let ev = sprinkle(&mut s, sr, &mut rng, 1.5, |r| signals::plosive(sr, r, 0.4));
    set.push(("speech_plosives.wav", "Speech-like + LF plosive pops", s, ev));

    // Smacks go into pauses over a room-tone floor, where de-smack acts (it leaves speech alone
    // by design); same generator and levels as the golden tests.
    let (s, ev) = signals::speech_with_pause_smacks(sr, secs, seed, (secs * 0.7) as usize, (-22.0, -10.0), -55.0);
    set.push(("speech_smacks.wav", "Speech-like with pauses + lip smacks in the pauses", s, ev));

    set.push((
        "music.wav",
        "Music-like: plucked strings + drums (false-positive check)",
        signals::music_like(sr, secs, seed),
        vec![],
    ));
    set.push(("pink.wav", "Pink noise, -20 dBFS RMS", signals::pink(&mut rng, n, 0.1), vec![]));
    set.push((
        "sweep.wav",
        "Log sine sweep 20 Hz-20 kHz, -6 dBFS",
        signals::log_sweep(sr, 20.0, 20_000.0, secs, 0.5),
        vec![],
    ));

    let mut manifest = Manifest { sample_rate: sr_u, secs, seed, signals: Vec::new() };
    for (file, description, samples, events) in set {
        wav::write(dir.join(file), &Wav::mono(sr_u, samples.clone()))?;
        manifest.signals.push(Entry {
            file: file.to_string(),
            description,
            rms_db: metrics::rms_db(&samples),
            peak_db: metrics::peak_db(&samples),
            events,
        });
        println!("wrote {}", dir.join(file).display());
    }
    std::fs::write(dir.join("manifest.json"), serde_json::to_string_pretty(&manifest)?)?;
    println!("wrote {}", dir.join("manifest.json").display());
    Ok(())
}
