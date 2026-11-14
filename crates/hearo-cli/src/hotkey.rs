//! A global hotkey for the CLI via Carbon's `RegisterEventHotKey` — the same mechanism the
//! app's KeyboardShortcuts library uses. Needs no Accessibility permission. While registered,
//! the key is captured system-wide (it won't type in other apps).
//!
//! Hotkey events are delivered through the main thread's run loop, so the caller must pump it
//! (see [`pump`]).

#![allow(unsafe_code)]

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};

/// Virtual key code of the "§" key (ISO keyboards, left of "1"): `kVK_ISO_Section`.
pub const KEY_SECTION: u32 = 0x0A;

static PRESSES: AtomicU64 = AtomicU64::new(0);

#[repr(C)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

#[repr(C)]
struct EventHotKeyId {
    signature: u32,
    id: u32,
}

type Handler = extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetApplicationEventTarget() -> *mut c_void;
    fn InstallEventHandler(
        target: *mut c_void,
        handler: Handler,
        num_types: u32,
        list: *const EventTypeSpec,
        user_data: *mut c_void,
        out_ref: *mut *mut c_void,
    ) -> i32;
    fn RegisterEventHotKey(
        key_code: u32,
        modifiers: u32,
        id: EventHotKeyId,
        target: *mut c_void,
        options: u32,
        out_ref: *mut *mut c_void,
    ) -> i32;
    fn UnregisterEventHotKey(hotkey: *mut c_void) -> i32;
    fn GetEventDispatcherTarget() -> *mut c_void;
    fn ReceiveNextEvent(
        num_types: u32,
        list: *const EventTypeSpec,
        timeout_secs: f64,
        pull_event: u8,
        out_event: *mut *mut c_void,
    ) -> i32;
    fn SendEventToEventTarget(event: *mut c_void, target: *mut c_void) -> i32;
    fn ReleaseEvent(event: *mut c_void);
    #[cfg(test)]
    fn CreateEvent(
        allocator: *const c_void,
        class: u32,
        kind: u32,
        when: f64,
        flags: u32,
        out_event: *mut *mut c_void,
    ) -> i32;
}

const K_EVENT_CLASS_KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
const K_EVENT_HOT_KEY_PRESSED: u32 = 5;

extern "C" fn on_hotkey(_: *mut c_void, _: *mut c_void, _: *mut c_void) -> i32 {
    PRESSES.fetch_add(1, Ordering::Relaxed);
    0
}

/// A registered global hotkey; unregistered on drop.
#[derive(Debug)]
pub struct HotKey(*mut c_void);

impl HotKey {
    /// Register `key_code` with no modifiers. Must be called on the main thread.
    pub fn register(key_code: u32) -> Result<Self, i32> {
        let spec = EventTypeSpec { event_class: K_EVENT_CLASS_KEYBOARD, event_kind: K_EVENT_HOT_KEY_PRESSED };
        let mut handler_ref = std::ptr::null_mut();
        let mut hotkey = std::ptr::null_mut();
        // SAFETY: plain Carbon calls with valid pointers; the handler is a static fn.
        unsafe {
            let target = GetApplicationEventTarget();
            let st = InstallEventHandler(target, on_hotkey, 1, &spec, std::ptr::null_mut(), &mut handler_ref);
            if st != 0 {
                return Err(st);
            }
            let id = EventHotKeyId { signature: u32::from_be_bytes(*b"hEAR"), id: 1 };
            let st = RegisterEventHotKey(key_code, 0, id, target, 0, &mut hotkey);
            if st != 0 {
                return Err(st);
            }
        }
        Ok(Self(hotkey))
    }
}

impl Drop for HotKey {
    fn drop(&mut self) {
        // SAFETY: registered in `register`.
        unsafe { UnregisterEventHotKey(self.0) };
    }
}

/// Total presses so far.
pub fn presses() -> u64 {
    PRESSES.load(Ordering::Relaxed)
}

/// Pump Carbon events on the main thread for up to `seconds`. A command-line process has no
/// `RunApplicationEventLoop`, so registered hotkeys are grabbed by the system but never
/// dispatched unless we pull events and send them to the event dispatcher ourselves.
pub fn pump(seconds: f64) {
    // SAFETY: Carbon event calls on the main thread; every received event is released.
    unsafe {
        let dispatcher = GetEventDispatcherTarget();
        let mut timeout = seconds;
        loop {
            let mut ev = std::ptr::null_mut();
            if ReceiveNextEvent(0, std::ptr::null(), timeout, 1, &mut ev) != 0 || ev.is_null() {
                break; // timed out (eventLoopTimedOutErr) or nothing to do
            }
            SendEventToEventTarget(ev, dispatcher);
            ReleaseEvent(ev);
            timeout = 0.0; // drain whatever else is queued, then return
        }
    }
}

/// Post a synthetic hot-key-pressed event to ourselves (tests the dispatch path without a
/// keyboard; the event carries no hotkey id, our handler doesn't need one).
#[cfg(test)]
fn post_synthetic_press() {
    // SAFETY: creates and dispatches a Carbon event to our own application target.
    unsafe {
        let mut ev = std::ptr::null_mut();
        assert_eq!(CreateEvent(std::ptr::null(), K_EVENT_CLASS_KEYBOARD, K_EVENT_HOT_KEY_PRESSED, 0.0, 0, &mut ev), 0);
        SendEventToEventTarget(ev, GetApplicationEventTarget());
        ReleaseEvent(ev);
    }
}

/// Show a macOS notification (non-blocking, silent so it doesn't end up in the audio).
pub fn notify(title: &str, message: &str) {
    let script = format!("display notification {:?} with title {:?}", message, title);
    let _ = std::process::Command::new("osascript")
        .args(["-e", &script])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Registering, pumping and dispatching work in a plain command-line process. (Real key
    /// presses can't be simulated without Accessibility permission; the human verifies those.)
    #[test]
    fn handler_receives_dispatched_hotkey_events() {
        let hk = HotKey::register(0x7A /* F1 — unlikely to clash in a test */);
        assert!(hk.is_ok(), "register failed: {:?}", hk.err());
        let before = presses();
        post_synthetic_press();
        pump(0.05);
        assert_eq!(presses(), before + 1);
    }
}
