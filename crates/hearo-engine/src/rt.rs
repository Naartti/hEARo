//! The IOProc and its pre-allocated state. Everything reachable from [`io_proc`] is
//! real-time safe: no allocation, locks, I/O or logging; panics are caught and turn into
//! passthrough (and stay passthrough: a poisoned state is never processed again).

#![allow(unsafe_code)]

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;
use std::sync::Arc;
use std::time::Instant;

use hearo_dsp::{AudioBlock, Chain};
use objc2_core_audio::AudioObjectID;
use objc2_core_audio_types::{AudioBuffer, AudioBufferList, AudioTimeStamp};

use crate::waveform::{ColumnBuilder, WaveformRing};
use crate::{CaptureRing, Controls, EventRing, MAX_PARAMS, MAX_SLOTS, Stats};

/// State owned by the audio thread while the IOProc runs.
pub(crate) struct RtState {
    pub chain: Chain,
    planar: Vec<f32>,
    max_frames: usize,
    sample_rate: f64,
    controls: Arc<Controls>,
    stats: Arc<Stats>,
    capture: Option<Arc<CaptureRing>>,
    waveform: Option<(Arc<WaveformRing>, ColumnBuilder)>,
    /// Where the chain's detection events go (auto-saving de-smack catches, ADR 0018).
    events: Option<Arc<EventRing>>,
    applied_bypass: bool,
    applied_enabled: [bool; MAX_SLOTS],
    applied_gen: u32,
    /// Set after a caught panic: from then on the IOProc only does passthrough.
    poisoned: bool,
}

impl RtState {
    #[cfg(test)]
    pub fn new(chain: Chain, max_frames: usize, sample_rate: f64, controls: Arc<Controls>, stats: Arc<Stats>) -> Self {
        Self::with_capture(chain, max_frames, sample_rate, controls, stats, None)
    }

    pub fn with_capture(
        chain: Chain,
        max_frames: usize,
        sample_rate: f64,
        controls: Arc<Controls>,
        stats: Arc<Stats>,
        capture: Option<Arc<CaptureRing>>,
    ) -> Self {
        let applied_enabled = std::array::from_fn(|i| i < chain.len() && chain.is_enabled(i));
        Self {
            applied_bypass: chain.is_bypassed(),
            chain,
            planar: vec![0.0; 2 * max_frames],
            max_frames,
            sample_rate,
            controls,
            stats,
            capture,
            waveform: None,
            events: None,
            applied_enabled,
            applied_gen: u32::MAX,
            poisoned: false,
        }
    }

    /// Publish a live waveform into `ring` (call before handing the state to Core Audio).
    pub fn with_waveform(mut self, ring: Arc<WaveformRing>) -> Self {
        let slots: Vec<_> =
            (0..self.chain.len()).filter_map(|i| self.chain.module(i).map(|m| (m.id(), m.latency_samples()))).collect();
        let builder = ColumnBuilder::new(self.sample_rate, &slots);
        self.waveform = Some((ring, builder));
        self
    }

    /// Publish the chain's events into `ring` (call before handing the state to Core Audio).
    pub fn with_events(mut self, ring: Arc<EventRing>) -> Self {
        self.events = Some(ring);
        self
    }

    fn apply_controls(&mut self) {
        let bypass = self.controls.bypass();
        if bypass != self.applied_bypass {
            self.chain.set_bypass(bypass);
            self.applied_bypass = bypass;
        }
        for i in 0..self.chain.len().min(MAX_SLOTS) {
            let on = self.controls.enabled(i);
            if on != self.applied_enabled[i] {
                self.chain.set_enabled(i, on);
                self.applied_enabled[i] = on;
            }
        }
        let generation = self.controls.param_generation();
        if generation != self.applied_gen {
            self.applied_gen = generation;
            for i in 0..self.chain.len().min(MAX_SLOTS) {
                for p in 0..MAX_PARAMS {
                    let v = self.controls.param(i, p);
                    let current = self.chain.module(i).map_or(f32::NAN, |m| m.param(p));
                    if !v.is_nan() && v != current {
                        self.chain.set_param(i, p, v);
                    }
                }
            }
        }
    }

