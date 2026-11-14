//! System Audio Recording permission (TCC service `kTCCServiceAudioCapture`).
//!
//! There's no public API to *check* this permission; like AudioCap we call the private
//! `TCCAccessPreflight` from TCC.framework (loaded with `dlopen`, so nothing breaks if it ever
//! disappears — we then report `Unknown`). It never prompts; macOS prompts by itself the first
//! time a tap is read.

#![allow(unsafe_code)]

use std::ffi::{CStr, c_char, c_int, c_void};

use objc2_core_foundation::CFString;

/// Whether this process (or rather its *responsible app*: Terminal, VS Code, hEARo) may tap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// Granted.
    Authorized,
    /// Explicitly denied in System Settings.
    Denied,
    /// Not asked yet: macOS will prompt when the tap starts delivering audio.
    NotDetermined,
    /// Couldn't check (TCC private API unavailable).
    Unknown,
}

unsafe extern "C" {
    fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}

const TCC_PATH: &CStr = c"/System/Library/PrivateFrameworks/TCC.framework/Versions/A/TCC";
const RTLD_LAZY: c_int = 1;

/// Check the System Audio Recording permission without prompting.
pub fn audio_capture() -> Permission {
    // SAFETY: dlopen/dlsym with valid C strings; the symbol's signature is
    // `int TCCAccessPreflight(CFStringRef service, CFDictionaryRef options)`.
    unsafe {
        let h = dlopen(TCC_PATH.as_ptr(), RTLD_LAZY);
        if h.is_null() {
            return Permission::Unknown;
        }
        let sym = dlsym(h, c"TCCAccessPreflight".as_ptr());
        if sym.is_null() {
            return Permission::Unknown;
        }
        let preflight: extern "C" fn(*const CFString, *const c_void) -> c_int = std::mem::transmute(sym);
        let service = CFString::from_str("kTCCServiceAudioCapture");
        match preflight(&*service, std::ptr::null()) {
            0 => Permission::Authorized,
            1 => Permission::Denied,
            _ => Permission::NotDetermined,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn preflight_returns_without_prompting() {
        // Any answer is fine; it must not crash or hang.
        let _ = super::audio_capture();
    }
}
