//! Supervisor policy tests against the scripted [`FakeBackend`] and an injected clock.

use std::time::{Duration, Instant, SystemTime};

use super::fake::{FakeBackend, Flow};
use super::*;

const MS: Duration = Duration::from_millis(1);

struct Rig {
    sup: Supervisor<FakeBackend>,
    t0: Instant,
    now: Instant,
}

impl Rig {
    fn new() -> Self {
        let t0 = Instant::now();
        Self { sup: Supervisor::new(FakeBackend::default(), Settings::all_enabled(3)), t0, now: t0 }
    }

    fn started() -> Self {
        let mut r = Self::new();
        r.sup.start(r.now).expect("starts");
        r
    }

    fn world(&mut self) -> &mut FakeBackend {
        self.sup.backend_mut()
    }

    /// Advance the clock in TICK steps for `d`, ticking each time.
    fn run(&mut self, d: Duration) {
        let end = self.now + d;
        while self.now < end {
            self.now += TICK;
            self.sup.tick(self.now);
            assert!(self.sup.backend().live() <= 1, "never two engines");
        }
    }

    /// Advance until `cond` holds (or panic after `limit`); returns the time it took.
    fn until(&mut self, limit: Duration, mut cond: impl FnMut(&Supervisor<FakeBackend>) -> bool) -> Duration {
        let start = self.now;
        while !cond(&self.sup) {
            assert!(self.now - start <= limit, "condition not reached within {limit:?}");
            self.run(TICK);
        }
        self.now - start
    }

    fn serial(&self) -> u32 {
        self.sup.engine().map_or(0, |e| e.serial)
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            assert_eq!(self.sup.backend().overlaps, 0, "a new engine was built before the old one was torn down");
        }
    }
}

#[test]
fn not_started_does_nothing() {
    let mut r = Rig::new();
    r.run(Duration::from_secs(10));
    assert_eq!(r.sup.backend().attempts, 0);
    assert!(r.sup.engine().is_none());
}

#[test]
fn steady_output_never_rebuilds() {
    let mut r = Rig::started();
    r.world().flow = Flow::Healthy;
    r.run(Duration::from_secs(60));
    r.world().flow = Flow::Idle;
    r.run(Duration::from_secs(60));
    assert_eq!(r.sup.counters().starts, 1);
    assert_eq!(r.serial(), 1);
}

#[test]
fn device_switch_rebuilds_on_the_new_output_and_reapplies_settings() {
    let mut r = Rig::started();
    r.sup.set_bypass(true);
    r.sup.set_enabled(1, false);
    r.sup.set_param(2, 0, -6.0);
    r.world().output = ("airpods".into(), 48_000.0);
    r.run(TICK);
    let e = r.sup.engine().expect("rebuilt");
    assert_eq!((e.serial, e.output.0.as_str()), (2, "airpods"));
    assert!(e.controls.bypass());
    assert!(e.controls.enabled(0) && !e.controls.enabled(1) && e.controls.enabled(2));
    assert_eq!(e.controls.param(2, 0), -6.0);
    assert_eq!(r.sup.counters().device_switches, 1);
    assert_eq!(r.sup.problem(), None);
}

#[test]
fn set_modules_applies_now_and_after_a_rebuild() {
    let mut r = Rig::started();
    r.sup.set_bypass(true);
    let params = [((1, 0), 3.0), ((2, 1), -9.0)].into_iter().collect();
    r.sup.set_modules(vec![true, false, true], params);
    let e = r.sup.engine().expect("running");
    assert!(!e.controls.enabled(1));
    assert_eq!(e.controls.param(2, 1), -9.0);
    r.world().output = ("airpods".into(), 48_000.0);
    r.run(TICK);
    let e = r.sup.engine().expect("rebuilt");
    assert_eq!(e.serial, 2);
    assert!(e.controls.bypass(), "bypass is kept");
    assert!(e.controls.enabled(0) && !e.controls.enabled(1));
    assert_eq!((e.controls.param(1, 0), e.controls.param(2, 1)), (3.0, -9.0));
}

#[test]
fn sample_rate_change_rebuilds() {
    let mut r = Rig::started();
    r.world().output.1 = 44_100.0;
    r.run(TICK);
    assert_eq!(r.sup.engine().map(|e| e.output.1), Some(44_100.0));
    assert_eq!(r.sup.counters().device_switches, 1);
}

#[test]
fn failed_start_reports_and_retries_with_backoff() {
    let mut r = Rig::new();
    r.world().fail_starts = 3;
    assert!(r.sup.start(r.now).is_err());
    assert!(r.sup.problem().is_some_and(|p| p.contains("not ready")));
    assert!(r.sup.engine().is_none(), "no engine = macOS plays audio unprocessed");
    // Attempts at 3 s, then 3 + 6 = 9 s, then 9 + 12 = 21 s (succeeds).
    let mut attempt_times = vec![];
    let mut seen = 1;
    while r.sup.engine().is_none() {
        r.run(TICK);
        if r.sup.backend().attempts != seen {
            seen = r.sup.backend().attempts;
            attempt_times.push((r.now - r.t0).as_millis());
        }
        assert!(r.now - r.t0 < Duration::from_secs(60));
    }
    assert_eq!(attempt_times, vec![3000, 9000, 21_000]);
    assert_eq!(r.sup.problem(), None, "problem clears once running");
    assert_eq!(r.sup.counters().start_failures, 3);
}

