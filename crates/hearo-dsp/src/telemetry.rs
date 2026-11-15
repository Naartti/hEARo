//! Fixed-capacity telemetry written on the audio thread.
//!
//! [`TelemetryWriter`] never allocates: events go into a fixed array and are drained by the
//! engine after each block (into a lock-free ring for the UI). If the array fills up, further
//! events are counted as dropped instead of blocking.

use crate::ModuleId;

/// Maximum events buffered between drains.
pub const EVENT_CAPACITY: usize = 256;

/// What kind of thing a module removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// A short impulsive click.
    Click,
    /// A lip smack / wet mouth noise.
    Smack,
    /// A low-frequency plosive pop.
    Plosive,
    /// A sibilance burst.
    Sibilance,
    /// Hum detected or lost.
    Hum,
    /// The limiter engaged.
    Limit,
}

/// A discrete detection event.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    /// Which module produced it.
    pub module: ModuleId,
    /// What it was.
    pub kind: EventKind,
    /// Absolute sample index (at the chain input) where the event starts.
    pub start_sample: u64,
    /// Length in samples.
    pub duration_samples: u32,
    /// Energy removed, in dB (positive = reduction).
    pub energy_removed_db: f32,
}

impl Event {
    const EMPTY: Event = Event {
        module: ModuleId::Utility,
        kind: EventKind::Click,
        start_sample: 0,
        duration_samples: 0,
        energy_removed_db: 0.0,
    };
}

/// Per-module scalar meters, overwritten every block.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ModuleMeters {
    /// Gain reduction in dB (≥ 0 = reducing) over the module's last processed block: the block
    /// maximum for event-like modules (de-click: largest repaired region; de-smack: deepest duck),
    /// so short events still show. 0 while the slot is off or warming up (set by the chain).
    pub gain_reduction_db: f32,
    /// Triggers since the chain was created.
    pub triggers: u64,
    /// Module-specific values (documented per module). De-hum: `[f0 Hz (0 = none),
    /// hum level dBFS, notch depth 0..1, tonal score dB]`.
    pub detail: [f32; 4],
}

/// Real-time-safe telemetry sink passed to every [`Module::process`](crate::Module::process).
#[derive(Debug)]
pub struct TelemetryWriter {
    events: [Event; EVENT_CAPACITY],
    len: usize,
    dropped: u64,
    meters: [ModuleMeters; ModuleId::COUNT],
    block_start: u64,
}

impl Default for TelemetryWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl TelemetryWriter {
    /// Create an empty writer.
    pub fn new() -> Self {
        Self {
            events: [Event::EMPTY; EVENT_CAPACITY],
            len: 0,
            dropped: 0,
            meters: [ModuleMeters::default(); ModuleId::COUNT],
            block_start: 0,
        }
    }

    /// Absolute index of the first sample of the block being processed.
    #[inline]
    pub fn block_start(&self) -> u64 {
        self.block_start
    }

    /// Set by the chain before each (sub-)block.
    #[inline]
    pub fn set_block_start(&mut self, sample: u64) {
        self.block_start = sample;
    }

    /// Record an event. Never blocks; drops (and counts) when full.
    #[inline]
    pub fn push_event(&mut self, event: Event) {
        if self.len < EVENT_CAPACITY {
            self.events[self.len] = event;
            self.len += 1;
        } else {
            self.dropped += 1;
        }
        self.meters[event.module.index()].triggers += 1;
    }

    /// Set a module's current gain reduction.
    #[inline]
    pub fn set_gain_reduction(&mut self, module: ModuleId, db: f32) {
        self.meters[module.index()].gain_reduction_db = db;
    }

    /// Mutable meters for one module (modules write their own `detail`).
    #[inline]
    pub fn meters_mut(&mut self, module: ModuleId) -> &mut ModuleMeters {
        &mut self.meters[module.index()]
    }

    /// Meters for one module.
    pub fn meters(&self, module: ModuleId) -> ModuleMeters {
        self.meters[module.index()]
    }

    /// Events recorded since the last [`clear_events`](Self::clear_events).
    pub fn events(&self) -> &[Event] {
        &self.events[..self.len]
    }

    /// Number of buffered events: a mark for [`retain_events_since`](Self::retain_events_since).
    #[inline]
    pub fn event_count(&self) -> usize {
        self.len
    }

    /// Drop the events recorded since `mark` for which `keep` is false, and uncount their
    /// triggers (the chain uses this for audio a slot didn't remove). Order is kept; events
    /// that overflowed the buffer can't be inspected and stay counted. Real-time safe.
    pub fn retain_events_since(&mut self, mark: usize, mut keep: impl FnMut(&Event) -> bool) {
        let from = mark.min(self.len);
        let mut kept = from;
        for i in from..self.len {
            let e = self.events[i];
            if keep(&e) {
                self.events[kept] = e;
                kept += 1;
            } else {
                let m = &mut self.meters[e.module.index()];
                m.triggers = m.triggers.saturating_sub(1);
            }
        }
        self.len = kept;
    }

    /// Forget buffered events (call after draining them).
    pub fn clear_events(&mut self) {
        self.len = 0;
    }

    /// Events dropped because the buffer was full.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev() -> Event {
        Event { module: ModuleId::DeClick, ..Event::EMPTY }
    }

    #[test]
    fn overflow_counts_drops_and_triggers() {
        let mut t = TelemetryWriter::new();
        for _ in 0..EVENT_CAPACITY + 5 {
            t.push_event(ev());
        }
        assert_eq!(t.events().len(), EVENT_CAPACITY);
        assert_eq!(t.dropped(), 5);
        assert_eq!(t.meters(ModuleId::DeClick).triggers, (EVENT_CAPACITY + 5) as u64);
        t.clear_events();
        assert!(t.events().is_empty());
    }

    #[test]
    fn retain_since_mark_uncounts_dropped_events_only() {
        let mut t = TelemetryWriter::new();
        t.push_event(Event { start_sample: 1, ..ev() });
        let mark = t.event_count();
        for s in 2..6 {
            t.push_event(Event { start_sample: s, ..ev() });
        }
        t.retain_events_since(mark, |e| e.start_sample % 2 == 1);
        let kept: Vec<u64> = t.events().iter().map(|e| e.start_sample).collect();
        assert_eq!(kept, [1, 3, 5], "events before the mark are untouched, order kept");
        assert_eq!(t.meters(ModuleId::DeClick).triggers, 3);
    }
}
