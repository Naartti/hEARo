//! Real-time statistics written by the IOProc with relaxed atomics and read by the UI/CLI.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use hearo_dsp::ModuleId;

/// Counters and meters. Peaks and max cycle time are "since last snapshot".
#[derive(Debug, Default)]
pub struct Stats {
    callbacks: AtomicU64,
    frames: AtomicU64,
    silent_callbacks: AtomicU64,
    short_input: AtomicU64,
    overruns: AtomicU64,
    panics: AtomicU64,
    faults: AtomicU64,
    in_peak: AtomicU32,
    out_peak: AtomicU32,
    max_cycle_ns: AtomicU32,
    last_frames: AtomicU32,
    tap_channels: AtomicU32,
    module_triggers: [AtomicU64; ModuleId::COUNT],
    /// Gain reduction in dB as f32 bits.
    module_gr: [AtomicU32; ModuleId::COUNT],
}

/// A copy of [`Stats`] at one moment.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StatsSnapshot {
    /// IOProc calls so far.
    pub callbacks: u64,
    /// Frames processed so far.
    pub frames: u64,
    /// Callbacks whose tap input was digital silence.
    pub silent_callbacks: u64,
    /// Callbacks where the tap delivered fewer frames than the output needed.
    pub short_input: u64,
    /// Callbacks that took longer than the real-time budget (the buffer duration).
    pub overruns: u64,
    /// Panics caught in the IOProc (output fell back to passthrough).
    pub panics: u64,
    /// Chain faults (non-finite samples caught).
    pub faults: u64,
    /// Input peak since the last snapshot (linear).
    pub in_peak: f32,
    /// Output peak since the last snapshot (linear).
    pub out_peak: f32,
    /// Longest IOProc cycle since the last snapshot, in nanoseconds.
    pub max_cycle_ns: u32,
    /// Frames in the most recent callback.
    pub last_frames: u32,
    /// Channels in the tap stream.
    pub tap_channels: u32,
}

#[inline]
fn max_f32(a: &AtomicU32, v: f32) {
    // Peaks are non-negative, so their bit patterns order like the floats.
    a.fetch_max(v.to_bits(), Ordering::Relaxed);
}

impl Stats {
    /// Record one IOProc cycle. Real-time safe.
    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub fn record(&self, frames: u32, in_peak: f32, out_peak: f32, cycle_ns: u32, budget_ns: u32, short: bool) {
        self.callbacks.fetch_add(1, Ordering::Relaxed);
        self.frames.fetch_add(frames as u64, Ordering::Relaxed);
        self.last_frames.store(frames, Ordering::Relaxed);
        if in_peak == 0.0 {
            self.silent_callbacks.fetch_add(1, Ordering::Relaxed);
        }
        if short {
            self.short_input.fetch_add(1, Ordering::Relaxed);
        }
        if cycle_ns > budget_ns {
            self.overruns.fetch_add(1, Ordering::Relaxed);
        }
        max_f32(&self.in_peak, in_peak.abs());
        max_f32(&self.out_peak, out_peak.abs());
        self.max_cycle_ns.fetch_max(cycle_ns, Ordering::Relaxed);
    }

    /// A panic was caught in the IOProc.
    pub fn record_panic(&self) {
        self.panics.fetch_add(1, Ordering::Relaxed);
    }

    /// Publish the chain's fault counter.
    pub fn set_faults(&self, faults: u64) {
        self.faults.store(faults, Ordering::Relaxed);
    }

    /// Publish the tap channel count.
    pub fn set_tap_channels(&self, n: u32) {
        self.tap_channels.store(n, Ordering::Relaxed);
    }

    /// Publish one module's meters (audio thread, after each cycle).
    #[inline]
    pub fn set_module(&self, id: ModuleId, triggers: u64, gain_reduction_db: f32) {
        self.module_triggers[id.index()].store(triggers, Ordering::Relaxed);
        self.module_gr[id.index()].store(gain_reduction_db.to_bits(), Ordering::Relaxed);
    }

    /// One module's (triggers, gain reduction dB).
    pub fn module(&self, id: ModuleId) -> (u64, f32) {
        (
            self.module_triggers[id.index()].load(Ordering::Relaxed),
            f32::from_bits(self.module_gr[id.index()].load(Ordering::Relaxed)),
        )
    }

    /// Callback counters only, resetting nothing the UI reads (for the supervisor's watchdog).
    pub fn activity(&self) -> crate::supervisor::Activity {
        crate::supervisor::Activity {
            callbacks: self.callbacks.load(Ordering::Relaxed),
            silent: self.silent_callbacks.load(Ordering::Relaxed),
        }
    }

    /// Read everything; resets the "since last snapshot" peaks and max cycle time.
    pub fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            callbacks: self.callbacks.load(Ordering::Relaxed),
            frames: self.frames.load(Ordering::Relaxed),
            silent_callbacks: self.silent_callbacks.load(Ordering::Relaxed),
            short_input: self.short_input.load(Ordering::Relaxed),
            overruns: self.overruns.load(Ordering::Relaxed),
            panics: self.panics.load(Ordering::Relaxed),
            faults: self.faults.load(Ordering::Relaxed),
            in_peak: f32::from_bits(self.in_peak.swap(0, Ordering::Relaxed)),
            out_peak: f32::from_bits(self.out_peak.swap(0, Ordering::Relaxed)),
            max_cycle_ns: self.max_cycle_ns.swap(0, Ordering::Relaxed),
            last_frames: self.last_frames.load(Ordering::Relaxed),
            tap_channels: self.tap_channels.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_resets_peaks() {
        let s = Stats::default();
        s.record(512, 0.5, 0.25, 1000, 10_000, false);
        s.record(512, 0.0, 0.0, 20_000, 10_000, true);
        let a = s.snapshot();
        assert_eq!((a.callbacks, a.frames, a.silent_callbacks, a.short_input, a.overruns), (2, 1024, 1, 1, 1));
        assert_eq!((a.in_peak, a.out_peak, a.max_cycle_ns), (0.5, 0.25, 20_000));
        let b = s.snapshot();
        assert_eq!((b.in_peak, b.max_cycle_ns, b.callbacks), (0.0, 0, 2));
        s.set_module(ModuleId::DeSmack, 7, 12.5);
        assert_eq!(s.module(ModuleId::DeSmack), (7, 12.5));
        assert_eq!(s.module(ModuleId::DeHum), (0, 0.0));
    }

    #[test]
    fn activity_reads_counters_without_resetting_meters() {
        let s = Stats::default();
        s.record(64, 0.5, 0.5, 1000, 10_000, false);
        s.record(64, 0.0, 0.0, 1000, 10_000, false);
        assert_eq!(s.activity(), crate::supervisor::Activity { callbacks: 2, silent: 1 });
        assert_eq!(s.snapshot().in_peak, 0.5, "peaks untouched by activity()");
    }
}
