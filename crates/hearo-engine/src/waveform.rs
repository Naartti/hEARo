//! Live waveform for the popover: 60 columns per second. Each column holds, aligned with the
//! processed output:
//! - the peak of the *original* sound and the peak of the *processed* output;
//! - per cleanup module ([`ModuleId::CLEANUP`], chain order) the largest gain reduction in dB,
//!   quantised to 0.25 dB (0..63.75), so the UI can colour each module's share "outside-in".
//!
//! Written by the IOProc, read by the UI. Lock-free and allocation-free on the audio thread:
//! two atomic `u64` per column, single writer; the reader stays half the ring behind the writer.

use std::sync::atomic::{AtomicU64, Ordering};

use hearo_dsp::ModuleId;

/// Columns per second.
pub const COLUMNS_PER_SECOND: f32 = 60.0;
/// Cleanup modules with a gain-reduction row, in this order.
pub const GR_MODULES: [ModuleId; ModuleId::CLEANUP.len()] = ModuleId::CLEANUP;
const N_GR: usize = GR_MODULES.len();
const _: () = assert!(N_GR <= 8, "Column::pack holds at most 8 gain-reduction bytes");
const CAPACITY: usize = 4096; // ~68 s
/// Gain reduction resolution (dB per step of the u8 code).
pub const GR_STEP_DB: f32 = 0.25;
/// "Removed" = some cleanup module cut more than this in the column.
pub const REMOVED_ABOVE_DB: f32 = 1.0;
/// Longest per-module alignment delay, in columns (chain lookahead is ≤ 45 ms ≈ 3 columns).
const HISTORY: usize = 16;

/// One column as the UI sees it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Column {
    /// Peak |sample| of the original (unprocessed) sound in this column, 0..=1 (clamped).
    pub peak: f32,
    /// Peak |sample| of the processed output, 0..=1 (clamped).
    pub out_peak: f32,
    /// Largest gain reduction per [`GR_MODULES`] module in dB (≥ 0, 0.25 dB steps, ≤ 63.75).
    pub gain_reduction_db: [f32; N_GR],
}

impl Column {
    /// A cleanup module removed something here (more than [`REMOVED_ABOVE_DB`]).
    pub fn removed(&self) -> bool {
        self.gain_reduction_db.iter().any(|&g| g > REMOVED_ABOVE_DB)
    }

    fn pack(self) -> [u64; 2] {
        let q16 = |x: f32| (x.clamp(0.0, 1.0) * 65_535.0).round() as u64;
        let mut bytes = [0u64; 8];
        for (b, g) in bytes.iter_mut().zip(self.gain_reduction_db) {
            // NaN → 0 (`as` saturates and maps NaN to 0).
            *b = (g / GR_STEP_DB).round().clamp(0.0, 255.0) as u64;
        }
        let w0 = q16(self.peak)
            | q16(self.out_peak) << 16
            | bytes[0] << 32
            | bytes[1] << 40
            | bytes[2] << 48
            | bytes[3] << 56;
        let w1 = bytes[4] | bytes[5] << 8 | bytes[6] << 16 | bytes[7] << 24;
        [w0, w1]
    }

    fn unpack([w0, w1]: [u64; 2]) -> Self {
        let byte = |i: usize| if i < 4 { (w0 >> (32 + 8 * i)) & 0xFF } else { (w1 >> (8 * (i - 4))) & 0xFF };
        Self {
            peak: (w0 & 0xFFFF) as f32 / 65_535.0,
            out_peak: ((w0 >> 16) & 0xFFFF) as f32 / 65_535.0,
            gain_reduction_db: core::array::from_fn(|i| byte(i) as f32 * GR_STEP_DB),
        }
    }
}

/// Lock-free column ring.
#[derive(Debug)]
pub struct WaveformRing {
    cols: Box<[[AtomicU64; 2]]>,
    written: AtomicU64,
}

impl Default for WaveformRing {
    fn default() -> Self {
        Self {
            cols: (0..CAPACITY).map(|_| [AtomicU64::new(0), AtomicU64::new(0)]).collect(),
            written: AtomicU64::new(0),
        }
    }
}

impl WaveformRing {
    /// Append a finished column (audio thread).
    #[inline]
    pub fn push(&self, c: Column) {
        let w = self.written.load(Ordering::Relaxed);
        let slot = &self.cols[(w % CAPACITY as u64) as usize];
        let [a, b] = c.pack();
        slot[0].store(a, Ordering::Relaxed);
        slot[1].store(b, Ordering::Relaxed);
        self.written.store(w + 1, Ordering::Release);
    }

