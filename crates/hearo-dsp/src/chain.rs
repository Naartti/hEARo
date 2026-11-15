//! The processing [`Chain`]: runs modules in order with latency compensation, click-free
//! toggles and bit-exact global bypass.
//!
//! ```text
//!            ┌──────── slot 0 ────────┐   ┌──────── slot 1 ────────┐
//! input ──┬─▶│ module 0 ──▶ fade ◀── delay(L0) │──▶│ module 1 ──▶ fade ◀── delay(L1) │──┬──▶ fade ──▶ output
//!         │  └────────────────────────┘   └────────────────────────┘  │     ▲
//!         └──────────────────────── delay(L0 + L1) ───────────────────┼─────┘ (global bypass)
//! ```
//!
//! Every slot always runs a dry delay line equal to its module's latency, so the total chain
//! latency is constant no matter which modules are on. A disabled slot outputs its delayed
//! dry signal (a pure copy), so "all modules off" and "bypass" are bit-exact delays of the input.

use crate::delay::MultiDelay;
use crate::denormal::DenormalGuard;
use crate::fade::fade_gain;
use crate::{AudioBlock, MAX_CHANNELS, Module, ModuleId, TelemetryWriter};

/// Crossfade length for toggles and bypass, in milliseconds.
pub const FADE_MS: f32 = 10.0;

/// Where a slot is in its enable/disable life cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotState {
    /// Disabled; module not processed, output is the delayed dry signal.
    Off,
    /// Just enabled: module runs (filling its lookahead) but output is still dry.
    WarmingUp {
        /// Samples left before the module's output is valid.
        remaining: usize,
    },
    /// Crossfading from dry to processed.
    FadingIn {
        /// Position in the fade, in samples.
        pos: usize,
    },
    /// Enabled and fully processing.
    On,
    /// Crossfading from processed to dry.
    FadingOut {
        /// Position in the fade, in samples.
        pos: usize,
    },
}

impl SlotState {
    /// Whether the slot is heading towards (or at) enabled.
    pub fn target_on(self) -> bool {
        matches!(self, Self::WarmingUp { .. } | Self::FadingIn { .. } | Self::On)
    }
}

struct Slot {
    module: Box<dyn Module>,
    state: SlotState,
    dry: MultiDelay,
    /// The module's latency (set by `prepare`): audio reaching it at chain time `t` leaves the
    /// slot at `t + latency`.
    latency: usize,
}

/// The ordered processing chain. Owned by the audio thread once prepared.
pub struct Chain {
    slots: Vec<Slot>,
    sample_rate: f32,
    max_block: usize,
    channels: usize,
    fade_len: usize,
    latency: usize,
    scratch: Vec<f32>,
    bypass_scratch: Vec<f32>,
    bypass_delay: MultiDelay,
    bypass_target: bool,
    /// 0 = fully processed, `fade_len` = fully bypassed.
    bypass_pos: usize,
    telemetry: TelemetryWriter,
    sample_pos: u64,
    faults: u64,
    prepared: bool,
}

impl core::fmt::Debug for Chain {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Chain")
            .field("modules", &self.slots.iter().map(|s| (s.module.id(), s.state)).collect::<Vec<_>>())
            .field("sample_rate", &self.sample_rate)
            .field("channels", &self.channels)
            .field("latency", &self.latency)
            .field("bypass", &self.bypass_target)
            .finish_non_exhaustive()
    }
}

impl Chain {
    /// Build a chain; every module starts enabled. Call [`prepare`](Self::prepare) before use.
    pub fn new(modules: Vec<Box<dyn Module>>) -> Self {
        let slots = modules
            .into_iter()
            .map(|module| Slot { module, state: SlotState::On, dry: MultiDelay::default(), latency: 0 })
            .collect();
        Self {
            slots,
            sample_rate: 0.0,
            max_block: 0,
            channels: 0,
            fade_len: 0,
            latency: 0,
            scratch: Vec::new(),
            bypass_scratch: Vec::new(),
            bypass_delay: MultiDelay::default(),
            bypass_target: false,
            bypass_pos: 0,
            telemetry: TelemetryWriter::new(),
            sample_pos: 0,
            faults: 0,
            prepared: false,
        }
    }