#[test]
fn retry_backoff_is_capped() {
    let mut r = Rig::new();
    r.world().fail_starts = u32::MAX;
    let _ = r.sup.start(r.now);
    r.run(Duration::from_secs(600));
    // 3, 9, 21, 45 s, then every 30 s: (600 − 45) / 30 = 18 more.
    assert_eq!(r.sup.backend().attempts, 1 + 4 + 18);
}

#[test]
fn new_output_ends_the_retry_wait() {
    let mut r = Rig::new();
    r.world().fail_starts = 1;
    let _ = r.sup.start(r.now);
    r.run(TICK);
    assert!(r.sup.engine().is_none(), "still waiting for the retry");
    r.world().output = ("usb-dac".into(), 96_000.0);
    r.run(TICK);
    assert_eq!(r.sup.engine().map(|e| e.output.0.as_str()), Some("usb-dac"));
}

#[test]
fn a_new_output_that_also_fails_backs_off_from_there() {
    let mut r = Rig::new();
    r.world().fail_starts = 2;
    let _ = r.sup.start(r.now);
    r.world().output = ("airpods".into(), 48_000.0);
    r.run(TICK); // tries the new output at once: fails too
    assert_eq!(r.sup.backend().attempts, 2);
    // Then a fresh backoff for the new device (3 s), not a retry on every tick.
    r.run(RETRY_MIN - TICK);
    assert_eq!(r.sup.backend().attempts, 2);
    r.run(TICK);
    assert!(r.sup.engine().is_some());
    assert_eq!(r.sup.backend().attempts, 3);
}

#[test]
fn watchdog_ignores_idle_and_quiet_but_flowing_audio() {
    let mut r = Rig::started();
    r.world().flow = Flow::Idle; // nothing plays → no callbacks: normal (ADR 0010)
    r.run(Duration::from_secs(120));
    r.world().flow = Flow::Healthy;
    r.run(Duration::from_secs(120));
    assert_eq!(r.sup.counters().watchdog_rebuilds, 0);
    assert_eq!(r.serial(), 1);
}

#[test]
fn watchdog_rebuilds_a_dead_ioproc_fast() {
    let mut r = Rig::started();
    r.world().flow = Flow::Dead; // apps play, no callbacks: the user hears silence
    r.world().flow_after_start = Some(Flow::Healthy);
    let took = r.until(Duration::from_secs(10), |s| s.counters().watchdog_rebuilds == 1);
    assert!(took <= STALL + TICK, "rebuilt after {took:?}");
    assert_eq!(r.serial(), 2);
    r.run(Duration::from_secs(60));
    assert_eq!(r.sup.counters().watchdog_rebuilds, 1, "healthy after the rebuild");
    assert_eq!(r.sup.problem(), None);
}

#[test]
fn watchdog_reapplies_settings_on_rebuild() {
    let mut r = Rig::started();
    r.sup.set_enabled(0, false);
    r.world().flow = Flow::Dead;
    r.world().flow_after_start = Some(Flow::Healthy);
    r.until(Duration::from_secs(10), |s| s.counters().watchdog_rebuilds == 1);
    assert!(!r.sup.engine().unwrap().controls.enabled(0));
}

#[test]
fn watchdog_gives_up_to_passthrough_and_reports() {
    let mut r = Rig::started();
    r.world().flow = Flow::Dead; // and stays dead
    let took = r.until(Duration::from_secs(30), |s| s.counters().give_ups == 1);
    // 2 s → rebuild, 2 s → rebuild, 4 s → give up (+ tick granularity).
    assert!(took <= STALL * 4 + TICK * 3, "gave up after {took:?}");
    assert_eq!(r.sup.counters().watchdog_rebuilds, DEAD_REBUILDS);
    assert!(r.sup.engine().is_none(), "torn down: macOS plays audio unprocessed");
    assert_eq!(r.sup.backend().live(), 0);
    assert_eq!(r.sup.problem(), Some(GAVE_UP));
    // Stays down (no rebuild churn) ...
    let attempts = r.sup.backend().attempts;
    r.run(Duration::from_secs(600));
    assert_eq!(r.sup.backend().attempts, attempts);
    // ... until the user switches it on again.
    r.world().flow = Flow::Healthy;
    r.sup.start(r.now).expect("restart");
    assert_eq!(r.sup.problem(), None);
    assert!(r.sup.engine().is_some());
}

