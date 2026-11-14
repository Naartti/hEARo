//! The [`Module`] trait every processing stage implements.

use crate::{AudioBlock, TelemetryWriter};

/// Identifies a processing module. Order matches the default chain order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum ModuleId {
    /// DC and mains-hum removal.
    DeHum = 0,
    /// Low-frequency "p"/"b" pop reduction.
    DePlosive,
    /// Short impulsive click repair.
    DeClick,
    /// ML mouth-noise removal.
    DeSmack,
    /// Sibilance control.
    DeEss,
    /// Always-on true-peak safety limiter.
    Limiter,
    /// Utility modules (gain, test helpers). Not shown as a user module.
    Utility,
}

impl ModuleId {
    /// Number of module ids (for fixed-size per-module arrays).
    pub const COUNT: usize = 7;
    /// All ids in chain order.
    pub const ALL: [ModuleId; Self::COUNT] =
        [Self::DeHum, Self::DePlosive, Self::DeClick, Self::DeSmack, Self::DeEss, Self::Limiter, Self::Utility];

    /// Index into per-module arrays.
    #[inline]
    pub fn index(self) -> usize {
        self as usize
    }

    /// Short, stable key used by the CLI and settings (`dehum`, `deess`, ...).
    pub fn key(self) -> &'static str {
        match self {
            Self::DeHum => "dehum",
            Self::DePlosive => "deplosive",
            Self::DeClick => "declick",
            Self::DeSmack => "desmack",
            Self::DeEss => "deess",
            Self::Limiter => "limiter",
            Self::Utility => "utility",
        }
    }

    /// Human-readable name for UIs and reports.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::DeHum => "De-hum",
            Self::DePlosive => "De-plosive",
            Self::DeClick => "De-click",
            Self::DeSmack => "De-smack",
            Self::DeEss => "De-ess",
            Self::Limiter => "Limiter",
            Self::Utility => "Utility",
        }
    }

    /// One-sentence description for UIs.
    pub fn description(self) -> &'static str {
        match self {
            Self::DeHum => "Removes DC offset and 50/60 Hz mains hum with its harmonics.",
            Self::DePlosive => "Softens low \"p\"/\"b\" pops and breath thumps.",
            Self::DeClick => "Repairs tiny mouth and tongue clicks inside speech.",
            Self::DeSmack => "Turns down lip smacks in the pauses between words.",
            Self::DeEss => "Tames harsh \"s\" and \"sh\" sounds.",
            Self::Limiter => "Always-on safety limiter that keeps peaks below the ceiling.",
            Self::Utility => "Utility stage for tests and level matching.",
        }
    }

    /// Cleanup modules (what hEARo *removes*) in chain order: the ones whose gain reduction the
    /// waveform colours. Excludes the limiter (a safety net, not cleanup) and utilities.
    pub const CLEANUP: [ModuleId; 5] = [Self::DeHum, Self::DePlosive, Self::DeClick, Self::DeSmack, Self::DeEss];

    /// The module with this key, if any.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|id| id.key() == key)
    }
}

/// Unit of a parameter, for musician-friendly display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamUnit {
    /// Hertz.
    Hz,
    /// Decibels.
    Db,
    /// Milliseconds.
    Ms,
    /// Ratio (e.g. 4:1).
    Ratio,
    /// Unitless 0..1 amount shown as percent.
    Percent,
    /// No unit.
    None,
}

/// Static description of one module parameter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamDesc {
    /// Stable key (`threshold`, `freq`, ...).
    pub key: &'static str,
    /// Display name.
    pub name: &'static str,
    /// Unit.
    pub unit: ParamUnit,
    /// Minimum value.
    pub min: f32,
    /// Maximum value.
    pub max: f32,
    /// Default value.
    pub default: f32,
    /// Resolution for UIs: values are multiples of `step` above `min`; 0 = continuous.
    pub step: f32,
    /// Discrete choices (value + label); empty = a range. When set, only these values exist.
    pub choices: &'static [ParamChoice],
}

/// One labelled value of a discrete parameter (e.g. de-hum mains: Auto / 50 Hz / 60 Hz).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamChoice {
    /// The parameter value.
    pub value: f32,
    /// Display label.
    pub label: &'static str,
}