    /// Allocate everything for a format. Not real-time safe. Settles any running fades.
    pub fn prepare(&mut self, sample_rate: f32, max_block: usize, channels: usize) {
        assert!(sample_rate > 0.0 && max_block > 0 && (1..=MAX_CHANNELS).contains(&channels));
        self.sample_rate = sample_rate;
        self.max_block = max_block;
        self.channels = channels;
        self.fade_len = ((sample_rate * FADE_MS / 1000.0).round() as usize).max(1);
        let mut total = 0;
        for slot in &mut self.slots {
            slot.module.prepare(sample_rate, max_block, channels);
            slot.module.reset();
            let l = slot.module.latency_samples();
            slot.dry.prepare(channels, l);
            slot.latency = l;
            slot.state = if slot.state.target_on() { SlotState::On } else { SlotState::Off };
            total += l;
        }
        self.latency = total;
        self.bypass_delay.prepare(channels, total);
        self.scratch = vec![0.0; channels * max_block];
        self.bypass_scratch = vec![0.0; channels * max_block];
        self.bypass_pos = if self.bypass_target { self.fade_len } else { 0 };
        self.sample_pos = 0;
        self.prepared = true;
    }

    /// Clear all signal history. Real-time safe.
    pub fn reset(&mut self) {
        for slot in &mut self.slots {
            slot.module.reset();
            slot.dry.reset();
        }
        self.bypass_delay.reset();
    }

    /// Total latency in samples (constant regardless of which modules are on).
    pub fn latency_samples(&self) -> usize {
        self.latency
    }

    /// Sample rate given to `prepare`.
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Number of modules.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// True if the chain has no modules.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Index of the first module with this id.
    pub fn find(&self, id: ModuleId) -> Option<usize> {
        self.slots.iter().position(|s| s.module.id() == id)
    }

    /// The module at `index`, if any.
    pub fn module(&self, index: usize) -> Option<&dyn Module> {
        self.slots.get(index).map(|s| s.module.as_ref())
    }

    /// Lifecycle state of slot `index`, if any.
    pub fn slot_state(&self, index: usize) -> Option<SlotState> {
        self.slots.get(index).map(|s| s.state)
    }

    /// Whether slot `index` is (heading towards) enabled. False for unknown indices.
    pub fn is_enabled(&self, index: usize) -> bool {
        self.slots.get(index).is_some_and(|s| s.state.target_on())
    }

    /// Enable or disable a module with a click-free crossfade. Real-time safe.
    pub fn set_enabled(&mut self, index: usize, on: bool) {
        let Some(slot) = self.slots.get_mut(index) else { return };
        let fade = self.fade_len;
        slot.state = match (slot.state, on) {
            (SlotState::Off, true) => {
                slot.module.reset();
                match slot.module.latency_samples() {
                    0 => SlotState::FadingIn { pos: 0 },
                    remaining => SlotState::WarmingUp { remaining },
                }
            }
            (SlotState::FadingOut { pos }, true) => SlotState::FadingIn { pos: fade.saturating_sub(pos) },
            (SlotState::On, false) => SlotState::FadingOut { pos: 0 },
            (SlotState::FadingIn { pos }, false) => SlotState::FadingOut { pos: fade.saturating_sub(pos) },
            (SlotState::WarmingUp { .. }, false) => SlotState::Off,
            (state, _) => state,
        };
    }

    /// Set a module parameter (clamped by the module). Real-time safe.
    pub fn set_param(&mut self, index: usize, param: usize, value: f32) {
        if let Some(slot) = self.slots.get_mut(index) {
            slot.module.set_param(param, value);
        }
    }

    /// Global bypass with a crossfade. Fully bypassed output is bit-exact delayed input;
    /// modules keep running so switching back (A/B) is instant.
    pub fn set_bypass(&mut self, on: bool) {
        self.bypass_target = on;
    }