    /// Columns written so far.
    pub fn written(&self) -> u64 {
        self.written.load(Ordering::Acquire)
    }

    /// Columns with index ≥ `since` (at most the last `max`), plus the next index to ask for.
    pub fn read_since(&self, since: u64, max: usize) -> (u64, Vec<Column>) {
        let end = self.written();
        // Leave a safety margin so the writer can't overwrite what we're reading.
        let oldest = end.saturating_sub((CAPACITY / 2).min(max) as u64);
        let start = since.clamp(oldest, end);
        let out = (start..end)
            .map(|i| {
                let s = &self.cols[(i % CAPACITY as u64) as usize];
                Column::unpack([s[0].load(Ordering::Relaxed), s[1].load(Ordering::Relaxed)])
            })
            .collect();
        (end, out)
    }
}

/// Builds columns on the audio thread, all aligned with the output that's playing now:
/// - input peaks are delayed by the whole chain latency;
/// - a module's gain reduction describes *its own* output block, which reaches the chain output
///   after the lookahead of the slots behind it, so it is delayed by that much;
/// - output peaks need no delay.
#[derive(Debug)]
pub struct ColumnBuilder {
    column_len: usize,
    count: usize,
    in_peak: f32,
    out_peak: f32,
    gr: [f32; N_GR],
    /// Recent columns' input peaks and reductions, for the delayed read-out.
    in_hist: [f32; HISTORY],
    gr_hist: [[f32; N_GR]; HISTORY],
    in_delay: usize,
    gr_delay: [usize; N_GR],
    n: u64,
}

impl ColumnBuilder {
    /// `slots`: `(module id, lookahead samples)` of every chain slot in order (not real-time:
    /// call before handing the builder to the audio thread).
    pub fn new(sample_rate: f64, slots: &[(ModuleId, usize)]) -> Self {
        let column_len = ((sample_rate / COLUMNS_PER_SECOND as f64).round() as usize).max(1);
        let cols = |samples: usize| ((samples as f64 / column_len as f64).round() as usize).min(HISTORY - 1);
        let total: usize = slots.iter().map(|s| s.1).sum();
        let mut gr_delay = [0; N_GR];
        let mut cum = 0;
        for &(id, lat) in slots {
            cum += lat;
            if let Some(row) = GR_MODULES.iter().position(|m| *m == id) {
                gr_delay[row] = cols(total - cum);
            }
        }
        Self {
            column_len,
            count: 0,
            in_peak: 0.0,
            out_peak: 0.0,
            gr: [0.0; N_GR],
            in_hist: [0.0; HISTORY],
            gr_hist: [[0.0; N_GR]; HISTORY],
            in_delay: cols(total),
            gr_delay,
            n: 0,
        }
    }

