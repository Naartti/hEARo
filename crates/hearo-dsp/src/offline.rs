//! Offline rendering of whole clips through a [`Chain`]: `hearo process` and the review
//! window's `render_clip` (hearo-ffi) share this, so both hear exactly what the live engine does.
//!
//! Not real-time code: it allocates the output and the column arrays up front.
//!
//! The output is latency-compensated: `latency` samples of silence flush the chain and the first
//! `latency` output samples are dropped, so output and input line up sample for sample and have
//! the same length. Columns (for waveforms) are on the same clip timeline:
//!
//! - input / output peak per column;
//! - per cleanup module ([`ModuleId::CLEANUP`]) the largest gain reduction (dB, ≥ 0) whose
//!   samples fall in the column. A module's meter describes the block of *its own output*; slot
//!   `i` outputs clip sample `s` at chain time `s + cum_i` (`cum_i` = lookahead of slots `0..=i`),
//!   so a block at chain time `[p, p + n)` is attributed to clip samples `[p − cum_i, p + n − cum_i)`.
//!   Under global bypass nothing is removed, so all reductions are 0.

use crate::{AudioBlock, Chain, Event, ModuleId};

/// Per-column overview of a rendered clip.
#[derive(Debug, Clone, PartialEq)]
pub struct Columns {
    /// Columns per second (as requested).
    pub per_second: f64,
    /// Peak |sample| of the input over all channels, per column.
    pub input_peak: Vec<f32>,
    /// Peak |sample| of the (latency-compensated) output, per column.
    pub output_peak: Vec<f32>,
    /// Largest gain reduction per column in dB (≥ 0), one row per [`ModuleId::CLEANUP`] module
    /// in that order. Modules not in the chain stay 0.
    pub gain_reduction_db: [Vec<f32>; ModuleId::CLEANUP.len()],
}

/// Result of [`render`].
#[derive(Debug, Clone, PartialEq)]
pub struct Rendered {
    /// Processed audio, one `Vec` per channel, same length as the input.
    pub channels: Vec<Vec<f32>>,
    /// Every event the modules reported (start in clip samples).
    pub events: Vec<Event>,
    /// Column overview, if asked for.
    pub columns: Option<Columns>,
}

