//! The private aggregate device: real output device (clock master) + our tap.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_core_audio::{
    AudioHardwareCreateAggregateDevice, AudioHardwareDestroyAggregateDevice, kAudioAggregateDeviceIsPrivateKey,
    kAudioAggregateDeviceIsStackedKey, kAudioAggregateDeviceMainSubDeviceKey, kAudioAggregateDeviceNameKey,
    kAudioAggregateDeviceSubDeviceListKey, kAudioAggregateDeviceTapAutoStartKey, kAudioAggregateDeviceTapListKey,
    kAudioAggregateDeviceUIDKey, kAudioSubDeviceUIDKey, kAudioSubTapDriftCompensationKey, kAudioSubTapUIDKey,
};
use objc2_core_foundation::CFDictionary;
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString};

use super::{ObjectId, Result, check};

/// A private aggregate device, destroyed on drop (and by macOS if we die).
#[derive(Debug)]
pub struct Aggregate {
    id: ObjectId,
    uid: String,
}

fn key(k: &std::ffi::CStr) -> Retained<NSString> {
    NSString::from_str(&k.to_string_lossy())
}

fn dict(pairs: &[(&std::ffi::CStr, &AnyObject)]) -> Retained<NSDictionary<NSString, AnyObject>> {
    let keys: Vec<Retained<NSString>> = pairs.iter().map(|(k, _)| key(k)).collect();
    let key_refs: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
    let values: Vec<&AnyObject> = pairs.iter().map(|(_, v)| *v).collect();
    NSDictionary::from_slices(&key_refs, &values)
}

impl Aggregate {
    /// Create `name` with `output_uid` as main (clock) sub-device and `tap_uid` as a tap with
    /// drift compensation. Output and tap then share one IOProc and one clock.
    pub fn create(name: &str, uid: &str, output_uid: &str, tap_uid: &str) -> Result<Self> {
        let one = NSNumber::new_i32(1);
        let zero = NSNumber::new_i32(0);
        let out_uid = NSString::from_str(output_uid);
        let tap_uid_s = NSString::from_str(tap_uid);
        let name_s = NSString::from_str(name);
        let uid_s = NSString::from_str(uid);

        let sub = dict(&[(kAudioSubDeviceUIDKey, &out_uid)]);
        let tap = dict(&[(kAudioSubTapUIDKey, &tap_uid_s), (kAudioSubTapDriftCompensationKey, &one)]);
        let subs = NSArray::from_retained_slice(&[sub]);
        let taps = NSArray::from_retained_slice(&[tap]);
        let desc = dict(&[
            (kAudioAggregateDeviceNameKey, &name_s),
            (kAudioAggregateDeviceUIDKey, &uid_s),
            (kAudioAggregateDeviceMainSubDeviceKey, &out_uid),
            (kAudioAggregateDeviceIsPrivateKey, &one),
            (kAudioAggregateDeviceIsStackedKey, &zero),
            (kAudioAggregateDeviceTapAutoStartKey, &one),
            (kAudioAggregateDeviceSubDeviceListKey, &subs),
            (kAudioAggregateDeviceTapListKey, &taps),
        ]);
        // NSDictionary is toll-free bridged to CFDictionary.
        let cf: &CFDictionary = unsafe { &*(Retained::as_ptr(&desc) as *const CFDictionary) };
        let mut id: ObjectId = 0;
        // SAFETY: valid dictionary and out-pointer.
        check("AudioHardwareCreateAggregateDevice", unsafe {
            AudioHardwareCreateAggregateDevice(cf, std::ptr::NonNull::from(&mut id))
        })?;
        Ok(Self { id, uid: uid.to_string() })
    }

    /// Device ID (add the IOProc here).
    pub fn id(&self) -> ObjectId {
        self.id
    }

    /// Our UID for it.
    pub fn uid(&self) -> &str {
        &self.uid
    }
}

impl Drop for Aggregate {
    fn drop(&mut self) {
        // SAFETY: we own this aggregate device.
        unsafe { AudioHardwareDestroyAggregateDevice(self.id) };
    }
}
