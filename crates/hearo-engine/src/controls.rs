//! Lock-free control plane: the UI/CLI writes, the audio thread reads at the start of each
//! IOProc cycle. Only atomics — no locks, no allocation on either side.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Maximum chain slots controllable from outside.
pub const MAX_SLOTS: usize = 16;
/// Maximum parameters per slot.
pub const MAX_PARAMS: usize = 8;

/// Shared control state. Create once, share via `Arc`.
#[derive(Debug)]
pub struct Controls {
    bypass: AtomicBool,
    enabled: [AtomicBool; MAX_SLOTS],
    params: [[AtomicU32; MAX_PARAMS]; MAX_SLOTS],
    /// Bumped on every parameter write so the audio thread can skip unchanged params cheaply.
    param_gen: AtomicU32,
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            bypass: AtomicBool::new(false),
            enabled: std::array::from_fn(|_| AtomicBool::new(true)),
            params: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU32::new(f32::NAN.to_bits()))),
            param_gen: AtomicU32::new(0),
        }
    }
}

impl Controls {
    /// Global bypass (A/B).
    pub fn set_bypass(&self, on: bool) {
        self.bypass.store(on, Ordering::Relaxed);
    }

    /// Current bypass target.
    pub fn bypass(&self) -> bool {
        self.bypass.load(Ordering::Relaxed)
    }

    /// Enable/disable chain slot `slot` (ignored if out of range).
    pub fn set_enabled(&self, slot: usize, on: bool) {
        if let Some(e) = self.enabled.get(slot) {
            e.store(on, Ordering::Relaxed);
        }
    }

    /// Whether slot `slot` should be enabled.
    pub fn enabled(&self, slot: usize) -> bool {
        self.enabled.get(slot).is_some_and(|e| e.load(Ordering::Relaxed))
    }

    /// Set a parameter value (NaN = "leave the module's own value").
    pub fn set_param(&self, slot: usize, param: usize, value: f32) {
        if let Some(p) = self.params.get(slot).and_then(|s| s.get(param)) {
            p.store(value.to_bits(), Ordering::Relaxed);
            self.param_gen.fetch_add(1, Ordering::Release);
        }
    }

    /// Requested parameter value (NaN if never set).
    pub fn param(&self, slot: usize, param: usize) -> f32 {
        self.params.get(slot).and_then(|s| s.get(param)).map_or(f32::NAN, |p| f32::from_bits(p.load(Ordering::Relaxed)))
    }

    /// Parameter generation counter.
    pub fn param_generation(&self) -> u32 {
        self.param_gen.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_bounds() {
        let c = Controls::default();
        assert!(!c.bypass());
        assert!(c.enabled(0));
        assert!(!c.enabled(MAX_SLOTS));
        assert!(c.param(0, 0).is_nan());
        let g = c.param_generation();
        c.set_param(1, 2, -3.0);
        c.set_param(MAX_SLOTS, 0, 1.0); // ignored
        assert_eq!(c.param(1, 2), -3.0);
        assert_eq!(c.param_generation(), g + 1);
    }
}
