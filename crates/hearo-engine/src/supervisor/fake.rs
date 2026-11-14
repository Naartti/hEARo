//! A scripted [`AudioBackend`] for testing the supervisor without Core Audio: the test sets the
//! "world" (default output, whether tapped apps play, whether the IOProc delivers sound, start
//! failures) and checks what the supervisor did.

use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use super::{Activity, AudioBackend};
use crate::Controls;

/// What the (fake) IOProc of an engine does on each observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// Nothing tapped is playing: no callbacks, and that's fine.
    Idle,
    /// Apps play and the IOProc processes sound.
    Healthy,
    /// Apps play but the tap delivers only digital silence.
    Silent,
    /// Apps play but the IOProc gets no callbacks (the user would hear silence).
    Dead,
}

/// A fake engine: what it was built on, its controls, its activity.
#[derive(Debug)]
pub struct FakeEngine {
    /// Start number (1 = first engine).
    pub serial: u32,
    /// Output UID and rate it plays to.
    pub output: (String, f64),
    /// Controls the supervisor writes.
    pub controls: Controls,
    activity: Cell<Activity>,
    live: Arc<AtomicU32>,
}

impl Drop for FakeEngine {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The scripted world.
#[derive(Debug)]
pub struct FakeBackend {
    /// Current default output (UID, nominal rate).
    pub output: (String, f64),
    /// What every running engine's IOProc does.
    pub flow: Flow,
    /// After each successful start, switch `flow` to this (models "a rebuild fixes it").
    pub flow_after_start: Option<Flow>,
    /// The next this-many starts fail.
    pub fail_starts: u32,
    /// Start attempts (successful or not).
    pub attempts: u32,
    /// Times `start` was called while another engine was still alive (must stay 0).
    pub overlaps: u32,
    serial: u32,
    live: Arc<AtomicU32>,
}

impl Default for FakeBackend {
    fn default() -> Self {
        Self {
            output: ("speakers".into(), 48_000.0),
            flow: Flow::Idle,
            flow_after_start: None,
            fail_starts: 0,
            attempts: 0,
            overlaps: 0,
            serial: 0,
            live: Arc::new(AtomicU32::new(0)),
        }
    }
}

impl FakeBackend {
    /// Engines currently alive (0 or 1).
    pub fn live(&self) -> u32 {
        self.live.load(Ordering::SeqCst)
    }
}

/// Error of a scripted start failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeError(pub String);

impl std::fmt::Display for FakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AudioBackend for FakeBackend {
    type Engine = FakeEngine;
    type Error = FakeError;

    fn start(&mut self) -> Result<FakeEngine, FakeError> {
        self.attempts += 1;
        if self.live() > 0 {
            self.overlaps += 1;
        }
        if self.fail_starts > 0 {
            self.fail_starts -= 1;
            return Err(FakeError(format!("device '{}' not ready", self.output.0)));
        }
        if let Some(f) = self.flow_after_start {
            self.flow = f;
        }
        self.serial += 1;
        self.live.fetch_add(1, Ordering::SeqCst);
        Ok(FakeEngine {
            serial: self.serial,
            output: self.output.clone(),
            controls: Controls::default(),
            activity: Cell::new(Activity::default()),
            live: self.live.clone(),
        })
    }

    fn output_changed(&mut self, engine: &FakeEngine) -> bool {
        engine.output != self.output
    }

    fn default_output(&mut self) -> Option<(String, f64)> {
        Some(self.output.clone())
    }

    fn controls<'a>(&self, engine: &'a FakeEngine) -> &'a Controls {
        &engine.controls
    }

    fn activity(&self, engine: &FakeEngine) -> Activity {
        // Each observation = one tick's worth of IOProc cycles.
        let mut a = engine.activity.get();
        match self.flow {
            Flow::Idle | Flow::Dead => {}
            Flow::Healthy => a.callbacks += 10,
            Flow::Silent => {
                a.callbacks += 10;
                a.silent += 10;
            }
        }
        engine.activity.set(a);
        a
    }

    fn tapped_output_running(&mut self, _engine: &FakeEngine) -> bool {
        self.flow != Flow::Idle
    }
}
