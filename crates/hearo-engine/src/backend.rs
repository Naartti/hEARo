//! [`LiveBackend`]: the Core Audio [`AudioBackend`] the supervisor drives in the app and CLI.

use crate::supervisor::{Activity, AudioBackend};
use crate::{Controls, EngineConfig, EngineError, LiveEngine};

/// Starts [`LiveEngine`]s from a fresh [`EngineConfig`] each time (modules are owned by the
/// chain, so every rebuild needs new ones).
pub struct LiveBackend {
    make_config: Box<dyn FnMut() -> EngineConfig + Send>,
}

impl LiveBackend {
    /// `make_config` is called once per (re)start.
    pub fn new(make_config: impl FnMut() -> EngineConfig + Send + 'static) -> Self {
        Self { make_config: Box::new(make_config) }
    }
}

impl std::fmt::Debug for LiveBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveBackend").finish_non_exhaustive()
    }
}

impl AudioBackend for LiveBackend {
    type Engine = LiveEngine;
    type Error = EngineError;

    fn start(&mut self) -> Result<LiveEngine, EngineError> {
        LiveEngine::start((self.make_config)())
    }

    fn output_changed(&mut self, engine: &LiveEngine) -> bool {
        engine.output_changed()
    }

    fn default_output(&mut self) -> Option<(String, f64)> {
        crate::ca::device::default_output_key().ok()
    }

    fn controls<'a>(&self, engine: &'a LiveEngine) -> &'a Controls {
        engine.controls()
    }

    fn activity(&self, engine: &LiveEngine) -> Activity {
        engine.stats().activity()
    }

    fn tapped_output_running(&mut self, engine: &LiveEngine) -> bool {
        engine.tapped_output_running()
    }
}