#[test]
fn new_output_retries_after_giving_up() {
    let mut r = Rig::started();
    r.world().flow = Flow::Dead;
    r.until(Duration::from_secs(30), |s| s.counters().give_ups == 1);
    r.world().flow = Flow::Healthy;
    r.world().output = ("airpods".into(), 48_000.0);
    r.run(TICK);
    assert_eq!(r.sup.engine().map(|e| e.output.0.as_str()), Some("airpods"));
    assert_eq!(r.sup.problem(), None);
}

#[test]
fn silent_tap_rebuilds_with_backoff_and_never_gives_up() {
    let mut r = Rig::started();
    r.world().flow = Flow::Silent; // could be a call where nobody talks: inaudible to rebuild
    r.run(Duration::from_secs(300));
    let c = r.sup.counters();
    assert_eq!(c.give_ups, 0);
    assert!(r.sup.engine().is_some());
    // 2, 4, 8, 16, 32, 60, 60, 60, 60 s … ≈ 9 rebuilds in 5 min (+ tick granularity).
    assert!((7..=10).contains(&c.watchdog_rebuilds), "{} rebuilds", c.watchdog_rebuilds);
}

#[test]
fn sound_resets_the_watchdog_episode() {
    let mut r = Rig::started();
    r.world().flow_after_start = Some(Flow::Healthy);
    for _ in 0..5 {
        let serial = r.serial();
        r.world().flow = Flow::Dead;
        r.run(STALL + TICK);
        assert_eq!(r.serial(), serial + 1, "each new episode starts with a short stall");
        r.run(Duration::from_secs(5)); // healthy again
    }
    assert_eq!(r.sup.counters().give_ups, 0);
    assert_eq!(r.sup.counters().watchdog_rebuilds, 5);
}

#[test]
fn wake_rebuilds_immediately_and_reapplies_settings() {
    let mut r = Rig::started();
    r.sup.set_bypass(true);
    r.world().flow = Flow::Healthy;
    r.sup.woke(r.now);
    let e = r.sup.engine().expect("rebuilt");
    assert_eq!(e.serial, 2);
    assert!(e.controls.bypass());
    assert_eq!(r.sup.counters().wake_rebuilds, 1);
}

#[test]
fn wake_with_device_not_ready_retries() {
    let mut r = Rig::started();
    r.world().fail_starts = 1;
    r.sup.woke(r.now);
    assert!(r.sup.engine().is_none());
    assert!(r.sup.problem().is_some());
    r.run(RETRY_MIN);
    assert!(r.sup.engine().is_some());
    assert_eq!(r.sup.problem(), None);
}

#[test]
fn wake_when_stopped_does_nothing_and_wake_after_give_up_restarts() {
    let mut r = Rig::new();
    r.sup.woke(r.now);
    assert_eq!(r.sup.backend().attempts, 0);

    let mut r = Rig::started();
    r.world().flow = Flow::Dead;
    r.until(Duration::from_secs(30), |s| s.counters().give_ups == 1);
    r.world().flow = Flow::Healthy;
    r.sup.woke(r.now);
    assert!(r.sup.engine().is_some());
    assert_eq!(r.sup.problem(), None);
}

#[test]
fn stop_tears_down_and_stays_down() {
    let mut r = Rig::started();
    r.sup.stop();
    assert_eq!(r.sup.backend().live(), 0);
    r.world().output = ("airpods".into(), 48_000.0);
    r.run(Duration::from_secs(30));
    assert_eq!(r.sup.backend().attempts, 1);
    assert_eq!(r.sup.problem(), None);
}

#[test]
fn settings_survive_while_stopped() {
    let mut r = Rig::new();
    r.sup.set_enabled(2, false);
    r.sup.set_enabled(99, false); // out of range: ignored
    r.sup.set_bypass(true);
    r.sup.start(r.now).unwrap();
    let c = &r.sup.engine().unwrap().controls;
    assert!(c.bypass() && !c.enabled(2) && c.enabled(0));
}

#[test]
fn sleep_detector_sees_wall_clock_jumps_only() {
    let mut d = SleepDetector::default();
    let (m, w) = (Instant::now(), SystemTime::now());
    assert!(!d.observe(m, w), "first observation");
    assert!(!d.observe(m + TICK, w + TICK), "normal tick");
    // Slept 1 h: monotonic advanced one tick, wall an hour.
    assert!(d.observe(m + TICK * 2, w + TICK * 2 + Duration::from_secs(3600)));
    let w = w + Duration::from_secs(3600);
    // Wall clock set back: not a wake.
    assert!(!d.observe(m + TICK * 3, w - Duration::from_secs(600)));
    let w = w - Duration::from_secs(600);
    // Small jitter (NTP slew, slow tick): not a wake.
    assert!(!d.observe(m + TICK * 4, w + TICK + 900 * MS));
    let w = w + TICK + 900 * MS;
    // Thread frozen for a long time (monotonic gap) → treat like a wake.
    assert!(d.observe(m + TICK * 4 + LONG_GAP, w + LONG_GAP));
}
