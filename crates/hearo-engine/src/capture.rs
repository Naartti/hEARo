//! Capture ring: the last N seconds of *unprocessed* input, for labeling snapshots (§13.5).
//!
//! A seqlock-style single-writer ring. The audio thread first *claims* the range it will write
//! (`claimed`, then a Release fence), writes samples with relaxed atomic stores (plain stores on
//! ARM), then publishes `written`. The control thread reads `written`, copies a window, issues
//! an Acquire fence and re-reads `claimed`: if anything claimed since could have overwritten the
//! window, the copy is discarded. No locks, no allocation on the audio thread.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};

/// Lock-free stereo ring of `capacity` frames.
#[derive(Debug)]
pub struct CaptureRing {
    left: Box<[AtomicU32]>,
    right: Box<[AtomicU32]>,
    /// Frames the writer has started writing (≥ `written`).
    claimed: AtomicU64,
    /// Frames fully written.
    written: AtomicU64,
}

impl CaptureRing {
    /// Allocate a ring holding `capacity` frames (control thread).
    pub fn new(capacity: usize) -> Self {
        let make = || (0..capacity.max(1)).map(|_| AtomicU32::new(0)).collect::<Box<[_]>>();
        Self { left: make(), right: make(), claimed: AtomicU64::new(0), written: AtomicU64::new(0) }
    }

    /// Capacity in frames.
    pub fn capacity(&self) -> usize {
        self.left.len()
    }

    /// Total frames ever written.
    pub fn written(&self) -> u64 {
        self.written.load(Ordering::Acquire)
    }

    /// Append frames. Real-time safe; single writer only (the IOProc).
    #[inline]
    pub fn push(&self, l: &[f32], r: &[f32]) {
        let cap = self.left.len();
        let w = self.written.load(Ordering::Relaxed);
        let n = l.len().min(r.len()) as u64;
        self.claimed.store(w + n, Ordering::Relaxed);
        // Readers that see any of the sample stores below also see the claim.
        fence(Ordering::Release);
        let mut pos = (w % cap as u64) as usize;
        for (a, b) in l.iter().zip(r) {
            self.left[pos].store(a.to_bits(), Ordering::Relaxed);
            self.right[pos].store(b.to_bits(), Ordering::Relaxed);
            pos += 1;
            if pos == cap {
                pos = 0;
            }
        }
        self.written.store(w + n, Ordering::Release);
    }

    /// Copy the most recent `frames` frames (fewer if less has been written). Returns `None` if
    /// the writer lapped the window during the copy (retry). Control thread only.
    pub fn snapshot(&self, frames: usize) -> Option<(Vec<f32>, Vec<f32>)> {
        let cap = self.left.len();
        let end = self.written.load(Ordering::Acquire);
        let n = (frames as u64).min(end).min(cap as u64 / 2) as usize;
        let start = end - n as u64;
        let read = |ring: &[AtomicU32]| -> Vec<f32> {
            (0..n)
                .map(|i| {
                    if i == n / 2 {
                        test_hooks::stall(); // no-op outside tests
                    }
                    f32::from_bits(ring[((start + i as u64) % cap as u64) as usize].load(Ordering::Relaxed))
                })
                .collect()
        };
        let (l, r) = (read(&self.left), read(&self.right));
        // Pairs with the writer's Release fence: if we read any sample from a later write, we
        // also see its claim below and reject the window.
        fence(Ordering::Acquire);
        let claimed = self.claimed.load(Ordering::Relaxed);
        (claimed - start <= cap as u64).then_some((l, r))
    }
}

#[cfg(not(test))]
mod test_hooks {
    #[inline(always)]
    pub fn stall() {}
}