    /// Feed one processed chunk (audio thread): its input and output peaks, the gain reduction
    /// each [`GR_MODULES`] module reported for it (0 for "nothing removed", e.g. under bypass),
    /// and its length. Pushes finished columns to `ring` (several if the chunk is longer than a
    /// column).
    #[inline]
    pub fn feed(&mut self, in_peak: f32, out_peak: f32, gr: &[f32; N_GR], frames: usize, ring: &WaveformRing) {
        self.in_peak = self.in_peak.max(in_peak);
        self.out_peak = self.out_peak.max(out_peak);
        for (a, g) in self.gr.iter_mut().zip(gr) {
            *a = a.max(*g);
        }
        self.count += frames;
        while self.count >= self.column_len {
            self.count -= self.column_len;
            let slot = (self.n % HISTORY as u64) as usize;
            self.in_hist[slot] = self.in_peak;
            self.gr_hist[slot] = self.gr;
            let ago = |d: usize| ((self.n + HISTORY as u64 - d as u64) % HISTORY as u64) as usize;
            ring.push(Column {
                peak: self.in_hist[ago(self.in_delay)],
                out_peak: self.out_peak,
                gain_reduction_db: core::array::from_fn(|i| self.gr_hist[ago(self.gr_delay[i])][i]),
            });
            self.n += 1;
            // Samples of this chunk left over belong to the next column too.
            let rest = self.count > 0;
            self.in_peak = if rest { in_peak } else { 0.0 };
            self.out_peak = if rest { out_peak } else { 0.0 };
            self.gr = if rest { *gr } else { [0.0; N_GR] };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(peak: f32, gr: [f32; N_GR]) -> Column {
        Column { peak, out_peak: peak * 0.5, gain_reduction_db: gr }
    }

    #[test]
    fn pack_roundtrip() {
        for c in [col(0.0, [0.0; N_GR]), col(1.0, [63.75, 0.25, 12.0, 30.5, 1.0]), col(0.5, [0.0, 0.0, 0.0, 6.0, 0.0])]
        {
            let u = Column::unpack(c.pack());
            assert!((u.peak - c.peak).abs() < 1e-4);
            assert!((u.out_peak - c.out_peak).abs() < 1e-4);
            assert_eq!(u.gain_reduction_db, c.gain_reduction_db);
            assert_eq!(u.removed(), c.removed());
        }
        let clamped = Column::unpack(
            Column { peak: 7.0, out_peak: -1.0, gain_reduction_db: [100.0, -3.0, f32::NAN, 0.1, 0.13] }.pack(),
        );
        assert_eq!((clamped.peak, clamped.out_peak), (1.0, 0.0));
        assert_eq!(clamped.gain_reduction_db, [63.75, 0.0, 0.0, 0.0, 0.25], "quantised to 0.25 dB, clamped");
    }

    #[test]
    fn read_since_returns_new_columns_only() {
        let ring = WaveformRing::default();
        for i in 0..10 {
            ring.push(col(i as f32 / 10.0, if i == 3 { [0.0, 0.0, 0.0, 9.0, 0.0] } else { [0.0; N_GR] }));
        }
        let (next, cols) = ring.read_since(0, 1000);
        assert_eq!((next, cols.len()), (10, 10));
        assert!(cols[3].removed() && !cols[4].removed());
        ring.push(col(0.9, [0.0; N_GR]));
        let (next, cols) = ring.read_since(next, 1000);
        assert_eq!((next, cols.len()), (11, 1));
        let (_, cols) = ring.read_since(0, 4);
        assert_eq!(cols.len(), 4, "capped to the most recent `max`");
    }

    #[test]
    fn builder_aligns_input_and_each_module_with_the_output() {
        // 48 kHz → 800-sample columns. Slots: de-hum (0), de-smack (1600 = 2 columns),
        // de-ess (800 = 1 column). Total latency 3 columns.
        let ring = WaveformRing::default();
        let slots = [(ModuleId::DeHum, 0), (ModuleId::DeSmack, 1600), (ModuleId::DeEss, 800)];
        let mut b = ColumnBuilder::new(48_000.0, &slots);
        let smack = GR_MODULES.iter().position(|m| *m == ModuleId::DeSmack).unwrap();
        let hum = GR_MODULES.iter().position(|m| *m == ModuleId::DeHum).unwrap();
        let ess = GR_MODULES.iter().position(|m| *m == ModuleId::DeEss).unwrap();
        // Column 0: a loud input event. De-smack ducks it at its output (column 2), de-hum at
        // column 0, de-ess at column 3 — all reach the chain output in column 3.
        let mut gr = [[0.0; N_GR]; 6];
        gr[0][hum] = 3.0;
        gr[2][smack] = 12.0;
        gr[3][ess] = 6.0;
        for (i, g) in gr.iter().enumerate() {
            b.feed(if i == 0 { 0.9 } else { 0.1 }, if i == 3 { 0.2 } else { 0.05 }, g, 800, &ring);
        }
        let (_, cols) = ring.read_since(0, 100);
        assert_eq!(cols.len(), 6);
        let c = cols[3];
        assert!((c.peak - 0.9).abs() < 1e-4, "input shown 3 columns late: {cols:?}");
        assert!((c.out_peak - 0.2).abs() < 1e-4);
        assert_eq!((c.gain_reduction_db[hum], c.gain_reduction_db[smack], c.gain_reduction_db[ess]), (3.0, 12.0, 6.0));
        assert!(cols.iter().enumerate().all(|(i, c)| i == 3 || !c.removed()), "{cols:?}");
    }

    #[test]
    fn long_chunks_emit_several_columns() {
        let ring = WaveformRing::default();
        let mut b = ColumnBuilder::new(48_000.0, &[]);
        b.feed(0.5, 0.5, &[0.0; N_GR], 4096, &ring);
        assert_eq!(ring.written(), 5, "4096 frames = 5 whole 800-sample columns");
        let (_, cols) = ring.read_since(0, 100);
        assert!(cols.iter().all(|c| c.out_peak == c.peak && (c.peak - 0.5).abs() < 1e-4), "no holes: {cols:?}");
    }
}
