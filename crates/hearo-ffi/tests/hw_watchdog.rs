//! Hardware test (plays a quiet tone through the default output with `afplay`): while a tapped
//! app plays, the IOProc gets callbacks and the watchdog leaves the healthy engine alone.
//! Run with `just test-hw` (quit hEARo first: two taps would tap each other).

#![allow(clippy::print_stderr)] // reports what the hardware did

use std::process::Command;
use std::time::{Duration, Instant};

use hearo_ffi::HearoEngine;
use hearo_testkit::{signals, wav};

#[test]
#[ignore = "hardware: plays a tone through the default output"]
fn watchdog_leaves_a_playing_engine_alone() {
    let path = std::env::temp_dir().join(format!("hearo-hw-watchdog-{}.wav", std::process::id()));
    // −30 dBFS 440 Hz for 8 s: audible but polite.
    wav::write(&path, &wav::Wav::mono(48_000, signals::sine(48_000.0, 440.0, 8.0, 0.03))).unwrap();

    let engine = HearoEngine::new(vec!["gain".into()]);
    engine.start().unwrap();
    let mut player = Command::new("afplay").arg(&path).spawn().expect("afplay");

    let t = Instant::now();
    let mut active_polls = 0;
    while t.elapsed() < Duration::from_secs(7) {
        std::thread::sleep(Duration::from_millis(250));
        let s = engine.status();
        if s.running && !s.idle {
            active_polls += 1;
        }
    }
    let _ = player.kill();
    let _ = player.wait();
    let _ = std::fs::remove_file(&path);
    let c = engine.counters();
    let s = engine.status();
    eprintln!("active polls {active_polls}/28, counters {c:?}, problem '{}'", s.problem);
    engine.stop();
    assert!(active_polls >= 20, "IOProc should run while afplay plays");
    assert_eq!((c.watchdog_rebuilds, c.give_ups), (0, 0), "a healthy engine must not be rebuilt");
    assert!(s.problem.is_empty());
}