#[cfg(test)]
mod test_hooks {
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicU64, Ordering};
    /// Microseconds the reader stalls mid-copy (tests use it to force laps).
    pub static STALL_US: AtomicU64 = AtomicU64::new(0);
    thread_local! {
        /// Runs on the reader's thread mid-copy (deterministic interleavings).
        pub static MID_COPY: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
    }
    pub fn stall() {
        MID_COPY.with(|h| {
            if let Some(f) = h.borrow().as_ref() {
                f();
            }
        });
        let us = STALL_US.load(Ordering::Relaxed);
        if us > 0 {
            std::thread::sleep(std::time::Duration::from_micros(us));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_latest_frames_across_wraps() {
        let ring = CaptureRing::new(100);
        let data: Vec<f32> = (0..250).map(|i| i as f32).collect();
        for chunk in data.chunks(33) {
            ring.push(chunk, chunk);
        }
        let (l, r) = ring.snapshot(40).unwrap();
        assert_eq!(l, (210..250).map(|i| i as f32).collect::<Vec<_>>());
        assert_eq!(l, r);
        // Asking for more than half the ring is capped (leaves room for the writer).
        assert_eq!(ring.snapshot(1000).unwrap().0.len(), 50);
    }

    #[test]
    fn short_history_returns_what_exists() {
        let ring = CaptureRing::new(100);
        ring.push(&[1.0, 2.0], &[3.0, 4.0]);
        assert_eq!(ring.snapshot(10).unwrap(), (vec![1.0, 2.0], vec![3.0, 4.0]));
    }

    /// Deterministic seqlock check: mid-copy, a writer *claims* a range and overwrites part of
    /// the reader's window but hasn't published `written` yet. The reader must reject the copy.
    #[test]
    fn in_flight_write_over_window_is_rejected() {
        use std::sync::Arc;
        let ring = Arc::new(CaptureRing::new(256));
        let data: Vec<f32> = (0..256).map(|i| i as f32).collect();
        ring.push(&data, &data); // written = 256; a 128-frame window = frames 128..256
        let w = ring.clone();
        test_hooks::MID_COPY.with(|h| {
            *h.borrow_mut() = Some(Box::new(move || {
                // Writer at frame 256 claims 160 frames (→ 416): ring slots 0..160 wrap onto the
                // window's slots 128..160, and it has already written them.
                w.claimed.store(256 + 160, Ordering::Relaxed);
                fence(Ordering::Release);
                for pos in 0..160 {
                    w.left[pos].store((-1.0f32).to_bits(), Ordering::Relaxed);
                }
            }))
        });
        let result = ring.snapshot(128);
        test_hooks::MID_COPY.with(|h| *h.borrow_mut() = None);
        assert!(result.is_none(), "torn window accepted: {:?}", result.map(|(l, _)| l[..4].to_vec()));
    }

    /// Small ring + a reader that stalls mid-copy: the writer laps the window constantly, so
    /// any flaw in the claim/fence protocol shows up as a non-consecutive window.
    #[test]
    fn concurrent_writer_never_yields_torn_windows() {
        use std::sync::Arc;
        test_hooks::STALL_US.store(30, Ordering::Relaxed);
        let ring = Arc::new(CaptureRing::new(256));
        let w = ring.clone();
        let writer = std::thread::spawn(move || {
            let mut x = 0u64;
            for _ in 0..20_000 {
                let chunk: Vec<f32> = (0..64).map(|i| ((x + i) % 1_000_000) as f32).collect();
                w.push(&chunk, &chunk);
                x += 64;
            }
        });
        let mut checked = 0;
        while !writer.is_finished() {
            if let Some((l, _)) = ring.snapshot(128)
                && l.len() > 1
            {
                // A valid window is a run of consecutive integers.
                assert!(l.windows(2).all(|p| p[1] - p[0] == 1.0 || p[0] == 999_999.0), "torn window");
                checked += 1;
            }
        }
        writer.join().unwrap();
        test_hooks::STALL_US.store(0, Ordering::Relaxed);
        let _ = checked; // windows may all be rejected under heavy lapping; correctness is the point
        assert!(ring.snapshot(16).is_some(), "quiescent ring must snapshot");
    }
}
