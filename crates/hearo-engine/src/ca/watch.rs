//! Device-change notifications: wakes a control thread when the default output device or the
//! device list changes (AirPods connected, Sonos selected in Control Center, a USB DAC
//! unplugged). Core Audio calls the listener on its own notification thread; it only bumps a
//! counter and wakes waiters. The rebuild itself happens on the waiting control thread
//! (ADR 0010: tap *and* aggregate must be rebuilt on an output change).

#![allow(unsafe_code)]

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use objc2_core_audio::{
    AudioObjectAddPropertyListener, AudioObjectPropertyAddress, AudioObjectRemovePropertyListener,
    kAudioHardwarePropertyDefaultOutputDevice, kAudioHardwarePropertyDevices, kAudioObjectPropertyElementMain,
};

use super::{GLOBAL, ObjectId, SYSTEM, check};

/// Counter + condition variable shared with the Core Audio listener.
#[derive(Debug, Default)]
struct Signal {
    generation: Mutex<u64>,
    cv: Condvar,
}

impl Signal {
    fn bump(&self) {
        let mut g = self.generation.lock().unwrap_or_else(|e| e.into_inner());
        *g += 1;
        self.cv.notify_all();
    }
}

const SELECTORS: [u32; 2] = [kAudioHardwarePropertyDefaultOutputDevice, kAudioHardwarePropertyDevices];

fn address(selector: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress { mSelector: selector, mScope: GLOBAL, mElement: kAudioObjectPropertyElementMain }
}

unsafe extern "C-unwind" fn on_change(
    _object: ObjectId,
    _count: u32,
    _addresses: NonNull<AudioObjectPropertyAddress>,
    client: *mut c_void,
) -> i32 {
    // SAFETY: `client` is the Signal registered in `OutputWatcher::new`, kept alive forever
    // (one leaked strong count) so a notification in flight during removal stays valid.
    if let Some(signal) = unsafe { client.cast::<Signal>().as_ref() } {
        signal.bump();
    }
    0
}

/// Watches the default output device and the device list. Dropping it unregisters.
#[derive(Debug)]
pub struct OutputWatcher {
    signal: Arc<Signal>,
    registered: Vec<u32>,
}

impl OutputWatcher {
    /// Register the listeners. Fails only if Core Audio refuses all of them.
    pub fn new() -> super::Result<Self> {
        let signal = Arc::new(Signal::default());
        // Core Audio may still be delivering a notification while we unregister; the listener
        // then reads a Signal that must still exist. Leak one strong count (a few bytes per
        // watcher; the app makes one).
        std::mem::forget(signal.clone());
        let client = Arc::as_ptr(&signal).cast_mut().cast::<c_void>();
        let mut registered = Vec::new();
        let mut last_err = None;
        for sel in SELECTORS {
            let addr = address(sel);
            // SAFETY: valid address, a correct C listener, and a client pointer that outlives it.
            let status =
                unsafe { AudioObjectAddPropertyListener(SYSTEM, NonNull::from(&addr), Some(on_change), client) };
            match check("AudioObjectAddPropertyListener", status) {
                Ok(()) => registered.push(sel),
                Err(e) => last_err = Some(e),
            }
        }
        match (registered.is_empty(), last_err) {
            (true, Some(e)) => Err(e),
            _ => Ok(Self { signal, registered }),
        }
    }

    /// Current change counter (compare with a later value to see whether anything changed).
    pub fn generation(&self) -> u64 {
        *self.signal.generation.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Block until the counter moves past `seen` or `timeout` passes; returns the counter.
    pub fn wait(&self, seen: u64, timeout: Duration) -> u64 {
        let g = self.signal.generation.lock().unwrap_or_else(|e| e.into_inner());
        let (g, _) = self.signal.cv.wait_timeout_while(g, timeout, |g| *g == seen).unwrap_or_else(|e| e.into_inner());
        *g
    }

    /// Wake waiters as if a device changed (tests; shutting a watcher thread down).
    pub fn poke(&self) {
        self.signal.bump();
    }
}

impl Drop for OutputWatcher {
    fn drop(&mut self) {
        let client = Arc::as_ptr(&self.signal).cast_mut().cast::<c_void>();
        for &sel in &self.registered {
            let addr = address(sel);
            // SAFETY: removing exactly the listener we added.
            unsafe { AudioObjectRemovePropertyListener(SYSTEM, NonNull::from(&addr), Some(on_change), client) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn wait_returns_on_poke_or_timeout() {
        let w = OutputWatcher::new().expect("listeners");
        let g = w.generation();
        let t = Instant::now();
        assert_eq!(w.wait(g, Duration::from_millis(30)), g);
        assert!(t.elapsed() >= Duration::from_millis(25));
        w.poke();
        assert_eq!(w.wait(g, Duration::from_secs(5)), g + 1);
    }
}
