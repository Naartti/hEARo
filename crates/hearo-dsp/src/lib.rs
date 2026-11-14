//! # hearo-dsp
//!
//! Pure, real-time-safe DSP for hEARo. Nothing in this crate touches the OS: every module
//! works identically on live audio and on WAV files, which is what makes the offline golden
//! tests meaningful.
//!
//! The real-time contract (see `docs/deep-dives/02-real-time-audio-in-rust.md`):
//! [`Module::process`] and [`Chain::process`] never allocate, lock, block, do I/O or log.
//! Allocation is only allowed in constructors and `prepare`.

pub mod analysis;
pub mod block;
pub mod chain;
pub mod delay;
pub mod denormal;
pub mod fade;
pub mod filters;
pub mod module;
pub mod modules;
pub mod offline;
pub mod telemetry;

pub use block::{AudioBlock, MAX_CHANNELS};
pub use chain::{Chain, SlotState};
pub use module::{Module, ModuleId, ParamChoice, ParamDesc, ParamUnit};
pub use telemetry::{Event, EventKind, ModuleMeters, TelemetryWriter};

/// Convert a linear amplitude to decibels (floored at -200 dB to stay finite).
#[inline]
pub fn amp_to_db(amp: f32) -> f32 {
    20.0 * amp.abs().max(1e-10).log10()
}

/// Convert decibels to a linear amplitude.
#[inline]
pub fn db_to_amp(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_roundtrip() {
        for db in [-60.0, -6.0, 0.0, 6.0] {
            assert!((amp_to_db(db_to_amp(db)) - db).abs() < 1e-4);
        }
        assert!(amp_to_db(0.0).is_finite());
    }
}