/// Run `input` (planar channels of equal length) through `chain`, which must be freshly
/// prepared for `input.len()` channels. `block` is the processing block size (the live engine
/// uses its IO buffer, 64 frames at 48 kHz); `columns_per_second` adds a [`Columns`] overview.
pub fn render(chain: &mut Chain, input: &[Vec<f32>], block: usize, columns_per_second: Option<f64>) -> Rendered {
    let channels = input.len();
    let frames = input.first().map_or(0, Vec::len);
    let block = block.max(1);
    let latency = chain.latency_samples();
    let total = frames + latency;

    // Cumulative lookahead per slot, for cleanup modules only: (row, cum).
    let mut cum = 0;
    let mut gr_slots = Vec::new();
    for i in 0..chain.len() {
        let Some(m) = chain.module(i) else { continue };
        cum += m.latency_samples();
        if let Some(row) = ModuleId::CLEANUP.iter().position(|id| *id == m.id()) {
            gr_slots.push((row, m.id(), cum));
        }
    }
    let spc = columns_per_second.filter(|c| c.is_finite() && *c > 0.0).map(|c| chain.sample_rate() as f64 / c);
    let n_cols = spc.map_or(0, |s| (frames as f64 / s).ceil() as usize);
    let col_of = |s: usize| spc.map_or(0, |spc| ((s as f64 / spc) as usize).min(n_cols.saturating_sub(1)));
    let mut gr: [Vec<f32>; ModuleId::CLEANUP.len()] = core::array::from_fn(|_| vec![0.0; n_cols]);

    let mut out: Vec<Vec<f32>> = vec![Vec::with_capacity(total); channels];
    let mut buf = vec![0.0f32; channels * block];
    let mut events = Vec::new();
    let mut pos = 0;
    while pos < total {
        let n = block.min(total - pos);
        for (c, ch) in input.iter().enumerate() {
            for i in 0..n {
                buf[c * n + i] = ch.get(pos + i).copied().unwrap_or(0.0);
            }
        }
        chain.process(&mut AudioBlock::from_planar(&mut buf[..channels * n], channels));
        events.extend_from_slice(chain.telemetry().events());
        chain.telemetry_mut().clear_events();
        if n_cols > 0 && !chain.is_bypassed() {
            for &(row, id, cum) in &gr_slots {
                let db = chain.telemetry().meters(id).gain_reduction_db;
                // Clip samples this block's module output belongs to.
                let (s0, s1) = (pos.saturating_sub(cum), (pos + n).saturating_sub(cum).min(frames));
                if db > 0.0 && s1 > s0 {
                    for col in &mut gr[row][col_of(s0)..=col_of(s1 - 1)] {
                        *col = col.max(db);
                    }
                }
            }
        }
        for (c, o) in out.iter_mut().enumerate() {
            o.extend_from_slice(&buf[c * n..(c + 1) * n]);
        }
        pos += n;
    }
    for o in &mut out {
        o.drain(..latency);
    }

    let columns = spc.map(|spc| {
        let peaks = |chans: &[Vec<f32>]| {
            (0..n_cols)
                .map(|j| {
                    let a = (j as f64 * spc) as usize;
                    let b = (((j + 1) as f64 * spc) as usize).min(frames).max(a);
                    chans.iter().flat_map(|c| &c[a..b]).fold(0.0f32, |m, x| m.max(x.abs()))
                })
                .collect::<Vec<f32>>()
        };
        Columns {
            per_second: chain.sample_rate() as f64 / spc,
            input_peak: peaks(input),
            output_peak: peaks(&out),
            gain_reduction_db: gr,
        }
    });
    Rendered { channels: out, events, columns }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::utility::Lookahead;
    use crate::{Module, TelemetryWriter};

    /// Reports 6 dB of reduction on the block that outputs clip sample `at`.
    struct Marker {
        at: u64,
        pos: u64,
        delay: Lookahead,
    }
    impl Module for Marker {
        fn id(&self) -> ModuleId {
            ModuleId::DeSmack
        }
        fn prepare(&mut self, sr: f32, max_block: usize, ch: usize) {
            self.delay.prepare(sr, max_block, ch);
        }
        fn reset(&mut self) {
            self.pos = 0;
            self.delay.reset();
        }
        fn latency_samples(&self) -> usize {
            self.delay.latency_samples()
        }
        fn process(&mut self, block: &mut AudioBlock<'_>, t: &mut TelemetryWriter) {
            self.delay.process(block, t);
            let n = block.frames() as u64;
            let hit = (self.pos..self.pos + n).contains(&(self.at + 1500)); // 500 + own 1000 lookahead
            t.set_gain_reduction(ModuleId::DeSmack, if hit { 6.0 } else { 0.0 });
            self.pos += n;
        }
    }

    fn chain(at: u64) -> Chain {
        let mut c = Chain::new(vec![
            Box::new(Lookahead::new(500, 0.0)) as Box<dyn Module>,
            Box::new(Marker { at, pos: 0, delay: Lookahead::new(1000, 0.0) }),
            Box::new(Lookahead::new(700, 0.0)),
        ]);
        c.prepare(48_000.0, 64, 1);
        c
    }

    #[test]
    fn output_is_aligned_and_same_length() {
        let input: Vec<f32> = (0..10_000).map(|i| ((i * 7919) % 1000) as f32 / 1000.0 - 0.5).collect();
        let r = render(&mut chain(0), core::slice::from_ref(&input), 64, None);
        assert_eq!(r.channels, vec![input], "pure-latency chain renders bit-exact after compensation");
        assert!(r.columns.is_none());
    }

    #[test]
    fn gain_reduction_lands_in_the_right_column() {
        // 48 kHz at 100 columns/s = 480 samples per column; mark clip sample 4400 → column 9 (4320..4800).
        let input = vec![0.1f32; 48_000];
        let r = render(&mut chain(4400), &[input], 64, Some(100.0));
        let cols = r.columns.unwrap();
        assert_eq!(cols.input_peak.len(), 100);
        assert_eq!(cols.output_peak.len(), 100);
        let row = ModuleId::CLEANUP.iter().position(|id| *id == ModuleId::DeSmack).unwrap();
        let hit: Vec<usize> = (0..100).filter(|&j| cols.gain_reduction_db[row][j] > 0.0).collect();
        assert_eq!(hit, vec![9], "64-sample block containing sample 4400 lies in column 9");
        assert!(cols.gain_reduction_db.iter().enumerate().all(|(i, r)| i == row || r.iter().all(|&x| x == 0.0)));
        assert!((cols.output_peak[50] - 0.1).abs() < 1e-6);
    }

    #[test]
    fn bypass_reports_nothing_removed() {
        let mut c = chain(4400);
        c.set_bypass(true);
        c.prepare(48_000.0, 64, 1);
        let r = render(&mut c, &[vec![0.1f32; 48_000]], 64, Some(100.0));
        assert!(r.columns.unwrap().gain_reduction_db.iter().flatten().all(|&x| x == 0.0));
    }
}