    /// Whether global bypass is on (target).
    pub fn is_bypassed(&self) -> bool {
        self.bypass_target
    }

    /// Telemetry written by modules.
    pub fn telemetry(&self) -> &TelemetryWriter {
        &self.telemetry
    }

    /// Mutable telemetry (the engine drains events after each block).
    pub fn telemetry_mut(&mut self) -> &mut TelemetryWriter {
        &mut self.telemetry
    }

    /// Blocks where non-finite output was caught and replaced (should stay 0).
    pub fn faults(&self) -> u64 {
        self.faults
    }

    /// Process a block in place. Any size is accepted (split into `max_block` chunks).
    /// Real-time safe. An unprepared chain or channel mismatch passes audio through untouched.
    pub fn process(&mut self, block: &mut AudioBlock<'_>) {
        if !self.prepared || block.channels() != self.channels {
            self.faults += 1;
            return;
        }
        let _ftz = DenormalGuard::new();
        let frames = block.frames();
        let mut start = 0;
        while start < frames {
            let n = self.max_block.min(frames - start);
            let mut sub = block.slice_mut(start, n);
            self.process_chunk(&mut sub);
            start += n;
        }
    }

    fn process_chunk(&mut self, block: &mut AudioBlock<'_>) {
        let ch = self.channels;
        let n = block.frames();
        let fade = self.fade_len;

        // Non-finite input never enters the delay lines (it would linger for `latency` samples).
        if !block.is_finite() {
            self.faults += 1;
            for c in block.channels_mut() {
                for x in c.iter_mut() {
                    if !x.is_finite() {
                        *x = 0.0;
                    }
                }
            }
        }

        let mut bypass_dry = AudioBlock::from_planar(&mut self.bypass_scratch[..ch * n], ch);
        bypass_dry.copy_from(block);
        self.bypass_delay.process(&mut bypass_dry);

        self.telemetry.set_block_start(self.sample_pos);
        for slot in &mut self.slots {
            let mut dry = AudioBlock::from_planar(&mut self.scratch[..ch * n], ch);
            dry.copy_from(block);
            slot.dry.process(&mut dry);
            let was_active = slot.state != SlotState::Off;
            let mark = self.telemetry.event_count();
            match slot.state {
                SlotState::Off => {
                    block.copy_from(&dry);
                    // Nothing is removed by a slot that doesn't run (its meter would go stale).
                    self.telemetry.set_gain_reduction(slot.module.id(), 0.0);
                }
                SlotState::On => slot.module.process(block, &mut self.telemetry),
                SlotState::WarmingUp { remaining } => {
                    slot.module.process(block, &mut self.telemetry);
                    block.copy_from(&dry);
                    // Output is still dry: nothing was removed yet. Events are for audio that
                    // leaves `latency` later; keep those that leave once the fade-in can start
                    // (at a chunk boundary: this whole chunk is dry).
                    self.telemetry.set_gain_reduction(slot.module.id(), 0.0);
                    let (exit, wet_from) = (slot.latency as u64, self.sample_pos + remaining.max(n) as u64);
                    self.telemetry.retain_events_since(mark, |e| {
                        e.start_sample.saturating_add(e.duration_samples as u64 + exit) > wet_from
                    });
                    slot.state = if remaining <= n {
                        SlotState::FadingIn { pos: 0 }
                    } else {
                        SlotState::WarmingUp { remaining: remaining - n }
                    };
                }
                SlotState::FadingIn { pos } => {
                    slot.module.process(block, &mut self.telemetry);
                    crossfade(block, &dry, |i| fade_gain(pos + i, fade));
                    slot.state = if pos + n >= fade { SlotState::On } else { SlotState::FadingIn { pos: pos + n } };
                }
                SlotState::FadingOut { pos } => {
                    slot.module.process(block, &mut self.telemetry);
                    crossfade(block, &dry, |i| 1.0 - fade_gain(pos + i, fade));
                    // Audio leaving after the fade ends plays dry: its events report nothing
                    // removed (a lookahead module still detects while fading out).
                    let (exit, dry_from) = (slot.latency as u64, self.sample_pos + fade.saturating_sub(pos) as u64);
                    self.telemetry.retain_events_since(mark, |e| e.start_sample.saturating_add(exit) < dry_from);
                    slot.state = if pos + n >= fade { SlotState::Off } else { SlotState::FadingOut { pos: pos + n } };
                }
            }
            // A module that produced NaN/Inf is replaced by its (finite) dry signal and reset.
            if was_active && !block.is_finite() {
                self.faults += 1;
                block.copy_from(&dry);
                slot.module.reset();
                // Its events this block removed nothing (and the reset emptied its lookahead).
                self.telemetry.retain_events_since(mark, |_| false);
                // Re-enter through warm-up + fade so a lookahead module never outputs its
                // emptied buffer (no dropout, no click).
                slot.state = if slot.state.target_on() {
                    match slot.module.latency_samples() {
                        0 => SlotState::FadingIn { pos: 0 },
                        remaining => SlotState::WarmingUp { remaining },
                    }
                } else {
                    SlotState::Off
                };
            }
        }

        // Global bypass.
        if self.bypass_target && self.bypass_pos >= fade {
            block.copy_from(&bypass_dry);
        } else if self.bypass_target || self.bypass_pos > 0 {
            let mut pos = self.bypass_pos;
            let step_up = self.bypass_target;
            for i in 0..n {
                let g = fade_gain(pos, fade);
                for c in 0..ch {
                    let wet = block.channel(c)[i];
                    let d = bypass_dry.channel(c)[i];
                    block.channel_mut(c)[i] = wet * (1.0 - g) + d * g;
                }
                pos = if step_up { (pos + 1).min(fade) } else { pos.saturating_sub(1) };
            }
            self.bypass_pos = pos;
        }

        // Last line of defence (e.g. overflow in the bypass mix): never emit NaN/Inf.
        if !block.is_finite() {
            self.faults += 1;
            block.copy_from(&bypass_dry);
        }

        self.sample_pos += n as u64;
    }
}

