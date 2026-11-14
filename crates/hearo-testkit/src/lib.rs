//! # hearo-testkit
//!
//! Deterministic (seeded) test signals, objective metrics, WAV helpers and a real-time
//! allocation checker. Shared by golden tests, benchmarks and the `hearo` CLI.
//!
//! Nothing here depends on `hearo-dsp`, so the DSP crate can use it as a dev-dependency
//! without type duplication.

pub mod filters;
pub mod metrics;
pub mod rng;
pub mod rt;
pub mod signals;
pub mod wav;

pub use rng::Rng;
