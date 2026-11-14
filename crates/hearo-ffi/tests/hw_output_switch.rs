//! Hardware test (needs two output devices; changes the default output and restores it):
//! the engine follows the system output. Run with `just test-hw` (quit hEARo first: two taps
//! would tap each other).

#![allow(clippy::print_stderr)] // reports what the hardware did

use std::time::{Duration, Instant};

use hearo_engine::ca::device;
use hearo_ffi::HearoEngine;

fn wait_for(what: &str, timeout: Duration, mut ok: impl FnMut() -> bool) {
    let t = Instant::now();
    while !ok() {
        assert!(t.elapsed() < timeout, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
#[ignore = "hardware: switches the default output device"]
fn engine_follows_the_default_output() {
    let original = device::default_output().unwrap();
    let other = device::outputs()
        .unwrap()
        .into_iter()
        .find(|d| d.id != original && !d.uid.starts_with("dev.hearo") && !d.name.contains("Teams"))
        .expect("needs a second output device");
    let engine = HearoEngine::new(vec!["desmack".into()]);
    engine.start().unwrap();
    let before = engine.status();
    eprintln!("started on {}", before.output_device);

    device::set_default_output(other.id).unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wait_for("rebuild on the new device", Duration::from_secs(5), || {
            let s = engine.status();
            s.running && s.output_device == other.name
        });
        let s = engine.status();
        eprintln!("followed to {} in ≤5 s (switches {}), problem '{}'", s.output_device, s.device_switches, s.problem);
        assert_eq!(s.device_switches, 1);
        assert!(s.problem.is_empty());
    }));
    // Always restore the user's output.
    device::set_default_output(original).unwrap();
    result.unwrap();
    wait_for("rebuild back on the original device", Duration::from_secs(5), || {
        let s = engine.status();
        s.running && s.output_device == before.output_device
    });
    assert_eq!(engine.status().device_switches, 2);
    engine.stop();
}