impl ParamDesc {
    /// Clamp a value into this parameter's range (NaN maps to the default).
    pub fn clamp(&self, v: f32) -> f32 {
        if v.is_nan() { self.default } else { v.clamp(self.min, self.max) }
    }

    /// [`clamp`](Self::clamp), then snap to the nearest choice or `step`: what settings and UIs
    /// store, so a saved value is exactly what the module uses.
    pub fn snap(&self, v: f32) -> f32 {
        let v = self.clamp(v);
        if let Some(c) = self.choices.iter().min_by(|a, b| (a.value - v).abs().total_cmp(&(b.value - v).abs())) {
            return c.value;
        }
        if self.step > 0.0 {
            return (self.min + ((v - self.min) / self.step).round() * self.step).clamp(self.min, self.max);
        }
        v
    }
}

/// A processing stage in the chain.
///
/// Enable/disable crossfading and latency compensation are handled by the
/// [`Chain`](crate::Chain), so a module only has to process audio.
///
/// # Real-time contract
/// `process`, `reset` and `set_param` are called on the audio thread and must not allocate,
/// lock, block, do I/O, log or panic. `prepare` may allocate.
pub trait Module: Send {
    /// Which module this is.
    fn id(&self) -> ModuleId;

    /// Allocate state for the given format. Called before any processing and whenever the
    /// format changes. `max_block` is the largest block `process` will ever see.
    fn prepare(&mut self, sample_rate: f32, max_block: usize, channels: usize);

    /// Clear all signal history (filters, lookahead buffers) without allocating.
    fn reset(&mut self);

    /// Fixed processing latency in samples (lookahead). Constant between `prepare` calls.
    fn latency_samples(&self) -> usize {
        0
    }

    /// Process one block in place.
    fn process(&mut self, block: &mut AudioBlock<'_>, telemetry: &mut TelemetryWriter);

    /// Parameter descriptions, in index order.
    fn params(&self) -> &[ParamDesc] {
        &[]
    }

    /// Set parameter `index` (out-of-range indices are ignored, values are clamped).
    fn set_param(&mut self, _index: usize, _value: f32) {}

    /// Current value of parameter `index` (NaN if out of range).
    fn param(&self, _index: usize) -> f32 {
        f32::NAN
    }

    /// Index of the one parameter a simple UI shows, meaning "how much is removed". `None` for
    /// modules without one (the limiter: its ceiling is an advanced setting).
    fn primary_param(&self) -> Option<usize> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_dense_and_ordered() {
        for (i, id) in ModuleId::ALL.iter().enumerate() {
            assert_eq!(id.index(), i);
        }
    }

    #[test]
    fn snap_to_step_and_choices() {
        let stepped = ParamDesc {
            key: "k",
            name: "K",
            unit: ParamUnit::None,
            min: 1.0,
            max: 8.0,
            default: 8.0,
            step: 1.0,
            choices: &[],
        };
        assert_eq!(stepped.snap(3.4), 3.0);
        assert_eq!(stepped.snap(99.0), 8.0);
        assert_eq!(stepped.snap(f32::NAN), 8.0);
        const C: &[ParamChoice] =
            &[ParamChoice { value: 0.0, label: "Auto" }, ParamChoice { value: 50.0, label: "50 Hz" }];
        let choice = ParamDesc { min: 0.0, max: 60.0, default: 0.0, choices: C, ..stepped };
        assert_eq!(choice.snap(30.0), 50.0);
        assert_eq!(choice.snap(10.0), 0.0);
        let cont = ParamDesc { step: 0.0, ..stepped };
        assert_eq!(cont.snap(3.4), 3.4);
    }

    #[test]
    fn keys_roundtrip() {
        for id in ModuleId::ALL {
            assert_eq!(ModuleId::from_key(id.key()), Some(id));
        }
        assert_eq!(ModuleId::from_key("nope"), None);
    }

    #[test]
    fn clamp_handles_nan() {
        let p = ParamDesc {
            key: "k",
            name: "K",
            unit: ParamUnit::Db,
            min: -1.0,
            max: 1.0,
            default: 0.5,
            step: 0.0,
            choices: &[],
        };
        assert_eq!(p.clamp(f32::NAN), 0.5);
        assert_eq!(p.clamp(5.0), 1.0);
    }
}