    /// One IOProc cycle.
    pub(crate) fn process(&mut self, input: Option<&AudioBufferList>, output: &mut AudioBufferList) {
        let start = Instant::now();
        self.apply_controls();
        let outs = buffers_mut(output);
        zero_all(outs);
        let out_frames = outs.iter().find(|b| !b.mData.is_null()).map_or(0, frames_of);
        if out_frames == 0 {
            return;
        }
        let tap = TapView::find(input);
        self.stats.set_tap_channels(tap.as_ref().map_or(0, |t| t.channels));
        let tap_frames = tap.as_ref().map_or(0, |t| t.frames);
        let short = tap_frames < out_frames;

        // Buffers larger than we prepared for are processed in max_frames chunks.
        let (mut in_peak, mut out_peak) = (0.0f32, 0.0f32);
        let mut offset = 0;
        while offset < out_frames {
            let n = self.max_frames.min(out_frames - offset);
            let (l, r) = self.planar.split_at_mut(self.max_frames);
            let (l, r) = (&mut l[..n], &mut r[..n]);
            match &tap {
                Some(t) => t.read(offset, l, r),
                None => {
                    l.fill(0.0);
                    r.fill(0.0);
                }
            }
            in_peak = l.iter().chain(r.iter()).fold(in_peak, |m, x| m.max(x.abs()));
            if let Some(ring) = &self.capture {
                ring.push(l, r); // unprocessed input, for labeling snapshots
            }
            let chunk_in_peak = l.iter().chain(r.iter()).fold(0.0f32, |m, x| m.max(x.abs()));
            let mut block = AudioBlock::from_channels([&mut l[..], &mut r[..]]);
            self.chain.process(&mut block);
            let chunk_out_peak = l.iter().chain(r.iter()).fold(0.0f32, |m, x| m.max(x.abs()));
            out_peak = out_peak.max(chunk_out_peak);
            if let Some((ring, builder)) = &mut self.waveform {
                // Per cleanup module (not the safety limiter): what it cut in this chunk. Under
                // global bypass nothing reaches the output, so nothing was removed.
                let t = self.chain.telemetry();
                let bypassed = self.chain.is_bypassed();
                let gr = core::array::from_fn(|i| {
                    if bypassed { 0.0 } else { t.meters(crate::waveform::GR_MODULES[i]).gain_reduction_db.max(0.0) }
                });
                builder.feed(chunk_in_peak, chunk_out_peak, &gr, n, ring);
            }
            write_out(outs, offset, l, r);
            offset += n;
        }
        // Events (with chain-input sample positions = capture-ring frames) to the control thread.
        // Under global bypass nothing was removed from what plays, so there is nothing to report.
        if let Some(ring) = &self.events
            && !self.chain.is_bypassed()
        {
            ring.push_all(self.chain.telemetry().events());
        }
        self.chain.telemetry_mut().clear_events();
        self.stats.set_faults(self.chain.faults());
        for id in hearo_dsp::ModuleId::ALL {
            let m = self.chain.telemetry().meters(id);
            self.stats.set_module(id, m.triggers, m.gain_reduction_db);
        }

        let cycle = start.elapsed().as_nanos().min(u32::MAX as u128) as u32;
        let budget = (out_frames as f64 / self.sample_rate * 1e9) as u32;
        self.stats.record(out_frames as u32, in_peak, out_peak, cycle, budget, short);
    }
}

