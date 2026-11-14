//! Utility modules: a smoothed gain stage and a pure-latency probe used to test the chain.

use crate::delay::MultiDelay;
use crate::module::{ParamDesc, ParamUnit};
use crate::{AudioBlock, Module, ModuleId, TelemetryWriter, db_to_amp};

const GAIN_PARAMS: [ParamDesc; 1] = [ParamDesc {
    key: "gain",
    name: "Gain",
    unit: ParamUnit::Db,
    min: -48.0,
    max: 24.0,
    default: 0.0,
    step: 0.0,
    choices: &[],
}];

/// Gain stage. Changes ramp linearly over a fixed 10 ms (sample-counted, carried across
/// blocks), so the output doesn't depend on block size and never zippers.
#[derive(Debug)]
pub struct Gain {
    gain_db: f32,
    current: f32,
    target: f32,
    step: f32,
    remaining: usize,
    ramp_len: usize,
}

impl Gain {
    /// Ramp length for parameter changes.
    pub const RAMP_MS: f32 = 10.0;

    /// A gain stage at `gain_db`.
    pub fn new(gain_db: f32) -> Self {
        let gain_db = GAIN_PARAMS[0].clamp(gain_db);
        let g = db_to_amp(gain_db);
        Self { gain_db, current: g, target: g, step: 0.0, remaining: 0, ramp_len: 480 }
    }
}

impl Module for Gain {
    fn id(&self) -> ModuleId {
        ModuleId::Utility
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize, _channels: usize) {
        self.ramp_len = ((sample_rate * Self::RAMP_MS / 1000.0).round() as usize).max(1);
    }

    fn reset(&mut self) {
        self.current = self.target;
        self.remaining = 0;
    }

    fn process(&mut self, block: &mut AudioBlock<'_>, _telemetry: &mut TelemetryWriter) {
        let n = block.frames();
        let mut i = 0;
        // Ramp portion, frame by frame.
        while self.remaining > 0 && i < n {
            self.remaining -= 1;
            self.current = if self.remaining == 0 { self.target } else { self.current + self.step };
            for c in 0..block.channels() {
                block.channel_mut(c)[i] *= self.current;
            }
            i += 1;
        }
        // Steady portion.
        let g = self.current;
        for c in block.channels_mut() {
            c[i..].iter_mut().for_each(|x| *x *= g);
        }
    }

    fn params(&self) -> &[ParamDesc] {
        &GAIN_PARAMS
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if index == 0 {
            self.gain_db = GAIN_PARAMS[0].clamp(value);
            self.target = db_to_amp(self.gain_db);
            self.remaining = self.ramp_len;
            self.step = (self.target - self.current) / self.ramp_len as f32;
        }
    }

    fn param(&self, index: usize) -> f32 {
        if index == 0 { self.gain_db } else { f32::NAN }
    }
}

/// A module with a fixed lookahead and a fixed gain. Exists to exercise latency compensation.
#[derive(Debug)]
pub struct Lookahead {
    delay: MultiDelay,
    samples: usize,
    gain: f32,
}

impl Lookahead {
    /// Delay by `samples` and scale by `gain_db`.
    pub fn new(samples: usize, gain_db: f32) -> Self {
        Self { delay: MultiDelay::default(), samples, gain: db_to_amp(gain_db) }
    }
}

impl Module for Lookahead {
    fn id(&self) -> ModuleId {
        ModuleId::Utility
    }

    fn prepare(&mut self, _sample_rate: f32, _max_block: usize, channels: usize) {
        self.delay.prepare(channels, self.samples);
    }

    fn reset(&mut self) {
        self.delay.reset();
    }

    fn latency_samples(&self) -> usize {
        self.samples
    }

    fn process(&mut self, block: &mut AudioBlock<'_>, _telemetry: &mut TelemetryWriter) {
        self.delay.process(block);
        let g = self.gain;
        for c in block.channels_mut() {
            c.iter_mut().for_each(|x| *x *= g);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gain_ramps_then_holds() {
        let mut g = Gain::new(0.0);
        g.prepare(400.0, 4, 1); // 10 ms ramp = 4 samples
        assert_eq!(g.ramp_len, 4);
        g.set_param(0, -6.0206);
        let mut t = TelemetryWriter::new();
        let mut data = vec![1.0; 4];
        g.process(&mut AudioBlock::from_planar(&mut data, 1), &mut t);
        assert!(data[0] < 1.0 && data[0] > 0.5);
        assert!((data[3] - 0.5).abs() < 1e-4);
        let mut data = vec![1.0; 4];
        g.process(&mut AudioBlock::from_planar(&mut data, 1), &mut t);
        assert!(data.iter().all(|x| (x - 0.5).abs() < 1e-4));
    }

    #[test]
    fn gain_clamps() {
        let mut g = Gain::new(100.0);
        assert_eq!(g.param(0), 24.0);
        g.set_param(0, f32::NAN);
        assert_eq!(g.param(0), 0.0);
    }
}