/// `block = block * g(i) + dry * (1 - g(i))` where `g` is the processed-signal gain.
#[inline]
fn crossfade(block: &mut AudioBlock<'_>, dry: &AudioBlock<'_>, g: impl Fn(usize) -> f32) {
    for c in 0..block.channels() {
        let d = dry.channel(c);
        for (i, x) in block.channel_mut(c).iter_mut().enumerate() {
            let w = g(i);
            *x = *x * w + d[i] * (1.0 - w);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::utility::{Gain, Lookahead};

    fn chain_with(modules: Vec<Box<dyn Module>>) -> Chain {
        let mut c = Chain::new(modules);
        c.prepare(48_000.0, 64, 1);
        c
    }

    fn run(c: &mut Chain, input: &[f32], block: usize) -> Vec<f32> {
        let mut out = input.to_vec();
        for chunk in out.chunks_mut(block) {
            let mut b = AudioBlock::from_planar(chunk, 1);
            c.process(&mut b);
        }
        out
    }

    #[test]
    fn latency_is_sum_of_modules() {
        let c = chain_with(vec![Box::new(Lookahead::new(10, 0.0)), Box::new(Lookahead::new(7, 0.0))]);
        assert_eq!(c.latency_samples(), 17);
    }

    #[test]
    fn disabled_module_keeps_alignment() {
        let mut c = chain_with(vec![Box::new(Lookahead::new(10, -6.0)), Box::new(Lookahead::new(5, 0.0))]);
        c.set_enabled(0, false);
        // Let the fade finish.
        let _ = run(&mut c, &vec![0.0; 1000], 64);
        let mut input = vec![0.0; 100];
        input[0] = 1.0;
        let out = run(&mut c, &input, 64);
        assert_eq!(out[15], 1.0, "impulse must arrive after exactly 15 samples, unscaled");
    }

    #[test]
    fn enable_goes_through_warmup_and_fade() {
        let mut c = chain_with(vec![Box::new(Lookahead::new(100, -6.0))]);
        c.set_enabled(0, false);
        let _ = run(&mut c, &vec![0.0; 1000], 64);
        assert_eq!(c.slot_state(0), Some(SlotState::Off));
        c.set_enabled(0, true);
        assert_eq!(c.slot_state(0), Some(SlotState::WarmingUp { remaining: 100 }));
        let _ = run(&mut c, &vec![0.0; 2000], 64);
        assert_eq!(c.slot_state(0), Some(SlotState::On));
    }

    #[test]
    fn reversing_mid_fade_is_symmetric() {
        let mut c = chain_with(vec![Box::new(Gain::new(-6.0))]);
        c.set_enabled(0, false);
        let _ = run(&mut c, &[0.0; 64], 64);
        assert_eq!(c.slot_state(0), Some(SlotState::FadingOut { pos: 64 }));
        c.set_enabled(0, true);
        assert_eq!(c.slot_state(0), Some(SlotState::FadingIn { pos: 480 - 64 }));
    }

    #[test]
    fn channel_mismatch_passes_through() {
        let mut c = chain_with(vec![Box::new(Gain::new(-6.0))]);
        let mut data = vec![1.0; 8];
        let mut b = AudioBlock::from_planar(&mut data, 2);
        c.process(&mut b);
        assert_eq!(data, vec![1.0; 8]);
        assert_eq!(c.faults(), 1);
    }

    #[test]
    fn nan_input_is_zeroed_once_and_does_not_linger() {
        let make = || chain_with(vec![Box::new(Lookahead::new(300, -6.0)), Box::new(Gain::new(-3.0))]);
        let input: Vec<f32> = (0..10_000).map(|i| (i as f32 * 0.01).sin() * 0.5).collect();
        let mut poisoned = input.clone();
        poisoned[100] = f32::NAN;
        poisoned[101] = f32::INFINITY;
        let mut reference = input.clone();
        reference[100] = 0.0;
        reference[101] = 0.0;
        let mut c = make();
        let out = run(&mut c, &poisoned, 64);
        let expected = run(&mut make(), &reference, 64);
        assert_eq!(out, expected, "bad samples must behave exactly like zeros");
        assert_eq!(c.faults(), 1, "one poisoned chunk = one fault, no lingering re-faults");
    }

    /// A module that emits NaN on its second block only.
    struct Poison(u32);
    impl Module for Poison {
        fn id(&self) -> ModuleId {
            ModuleId::Utility
        }
        fn prepare(&mut self, _: f32, _: usize, _: usize) {}
        fn reset(&mut self) {}
        fn process(&mut self, block: &mut AudioBlock<'_>, _: &mut TelemetryWriter) {
            self.0 += 1;
            if self.0 == 2 {
                block.channel_mut(0)[0] = f32::NAN;
            }
        }
    }

    #[test]
    fn faulty_module_is_isolated_and_reset() {
        let mut c = chain_with(vec![Box::new(Gain::new(-6.0)), Box::new(Poison(0))]);
        let out = run(&mut c, &[0.5; 256], 64);
        assert!(out.iter().all(|x| x.is_finite()));
        assert_eq!(c.faults(), 1, "exactly the poisoned block faulted");
        assert!((out[255] - 0.5 * 0.501_187).abs() < 1e-5, "gain module unaffected");
    }

    #[test]
    fn nan_on_last_fade_out_block_is_still_caught() {
        // Poison fires on its 2nd block; disabling first makes that block the final fade-out
        // block (fade = 480 = 7.5 × 64, so block 2 of 64 frames is mid-fade; use 480-frame blocks).
        let mut c = chain_with(vec![Box::new(Poison(0)), Box::new(Gain::new(-6.0))]);
        c.prepare(48_000.0, 480, 1);
        let _ = run(&mut c, &[0.5; 480], 480); // block 1: clean
        c.set_enabled(0, false);
        let out = run(&mut c, &[0.5; 480], 480); // block 2: poisoned, and the whole fade-out
        assert_eq!(c.slot_state(0), Some(SlotState::Off));
        assert!(out.iter().all(|x| x.is_finite()), "NaN leaked on the last fade-out block");
        assert_eq!(c.faults(), 1);
    }

    #[test]
    fn faulting_lookahead_module_rewarms_instead_of_dropping_out() {
        let mut c = chain_with(vec![Box::new(Lookahead::new(100, 0.0)), Box::new(Poison(0))]);
        let _ = run(&mut c, &[0.5; 128], 64);
        assert_eq!(c.faults(), 1);
        assert!(matches!(c.slot_state(1), Some(SlotState::FadingIn { .. } | SlotState::On)));
    }

    #[test]
    fn disabled_slot_reports_no_gain_reduction() {
        /// Reports a constant 9 dB of gain reduction whenever it runs.
        struct Meter;
        impl Module for Meter {
            fn id(&self) -> ModuleId {
                ModuleId::DeSmack
            }
            fn prepare(&mut self, _: f32, _: usize, _: usize) {}
            fn reset(&mut self) {}
            fn process(&mut self, _: &mut AudioBlock<'_>, t: &mut TelemetryWriter) {
                t.set_gain_reduction(ModuleId::DeSmack, 9.0);
            }
        }
        let mut c = chain_with(vec![Box::new(Meter)]);
        let _ = run(&mut c, &[0.1; 64], 64);
        assert_eq!(c.telemetry().meters(ModuleId::DeSmack).gain_reduction_db, 9.0);
        c.set_enabled(0, false);
        let _ = run(&mut c, &[0.1; 1024], 64);
        assert_eq!(c.slot_state(0), Some(SlotState::Off));
        assert_eq!(c.telemetry().meters(ModuleId::DeSmack).gain_reduction_db, 0.0, "stale meter after disabling");
    }

    /// Delays by `latency` and reports one event per block, for the block's audio shifted back
    /// by `back` samples: `back = 0` is a lookahead detector (the audio entering now, which
    /// leaves `latency` later); `back = latency` reports the audio leaving now.
    struct Reporter {
        inner: Lookahead,
        back: u64,
    }
    impl Reporter {
        fn new(latency: usize, back: u64) -> Self {
            Self { inner: Lookahead::new(latency, 0.0), back }
        }
    }
    impl Module for Reporter {
        fn id(&self) -> ModuleId {
            ModuleId::DeSmack
        }
        fn prepare(&mut self, sr: f32, max_block: usize, ch: usize) {
            self.inner.prepare(sr, max_block, ch);
        }
        fn reset(&mut self) {
            self.inner.reset();
        }
        fn latency_samples(&self) -> usize {
            self.inner.latency_samples()
        }
        fn process(&mut self, block: &mut AudioBlock<'_>, t: &mut TelemetryWriter) {
            self.inner.process(block, t);
            t.push_event(crate::Event {
                module: ModuleId::DeSmack,
                kind: crate::EventKind::Smack,
                start_sample: t.block_start().saturating_sub(self.back),
                duration_samples: block.frames() as u32,
                energy_removed_db: 6.0,
            });
        }
    }

    /// Run `frames` of silence in 64-frame blocks; (events reported, triggers counted) meanwhile.
    fn run_counting(c: &mut Chain, frames: usize) -> (usize, u64) {
        let before = c.telemetry().meters(ModuleId::DeSmack).triggers;
        let mut events = 0;
        for _ in 0..frames / 64 {
            let mut data = [0.0f32; 64];
            c.process(&mut AudioBlock::from_planar(&mut data, 1));
            events += c.telemetry().events().len();
            c.telemetry_mut().clear_events();
        }
        (events, c.telemetry().meters(ModuleId::DeSmack).triggers - before)
    }

    /// Fading out, a lookahead module still detects, but that audio leaves the slot after the
    /// fade has ended (latency 960 > fade 480): it is never removed, so it is not reported
    /// (RT review 2026-11-14: spurious auto-saved catches). Triggers stay in step with events.
    #[test]
    fn fade_out_drops_events_for_audio_that_leaves_dry() {
        let mut c = chain_with(vec![Box::new(Reporter::new(960, 0))]);
        assert_eq!(run_counting(&mut c, 640), (10, 10));
        c.set_enabled(0, false);
        assert_eq!(run_counting(&mut c, 1024), (0, 0), "events reported for audio that played dry");
        assert_eq!(c.slot_state(0), Some(SlotState::Off));
    }

    /// Without lookahead the audio fading out is still partly processed: its events stay.
    #[test]
    fn fade_out_keeps_events_for_audio_still_partly_processed() {
        let mut c = chain_with(vec![Box::new(Reporter::new(0, 0))]);
        let _ = run_counting(&mut c, 640);
        c.set_enabled(0, false);
        // Fade 480 = 7.5 blocks: 8 blocks have some processed signal.
        assert_eq!(run_counting(&mut c, 1024), (8, 8));
    }

    /// Warming up, the output is dry: events for audio that leaves the slot during warm-up are
    /// dropped; a lookahead detector's events (audio leaving after warm-up) are kept.
    #[test]
    fn warm_up_drops_only_events_for_audio_that_leaves_dry() {
        for (back, expected) in [(128, 0), (0, 2)] {
            let mut c = chain_with(vec![Box::new(Reporter::new(128, back))]);
            c.set_enabled(0, false);
            let _ = run_counting(&mut c, 1024);
            c.set_enabled(0, true);
            assert_eq!(run_counting(&mut c, 128), (expected, expected as u64), "back {back}");
            assert!(matches!(c.slot_state(0), Some(SlotState::FadingIn { .. })));
        }
    }

    /// Warm-up ends on a chunk boundary: a chunk where `remaining` runs out is still all dry,
    /// so events for audio leaving anywhere in it are dropped too (review finding).
    #[test]
    fn warm_up_ending_mid_chunk_still_drops_that_chunks_events() {
        let mut c = chain_with(vec![Box::new(Reporter::new(100, 100))]);
        c.set_enabled(0, false);
        let _ = run_counting(&mut c, 1024);
        c.set_enabled(0, true);
        assert_eq!(run_counting(&mut c, 128), (0, 0));
        assert!(matches!(c.slot_state(0), Some(SlotState::FadingIn { .. })));
    }

    /// A module whose output was replaced by dry (non-finite) removed nothing in that block,
    /// and its lookahead was emptied: its events from that block are dropped (review finding).
    #[test]
    fn faulting_block_reports_no_events() {
        struct BadReporter(Reporter, u32);
        impl Module for BadReporter {
            fn id(&self) -> ModuleId {
                ModuleId::DeSmack
            }
            fn prepare(&mut self, sr: f32, b: usize, ch: usize) {
                self.0.prepare(sr, b, ch);
            }
            fn reset(&mut self) {
                self.0.reset();
            }
            fn process(&mut self, block: &mut AudioBlock<'_>, t: &mut TelemetryWriter) {
                self.0.process(block, t);
                self.1 += 1;
                if self.1 == 2 {
                    block.channel_mut(0)[0] = f32::NAN;
                }
            }
        }
        let mut c = chain_with(vec![Box::new(BadReporter(Reporter::new(0, 0), 0))]);
        assert_eq!(run_counting(&mut c, 128), (1, 1), "only the clean block reports");
        assert_eq!(c.faults(), 1);
    }

    #[test]
    fn out_of_range_indices_are_harmless() {
        let mut c = chain_with(vec![Box::new(Gain::new(-6.0))]);
        assert!(c.module(5).is_none());
        assert!(c.slot_state(5).is_none());
        assert!(!c.is_enabled(5));
        c.set_enabled(5, false);
        c.set_param(5, 0, 1.0);
        c.set_param(0, 99, 1.0);
    }

    #[test]
    #[should_panic]
    fn too_many_channels_rejected_at_prepare() {
        let mut c = Chain::new(vec![]);
        c.prepare(48_000.0, 64, MAX_CHANNELS + 1);
    }
}