/// The tap's samples inside the aggregate's input buffer list: either one interleaved buffer
/// (usual) or one mono buffer per channel. The tap streams are the *last* input buffers.
struct TapView<'a> {
    /// (data, stride, channel offset) for L and R.
    left: (&'a [f32], usize, usize),
    right: (&'a [f32], usize, usize),
    frames: usize,
    channels: u32,
}

impl<'a> TapView<'a> {
    fn find(input: Option<&'a AudioBufferList>) -> Option<Self> {
        let bufs = buffers(input?);
        let last = bufs.iter().rposition(|b| !b.mData.is_null() && b.mNumberChannels > 0)?;
        let b = &bufs[last];
        let data = slice_of(b);
        let ch = b.mNumberChannels as usize;
        if ch >= 2 {
            return Some(Self {
                left: (data, ch, 0),
                right: (data, ch, 1),
                frames: data.len() / ch,
                channels: ch as u32,
            });
        }
        // Mono buffer: is the previous one its non-interleaved partner?
        if last > 0 {
            let p = &bufs[last - 1];
            if !p.mData.is_null() && p.mNumberChannels == 1 && p.mDataByteSize == b.mDataByteSize {
                let pd = slice_of(p);
                return Some(Self { left: (pd, 1, 0), right: (data, 1, 0), frames: data.len(), channels: 2 });
            }
        }
        Some(Self { left: (data, 1, 0), right: (data, 1, 0), frames: data.len(), channels: 1 })
    }

    /// Copy frames `offset..offset + l.len()` into L/R (zero-padded past the tap's end).
    fn read(&self, offset: usize, l: &mut [f32], r: &mut [f32]) {
        let n = self.frames.saturating_sub(offset).min(l.len());
        let (ld, ls, lc) = self.left;
        let (rd, rs, rc) = self.right;
        for i in 0..n {
            l[i] = ld[(offset + i) * ls + lc];
            r[i] = rd[(offset + i) * rs + rc];
        }
        l[n..].fill(0.0);
        r[n..].fill(0.0);
    }
}

fn slice_of(b: &AudioBuffer) -> &[f32] {
    if b.mData.is_null() {
        return &[];
    }
    // SAFETY: Core Audio guarantees mData holds mDataByteSize bytes of f32 samples.
    unsafe { std::slice::from_raw_parts(b.mData as *const f32, b.mDataByteSize as usize / 4) }
}

fn buffers(list: &AudioBufferList) -> &[AudioBuffer] {
    // SAFETY: AudioBufferList is a C struct with a trailing array of mNumberBuffers buffers.
    unsafe { std::slice::from_raw_parts(list.mBuffers.as_ptr(), list.mNumberBuffers as usize) }
}

fn buffers_mut(list: &mut AudioBufferList) -> &mut [AudioBuffer] {
    // SAFETY: as above.
    unsafe { std::slice::from_raw_parts_mut(list.mBuffers.as_mut_ptr(), list.mNumberBuffers as usize) }
}

fn frames_of(b: &AudioBuffer) -> usize {
    if b.mNumberChannels == 0 { 0 } else { b.mDataByteSize as usize / 4 / b.mNumberChannels as usize }
}

fn zero_all(outs: &mut [AudioBuffer]) {
    for b in outs {
        if !b.mData.is_null() {
            // SAFETY: mData holds mDataByteSize bytes.
            unsafe { std::ptr::write_bytes(b.mData as *mut u8, 0, b.mDataByteSize as usize) };
        }
    }
}

/// Write L/R into output frames `offset..offset + l.len()`. Device channel 0 gets L, channel 1
/// gets R, counting channels across buffers (interleaved or one buffer per channel); other
/// channels stay silent.
fn write_out(outs: &mut [AudioBuffer], offset: usize, l: &[f32], r: &[f32]) {
    let mut first_channel = 0usize;
    for b in outs.iter_mut() {
        let ch = b.mNumberChannels as usize;
        if !b.mData.is_null() && ch > 0 {
            let frames = frames_of(b);
            // SAFETY: mData holds mDataByteSize bytes of f32.
            let data = unsafe { std::slice::from_raw_parts_mut(b.mData as *mut f32, b.mDataByteSize as usize / 4) };
            let n = frames.saturating_sub(offset).min(l.len());
            for c in 0..ch {
                let src = match first_channel + c {
                    0 => l,
                    1 => r,
                    _ => continue,
                };
                for i in 0..n {
                    data[(offset + i) * ch + c] = src[i];
                }
            }
        }
        first_channel += ch;
    }
}

/// Plain passthrough (used after a panic). Real-time safe; uses the same channel mapping.
fn passthrough(input: Option<&AudioBufferList>, output: &mut AudioBufferList, l: &mut [f32], r: &mut [f32]) {
    let outs = buffers_mut(output);
    zero_all(outs);
    let Some(tap) = TapView::find(input) else { return };
    let total = outs.iter().find(|b| !b.mData.is_null()).map_or(0, frames_of);
    let mut offset = 0;
    while offset < total {
        let n = l.len().min(total - offset);
        tap.read(offset, &mut l[..n], &mut r[..n]);
        write_out(outs, offset, &l[..n], &r[..n]);
        offset += n;
    }
}

/// The C IOProc. `client` is a `*mut RtState` owned by the engine.
pub(crate) unsafe extern "C-unwind" fn io_proc(
    _device: AudioObjectID,
    _now: NonNull<AudioTimeStamp>,
    input: NonNull<AudioBufferList>,
    _input_time: NonNull<AudioTimeStamp>,
    output: NonNull<AudioBufferList>,
    _output_time: NonNull<AudioTimeStamp>,
    client: *mut c_void,
) -> i32 {
    // SAFETY: the engine keeps RtState alive until after AudioDeviceStop + DestroyIOProcID,
    // and only this thread touches it while running.
    let state = unsafe { &mut *(client as *mut RtState) };
    // SAFETY: Core Audio passes valid buffer lists for the duration of the call.
    let input = unsafe { input.as_ref() };
    let output = unsafe { &mut *output.as_ptr() };
    if !state.poisoned {
        // catch_unwind is free on the non-panicking path (landing pads only).
        let result = catch_unwind(AssertUnwindSafe(|| state.process(Some(input), output)));
        if result.is_ok() {
            return 0;
        }
        // After a panic RtState may be half-updated: never run the chain again.
        state.poisoned = true;
        state.stats.record_panic();
    }
    let (l, r) = state.planar.split_at_mut(state.max_frames);
    passthrough(Some(input), output, l, r);
    0
}

#[cfg(test)]
mod tests {
    use std::ffi::c_void;

    use hearo_dsp::modules::utility::Gain;
    use hearo_dsp::{Module, ModuleId, TelemetryWriter};
    use hearo_testkit::rt::{CountingAllocator, count_allocs};
    use objc2_core_audio_types::{AudioBuffer, AudioBufferList};

    use super::*;

    #[global_allocator]
    static ALLOC: CountingAllocator = CountingAllocator;

    /// A heap-backed AudioBufferList with `n` buffers.
    struct FakeList {
        words: Vec<u64>,
    }

    impl FakeList {
        fn new(bufs: &mut [(u32, Option<&mut Vec<f32>>)]) -> Self {
            // u32 count + pad, then 16 bytes per AudioBuffer.
            let mut words = vec![0u64; 1 + 2 * bufs.len().max(1)];
            // SAFETY: words is large enough and 8-aligned for AudioBufferList with n buffers.
            let list = unsafe { &mut *(words.as_mut_ptr() as *mut AudioBufferList) };
            list.mNumberBuffers = bufs.len() as u32;
            let out = buffers_mut(list);
            for (b, (ch, data)) in out.iter_mut().zip(bufs.iter_mut()) {
                *b = AudioBuffer {
                    mNumberChannels: *ch,
                    mDataByteSize: data.as_ref().map_or(0, |d| (d.len() * 4) as u32),
                    mData: data.as_mut().map_or(std::ptr::null_mut(), |d| d.as_mut_ptr() as *mut c_void),
                };
            }
            Self { words }
        }
        fn list(&mut self) -> &mut AudioBufferList {
            // SAFETY: built in `new`.
            unsafe { &mut *(self.words.as_mut_ptr() as *mut AudioBufferList) }
        }
    }

    fn state(modules: Vec<Box<dyn Module>>, max_frames: usize) -> RtState {
        let mut chain = Chain::new(modules);
        chain.prepare(48_000.0, max_frames, 2);
        RtState::new(chain, max_frames, 48_000.0, Arc::new(Controls::default()), Arc::new(Stats::default()))
    }

    #[test]
    fn interleaved_passthrough_with_odd_sizes_and_big_buffers() {
        // 1000 frames > max_frames 256: chunked, never truncated.
        let mut st = state(vec![], 256);
        let mut tap: Vec<f32> = (0..2000).map(|i| i as f32).collect();
        let mut out = vec![0.0f32; 2000];
        let mut inp = FakeList::new(&mut [(2, Some(&mut tap))]);
        let mut outp = FakeList::new(&mut [(2, Some(&mut out))]);
        let (_, allocs) = count_allocs(|| st.process(Some(inp.list()), outp.list()));
        assert_eq!(allocs, 0);
        assert_eq!(out, (0..2000).map(|i| i as f32).collect::<Vec<_>>());
        assert_eq!(st.stats.snapshot().short_input, 0);
    }

    #[test]
    fn non_interleaved_tap_and_output_keep_left_and_right() {
        let mut st = state(vec![], 64);
        let mut tl = vec![1.0f32; 64];
        let mut tr = vec![-1.0f32; 64];
        let mut ol = vec![0.0f32; 64];
        let mut or = vec![0.0f32; 64];
        let mut o3 = vec![9.0f32; 64];
        // An AirPods-like mic stream (switched off: null data) before the tap streams.
        let mut inp = FakeList::new(&mut [(1, None), (1, Some(&mut tl)), (1, Some(&mut tr))]);
        let mut outp = FakeList::new(&mut [(1, None), (1, Some(&mut ol)), (1, Some(&mut or)), (1, Some(&mut o3))]);
        let (_, allocs) = count_allocs(|| st.process(Some(inp.list()), outp.list()));
        assert_eq!(allocs, 0);
        // Device channels are counted across buffers, including a null one: buffer 0 (null) is
        // channel 0 (L, dropped by Core Audio), so buffer 1 is channel 1 and gets R.
        assert!(ol.iter().all(|&x| x == -1.0), "channel 1 gets R");
        assert!(or.iter().all(|&x| x == 0.0) && o3.iter().all(|&x| x == 0.0), "extra channels silent");
    }

    #[test]
    fn short_input_is_zero_padded_and_counted() {
        let mut st = state(vec![], 256);
        let mut tap = vec![0.5f32; 2 * 100];
        let mut out = vec![7.0f32; 2 * 128];
        let mut inp = FakeList::new(&mut [(2, Some(&mut tap))]);
        let mut outp = FakeList::new(&mut [(2, Some(&mut out))]);
        st.process(Some(inp.list()), outp.list());
        assert!(out[..200].iter().all(|&x| x == 0.5));
        assert!(out[200..].iter().all(|&x| x == 0.0));
        assert_eq!(st.stats.snapshot().short_input, 1);
    }

    #[test]
    fn controls_reach_the_chain_without_allocating() {
        let mut st = state(vec![Box::new(Gain::new(0.0))], 512);
        st.controls.set_param(0, 0, -6.0206);
        st.controls.set_enabled(0, true);
        let mut tap = vec![1.0f32; 2 * 512];
        let mut out = vec![0.0f32; 2 * 512];
        let mut inp = FakeList::new(&mut [(2, Some(&mut tap))]);
        let mut outp = FakeList::new(&mut [(2, Some(&mut out))]);
        let (_, allocs) = count_allocs(|| st.process(Some(inp.list()), outp.list()));
        assert_eq!(allocs, 0);
        assert!((out[1023] - 0.5).abs() < 1e-3, "gain applied after its 10 ms ramp: {}", out[1023]);
        st.controls.set_bypass(true);
        for _ in 0..3 {
            st.process(Some(inp.list()), outp.list());
        }
        assert_eq!(out[1023], 1.0, "bypass reached the chain");
    }

    #[test]
    fn waveform_columns_flow_without_allocating() {
        let ring = Arc::new(crate::waveform::WaveformRing::default());
        let mut st = state(vec![Box::new(Gain::new(0.0))], 512).with_waveform(ring.clone());
        let mut tap = vec![0.25f32; 2 * 512];
        let mut out = vec![0.0f32; 2 * 512];
        let mut inp = FakeList::new(&mut [(2, Some(&mut tap))]);
        let mut outp = FakeList::new(&mut [(2, Some(&mut out))]);
        let (_, allocs) = count_allocs(|| {
            for _ in 0..94 {
                st.process(Some(inp.list()), outp.list()); // ≈ 1 s at 48 kHz
            }
        });
        assert_eq!(allocs, 0);
        let (next, cols) = ring.read_since(0, 1000);
        assert!((58..=61).contains(&next), "{next} columns for 1 s");
        assert!(cols.iter().skip(2).all(|c| (c.peak - 0.25).abs() < 1e-3 && !c.removed()));
    }

    /// Under global bypass (A/B) de-smack still detects, but nothing was removed from what plays:
    /// no events reach the ring (no auto-saved "catches", no orange flash; review finding).
    #[test]
    fn bypassed_chain_reports_no_events() {
        let sr = 48_000.0;
        let (sig, _) = hearo_testkit::signals::speech_with_pause_smacks(sr, 8.0, 41, 5, (-18.0, -12.0), -55.0);
        let events = Arc::new(EventRing::new(64));
        let mut st =
            state(vec![Box::new(hearo_dsp::modules::desmack::DeSmack::new())], 512).with_events(events.clone());
        st.controls.set_bypass(true);
        let mut tap = vec![0.0f32; 2 * 512];
        let mut out = vec![0.0f32; 2 * 512];
        let tap_ptr = tap.as_mut_ptr();
        let mut inp = FakeList::new(&mut [(2, Some(&mut tap))]);
        let mut outp = FakeList::new(&mut [(2, Some(&mut out))]);
        let (_, allocs) = count_allocs(|| {
            for chunk in sig.as_chunks::<512>().0 {
                for (i, x) in chunk.iter().enumerate() {
                    // SAFETY: the fake list points at `tap`, which outlives the loop.
                    unsafe {
                        *tap_ptr.add(2 * i) = *x;
                        *tap_ptr.add(2 * i + 1) = *x;
                    }
                }
                st.process(Some(inp.list()), outp.list());
            }
        });
        assert_eq!(allocs, 0);
        let mut got = Vec::new();
        events.drain(&mut got);
        assert!(got.is_empty(), "{} events while bypassed", got.len());
    }

    /// De-smack in the live path: its gain reduction lands in the waveform columns (with the
    /// processed peak below the original there), a parameter handed over mid-run arrives, and
    /// none of it allocates.
    #[test]
    fn per_module_reduction_reaches_the_columns_without_allocating() {
        use crate::waveform::GR_MODULES;
        let sr = 48_000.0;
        let (sig, smacks) = hearo_testkit::signals::speech_with_pause_smacks(sr, 8.0, 41, 5, (-18.0, -12.0), -55.0);
        let ring = Arc::new(crate::waveform::WaveformRing::default());
        let events = Arc::new(EventRing::new(64));
        let mut st = state(vec![Box::new(hearo_dsp::modules::desmack::DeSmack::new())], 512)
            .with_waveform(ring.clone())
            .with_events(events.clone());
        let mut tap = vec![0.0f32; 2 * 512];
        let mut out = vec![0.0f32; 2 * 512];
        let mut inp = FakeList::new(&mut [(2, Some(&mut tap))]);
        let mut outp = FakeList::new(&mut [(2, Some(&mut out))]);
        let tap_ptr = tap.as_mut_ptr();
        let (_, allocs) = count_allocs(|| {
            for (k, chunk) in sig.as_chunks::<512>().0.iter().enumerate() {
                for (i, x) in chunk.iter().enumerate() {
                    // SAFETY: the fake list points at `tap`, which outlives the loop.
                    unsafe {
                        *tap_ptr.add(2 * i) = *x;
                        *tap_ptr.add(2 * i + 1) = *x;
                    }
                }
                if k == 10 {
                    st.controls.set_param(0, 2, 30.0); // de-smack range, via the lock-free handoff
                }
                st.process(Some(inp.list()), outp.list());
            }
        });
        assert_eq!(allocs, 0);
        assert_eq!(st.chain.module(0).map(|m| m.param(2)), Some(30.0), "param reached the chain");
        let (_, cols) = ring.read_since(0, 1000);
        let row = GR_MODULES.iter().position(|m| *m == ModuleId::DeSmack).unwrap();
        let ducked: Vec<_> = cols.iter().filter(|c| c.gain_reduction_db[row] >= 3.0).collect();
        assert!(ducked.len() >= smacks.len() / 2, "{} ducked columns for {} smacks", ducked.len(), smacks.len());
        assert!(ducked.iter().all(|c| c.removed()));
        assert!(ducked.iter().any(|c| c.out_peak < 0.7 * c.peak), "output envelope shows the duck");
        let others = cols.iter().all(|c| c.gain_reduction_db.iter().enumerate().all(|(i, g)| i == row || *g == 0.0));
        assert!(others, "modules not in the chain report nothing");
        // The catches left the audio thread through the event ring, at the smacks' input
        // positions (the capture ring's frame clock), so a clip can be cut around them.
        let mut got = Vec::new();
        events.drain(&mut got);
        assert!(!got.is_empty() && got.iter().all(|e| e.module == ModuleId::DeSmack));
        let near = |e: &hearo_dsp::Event| {
            smacks
                .iter()
                .any(|s| (e.start_sample as usize) + 480 >= s.start && e.start_sample as usize <= s.end() + 240)
        };
        assert!(
            got.iter().all(near),
            "event positions off the smacks: {:?}",
            got.iter().map(|e| e.start_sample).collect::<Vec<_>>()
        );
    }

    struct Panicky;
    impl Module for Panicky {
        fn id(&self) -> ModuleId {
            ModuleId::Utility
        }
        fn prepare(&mut self, _: f32, _: usize, _: usize) {}
        fn reset(&mut self) {}
        fn process(&mut self, _: &mut AudioBlock<'_>, _: &mut TelemetryWriter) {
            panic!("boom");
        }
    }

    #[test]
    fn panic_poisons_into_passthrough() {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let mut st = state(vec![Box::new(Panicky)], 64);
        let mut tap = vec![0.25f32; 128];
        let mut out = vec![0.0f32; 128];
        let mut inp = FakeList::new(&mut [(2, Some(&mut tap))]);
        let mut outp = FakeList::new(&mut [(2, Some(&mut out))]);
        let state_ptr = &mut st as *mut RtState as *mut c_void;
        let call = |inp: &mut FakeList, outp: &mut FakeList| {
            // SAFETY: valid fake lists and state for the duration of the call.
            unsafe {
                io_proc(
                    0,
                    NonNull::dangling(),
                    NonNull::from(inp.list()),
                    NonNull::dangling(),
                    NonNull::from(outp.list()),
                    NonNull::dangling(),
                    state_ptr,
                )
            }
        };
        call(&mut inp, &mut outp);
        assert!(out.iter().all(|&x| x == 0.25), "passthrough after panic");
        let (_, allocs) = count_allocs(|| call(&mut inp, &mut outp));
        assert_eq!(allocs, 0, "poisoned state goes straight to passthrough");
        std::panic::set_hook(prev);
        assert!(st.poisoned);
        assert_eq!(st.stats.snapshot().panics, 1, "panicked once, not every cycle");
    }
}
