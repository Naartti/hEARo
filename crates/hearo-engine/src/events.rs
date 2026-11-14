//! Event ring: the chain's detection events (smack removed, click repaired, …) from the audio
//! thread to the control thread, e.g. for auto-saving de-smack catches (ADR 0018).
//!
//! Single producer (the IOProc), single consumer (one control thread). Each slot is a few
//! atomics; the producer fills a slot with relaxed stores and publishes it by advancing `head`
//! (Release); the consumer reads up to `head` (Acquire) and hands the slots back by advancing
//! `tail` (Release), which the producer reads (Acquire) before reusing a slot. A full ring drops
//! the newest event and counts it — the audio thread never waits. No locks, no allocation.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use hearo_dsp::{Event, EventKind, ModuleId};

/// Slots in the ring. The control thread drains every 0.5 s; even a busy de-ess makes far fewer.
pub const EVENT_RING_CAPACITY: usize = 1024;

#[derive(Debug)]
struct Slot {
    start: AtomicU64,
    /// module index (bits 0–7) | kind (8–15) | duration in samples (32–63).
    packed: AtomicU64,
    energy: AtomicU32,
}

/// Lock-free SPSC ring of [`Event`]s.
#[derive(Debug)]
pub struct EventRing {
    slots: Box<[Slot]>,
    /// Events published (producer-owned).
    head: AtomicU64,
    /// Events consumed (consumer-owned).
    tail: AtomicU64,
    dropped: AtomicU64,
}

impl Default for EventRing {
    fn default() -> Self {
        Self::new(EVENT_RING_CAPACITY)
    }
}

fn kind_code(k: EventKind) -> u64 {
    match k {
        EventKind::Click => 0,
        EventKind::Smack => 1,
        EventKind::Plosive => 2,
        EventKind::Sibilance => 3,
        EventKind::Hum => 4,
        EventKind::Limit => 5,
    }
}

fn kind_from(c: u64) -> EventKind {
    match c {
        0 => EventKind::Click,
        1 => EventKind::Smack,
        2 => EventKind::Plosive,
        3 => EventKind::Sibilance,
        4 => EventKind::Hum,
        _ => EventKind::Limit,
    }
}

impl EventRing {
    /// A ring of `capacity` slots (control thread; allocates).
    pub fn new(capacity: usize) -> Self {
        let slots = (0..capacity.max(1))
            .map(|_| Slot { start: AtomicU64::new(0), packed: AtomicU64::new(0), energy: AtomicU32::new(0) })
            .collect();
        Self { slots, head: AtomicU64::new(0), tail: AtomicU64::new(0), dropped: AtomicU64::new(0) }
    }

    /// Publish events. Real-time safe; single producer only (the IOProc).
    #[inline]
    pub fn push_all(&self, events: &[Event]) {
        let cap = self.slots.len() as u64;
        let mut head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        for e in events {
            if head - tail >= cap {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            let s = &self.slots[(head % cap) as usize];
            s.start.store(e.start_sample, Ordering::Relaxed);
            let packed = e.module.index() as u64 | kind_code(e.kind) << 8 | (e.duration_samples as u64) << 32;
            s.packed.store(packed, Ordering::Relaxed);
            s.energy.store(e.energy_removed_db.to_bits(), Ordering::Relaxed);
            head += 1;
        }
        self.head.store(head, Ordering::Release);
    }

    /// Move every published event into `out` (oldest first). Single consumer only.
    pub fn drain(&self, out: &mut Vec<Event>) {
        let cap = self.slots.len() as u64;
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        for i in tail..head {
            let s = &self.slots[(i % cap) as usize];
            let packed = s.packed.load(Ordering::Relaxed);
            out.push(Event {
                module: ModuleId::ALL.get((packed & 0xFF) as usize).copied().unwrap_or(ModuleId::Utility),
                kind: kind_from((packed >> 8) & 0xFF),
                start_sample: s.start.load(Ordering::Relaxed),
                duration_samples: (packed >> 32) as u32,
                energy_removed_db: f32::from_bits(s.energy.load(Ordering::Relaxed)),
            });
        }
        self.tail.store(head, Ordering::Release);
    }

    /// Events dropped because the ring was full.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(module: ModuleId, kind: EventKind, start: u64) -> Event {
        Event { module, kind, start_sample: start, duration_samples: 1234, energy_removed_db: 17.5 }
    }

    #[test]
    fn round_trips_every_field_in_order() {
        let ring = EventRing::new(8);
        let a = ev(ModuleId::DeSmack, EventKind::Smack, 5_000_000_000);
        let b = ev(ModuleId::DeEss, EventKind::Sibilance, 7);
        ring.push_all(&[a, b]);
        let mut out = Vec::new();
        ring.drain(&mut out);
        assert_eq!(out, vec![a, b]);
        out.clear();
        ring.drain(&mut out);
        assert!(out.is_empty(), "drained events are gone");
    }

    #[test]
    fn full_ring_drops_newest_and_counts() {
        let ring = EventRing::new(4);
        let evs: Vec<Event> = (0..6).map(|i| ev(ModuleId::DeClick, EventKind::Click, i)).collect();
        ring.push_all(&evs);
        assert_eq!(ring.dropped(), 2);
        let mut out = Vec::new();
        ring.drain(&mut out);
        assert_eq!(out.iter().map(|e| e.start_sample).collect::<Vec<_>>(), vec![0, 1, 2, 3]);
        // Space again after draining, across the wrap.
        ring.push_all(&evs[..3]);
        out.clear();
        ring.drain(&mut out);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn push_does_not_allocate() {
        let ring = EventRing::new(64);
        let evs = [ev(ModuleId::DeSmack, EventKind::Smack, 1); 8];
        let (_, allocs) = hearo_testkit::rt::count_allocs(|| {
            for _ in 0..100 {
                ring.push_all(&evs);
            }
        });
        assert_eq!(allocs, 0);
    }

    /// A producer and a consumer thread: every event arrives exactly once, in order, unless it
    /// was counted as dropped.
    #[test]
    fn concurrent_producer_and_consumer_lose_nothing_silently() {
        use std::sync::Arc;
        let ring = Arc::new(EventRing::new(32));
        let p = ring.clone();
        let producer = std::thread::spawn(move || {
            for i in 0..20_000u64 {
                p.push_all(&[ev(ModuleId::DeSmack, EventKind::Smack, i)]);
            }
        });
        let mut got = Vec::new();
        while !producer.is_finished() {
            ring.drain(&mut got);
        }
        producer.join().unwrap();
        ring.drain(&mut got);
        assert!(got.windows(2).all(|w| w[1].start_sample > w[0].start_sample), "out of order");
        assert!(got.iter().all(|e| e.duration_samples == 1234 && e.energy_removed_db == 17.5), "torn slot");
        assert_eq!(got.len() as u64 + ring.dropped(), 20_000);
    }
}
