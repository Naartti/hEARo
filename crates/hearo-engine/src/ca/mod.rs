//! Thin, safe wrappers over the Core Audio HAL (via `objc2-core-audio`, ADR 0010).
//! Control-thread only: nothing here is real-time safe.

#![allow(unsafe_code)]

pub mod aggregate;
pub mod device;
pub mod permission;
pub mod procs;
pub mod tap;
pub mod watch;

use std::ffi::c_void;
use std::fmt;
use std::mem::{MaybeUninit, size_of};
use std::ptr::NonNull;

use objc2_core_audio::{
    AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectID, AudioObjectPropertyAddress,
    AudioObjectSetPropertyData, kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal,
};
use objc2_core_foundation::{CFRetained, CFString};

/// A Core Audio object ID (device, process, tap, ...).
pub type ObjectId = AudioObjectID;
/// The system object.
pub const SYSTEM: ObjectId = objc2_core_audio::kAudioObjectSystemObject as ObjectId;
/// Global scope.
pub const GLOBAL: u32 = kAudioObjectPropertyScopeGlobal;
/// Input scope.
pub const INPUT: u32 = objc2_core_audio::kAudioObjectPropertyScopeInput;
/// Output scope.
pub const OUTPUT: u32 = objc2_core_audio::kAudioObjectPropertyScopeOutput;

/// A failed Core Audio call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaError {
    /// What we were doing.
    pub op: &'static str,
    /// The OSStatus.
    pub status: i32,
}

impl fmt::Display for CaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Many OSStatus values are four-char codes ('who?', '!dev', ...).
        let b = self.status.to_be_bytes();
        if b.iter().all(|c| c.is_ascii_graphic() || *c == b' ') {
            write!(f, "{} failed: '{}' ({})", self.op, String::from_utf8_lossy(&b), self.status)
        } else {
            write!(f, "{} failed: OSStatus {}", self.op, self.status)
        }
    }
}

impl std::error::Error for CaError {}

/// Result alias.
pub type Result<T> = std::result::Result<T, CaError>;

pub(crate) fn check(op: &'static str, status: i32) -> Result<()> {
    if status == 0 { Ok(()) } else { Err(CaError { op, status }) }
}

fn addr(selector: u32, scope: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress { mSelector: selector, mScope: scope, mElement: kAudioObjectPropertyElementMain }
}

/// Read a fixed-size (plain-old-data) property.
pub fn get<T: Copy>(obj: ObjectId, selector: u32, scope: u32) -> Result<T> {
    get_qualified::<T, ()>(obj, selector, scope, None)
}

/// Read a fixed-size property with a qualifier (e.g. a PID).
pub fn get_qualified<T: Copy, Q>(obj: ObjectId, selector: u32, scope: u32, qualifier: Option<&Q>) -> Result<T> {
    let a = addr(selector, scope);
    let mut out = MaybeUninit::<T>::uninit();
    let mut size = size_of::<T>() as u32;
    let (qsize, qptr) = match qualifier {
        Some(q) => (size_of::<Q>() as u32, q as *const Q as *const c_void),
        None => (0, std::ptr::null()),
    };
    // SAFETY: all pointers are valid for the sizes passed; Core Audio writes at most `size` bytes.
    let status = unsafe {
        AudioObjectGetPropertyData(
            obj,
            NonNull::from(&a),
            qsize,
            qptr,
            NonNull::from(&mut size),
            NonNull::new_unchecked(out.as_mut_ptr().cast()),
        )
    };
    check("AudioObjectGetPropertyData", status)?;
    if size as usize != size_of::<T>() {
        return Err(CaError { op: "property size mismatch", status: size as i32 });
    }
    // SAFETY: Core Audio filled exactly size_of::<T>() bytes of a POD type.
    Ok(unsafe { out.assume_init() })
}

/// Read an array property (e.g. a list of object IDs).
pub fn get_vec<T: Copy + Default>(obj: ObjectId, selector: u32, scope: u32) -> Result<Vec<T>> {
    let a = addr(selector, scope);
    let mut size = 0u32;
    // SAFETY: valid pointers; size query only.
    let status = unsafe {
        AudioObjectGetPropertyDataSize(obj, NonNull::from(&a), 0, std::ptr::null(), NonNull::from(&mut size))
    };
    check("AudioObjectGetPropertyDataSize", status)?;
    let n = size as usize / size_of::<T>();
    let mut v = vec![T::default(); n];
    if n == 0 {
        return Ok(v);
    }
    // SAFETY: `v` has room for `size` bytes.
    let status = unsafe {
        AudioObjectGetPropertyData(
            obj,
            NonNull::from(&a),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::new_unchecked(v.as_mut_ptr().cast()),
        )
    };
    check("AudioObjectGetPropertyData", status)?;
    v.truncate(size as usize / size_of::<T>());
    Ok(v)
}

/// Read a variable-size property as raw, 8-byte-aligned bytes.
pub fn get_raw(obj: ObjectId, selector: u32, scope: u32) -> Result<Vec<u64>> {
    let a = addr(selector, scope);
    let mut size = 0u32;
    // SAFETY: size query only.
    let status = unsafe {
        AudioObjectGetPropertyDataSize(obj, NonNull::from(&a), 0, std::ptr::null(), NonNull::from(&mut size))
    };
    check("AudioObjectGetPropertyDataSize", status)?;
    let mut v = vec![0u64; (size as usize).div_ceil(8).max(1)];
    // SAFETY: `v` has room for at least `size` bytes.
    let status = unsafe {
        AudioObjectGetPropertyData(
            obj,
            NonNull::from(&a),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::new_unchecked(v.as_mut_ptr().cast()),
        )
    };
    check("AudioObjectGetPropertyData", status)?;
    Ok(v)
}

/// Read a CFString property (UIDs, names, bundle IDs).
pub fn get_string(obj: ObjectId, selector: u32, scope: u32) -> Result<String> {
    let ptr: *const CFString = get(obj, selector, scope)?;
    let Some(nn) = NonNull::new(ptr as *mut CFString) else { return Ok(String::new()) };
    // SAFETY: Core Audio returns a +1 retained CFStringRef ("Copy" semantics); we take ownership.
    let s = unsafe { CFRetained::from_raw(nn) };
    Ok(s.to_string())
}

/// Write a fixed-size property.
pub fn set<T: Copy>(obj: ObjectId, selector: u32, scope: u32, value: &T) -> Result<()> {
    set_raw(obj, selector, scope, (value as *const T).cast(), size_of::<T>())
}

/// Write a property from raw bytes.
pub fn set_raw(obj: ObjectId, selector: u32, scope: u32, data: *const c_void, len: usize) -> Result<()> {
    let a = addr(selector, scope);
    // SAFETY: `data` points to `len` readable bytes (caller contract).
    let status = unsafe {
        AudioObjectSetPropertyData(
            obj,
            NonNull::from(&a),
            0,
            std::ptr::null(),
            len as u32,
            NonNull::new_unchecked(data as *mut c_void),
        )
    };
    check("AudioObjectSetPropertyData", status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_shows_fourcc() {
        let e = CaError { op: "x", status: i32::from_be_bytes(*b"!dev") };
        assert!(e.to_string().contains("'!dev'"));
        assert!(CaError { op: "x", status: -50 }.to_string().contains("-50"));
    }

    /// Reads the default output device. Needs a Mac with audio hardware (no permission needed).
    #[test]
    fn can_read_default_output_device() {
        let id = device::default_output().expect("default output");
        let info = device::info(id).expect("device info");
        assert!(info.sample_rate > 0.0, "{info:?}");
        assert!(!info.uid.is_empty());
    }
}
