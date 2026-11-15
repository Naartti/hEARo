//! # hearo-engine
//!
//! The live audio engine: a Core Audio process tap feeding a private aggregate device whose
//! IOProc runs the `hearo-dsp` chain and writes to the real output device
//! (see `docs/deep-dives/01-core-audio-taps.md`, ADR 0003, ADR 0010).
//!
//! Threads: the IOProc (real time) only touches pre-allocated state, atomics in
//! [`Controls`] and [`Stats`], and lock-free rings ([`CaptureRing`], [`EventRing`], the waveform).
//! Everything else (setup, teardown, polling) is control-thread work.

pub mod autocapture;
pub mod capture;
pub mod controls;
pub mod events;
pub mod snapshots;
pub mod stats;
pub mod supervisor;
pub mod waveform;

#[cfg(target_os = "macos")]
mod backend;
#[cfg(target_os = "macos")]
pub mod ca;
#[cfg(target_os = "macos")]
mod engine;
#[cfg(target_os = "macos")]
mod rt;

#[cfg(target_os = "macos")]
pub use backend::LiveBackend;
pub use capture::CaptureRing;
pub use controls::{Controls, MAX_PARAMS, MAX_SLOTS};
#[cfg(target_os = "macos")]
pub use engine::{EngineConfig, EngineError, LiveEngine, RunInfo, scaled_buffer_frames};
pub use events::EventRing;
pub use stats::{Stats, StatsSnapshot};

/// The app's bundle id; also the prefix of the private aggregate device UIDs hEARo creates.
pub const BUNDLE_ID: &str = "io.github.naartti.hearo";

/// UID of the private aggregate device hEARo creates for process `pid` and tap `tap_id`.
pub fn aggregate_uid(pid: impl std::fmt::Display, tap_id: impl std::fmt::Display) -> String {
    format!("{BUNDLE_ID}.aggregate.{pid}.{tap_id}")
}

/// Whether a device UID belongs to a hEARo aggregate (this build's, or one created by a
/// still-running build from before the 2026-11-15 rename, which used `dev.hearo`).
pub fn is_own_device(uid: &str) -> bool {
    uid.starts_with(&format!("{BUNDLE_ID}.")) || uid.starts_with("dev.hearo.")
}

/// Bundle-ID prefixes excluded from processing by default: music players and DAWs (§13.3).
pub const DEFAULT_EXCLUDES: &[&str] = &[
    "com.apple.Music",
    "com.spotify.client",
    "com.apple.TV",
    "com.apple.logic10",
    "com.apple.garageband10",
    "com.ableton.live",
];

/// Whether `bundle_id` matches one of the exclusion prefixes (helpers like
/// `com.spotify.client.helper` match `com.spotify.client`).
pub fn is_excluded(bundle_id: &str, prefixes: &[impl AsRef<str>]) -> bool {
    prefixes.iter().any(|p| {
        let p = p.as_ref();
        !p.is_empty() && (bundle_id == p || bundle_id.starts_with(&format!("{p}.")))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn buffer_scales_with_sample_rate() {
        assert_eq!(scaled_buffer_frames(64, 44_100.0), 64);
        assert_eq!(scaled_buffer_frames(64, 48_000.0), 64);
        assert_eq!(scaled_buffer_frames(64, 96_000.0), 128);
        assert_eq!(scaled_buffer_frames(64, 192_000.0), 256);
        assert_eq!(scaled_buffer_frames(16, 22_050.0), 32);
    }

    #[test]
    fn exclusion_prefixes_match_helpers_not_lookalikes() {
        assert!(is_excluded("com.spotify.client", DEFAULT_EXCLUDES));
        assert!(is_excluded("com.spotify.client.helper", DEFAULT_EXCLUDES));
        assert!(!is_excluded("com.spotify.clientele", DEFAULT_EXCLUDES));
        assert!(!is_excluded("com.microsoft.teams2", DEFAULT_EXCLUDES));
        assert!(!is_excluded("", DEFAULT_EXCLUDES));
    }

    #[test]
    fn aggregate_uids_are_recognised_as_our_own() {
        let uid = aggregate_uid(4242, 17);
        assert_eq!(uid, "io.github.naartti.hearo.aggregate.4242.17");
        assert!(is_own_device(&uid));
        assert!(is_own_device("dev.hearo.aggregate.1.2"), "pre-rename builds");
        assert!(!is_own_device("BuiltInSpeakerDevice"));
        assert!(!is_own_device("io.github.naartti.hearosomething"));
    }
}
