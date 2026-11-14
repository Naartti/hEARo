//! Process taps (macOS 14.2+; bundle-ID selection macOS 26+).

use objc2::AllocAnyThread;
use objc2_core_audio::{
    AudioHardwareCreateProcessTap, AudioHardwareDestroyProcessTap, CATapDescription, CATapMuteBehavior,
    kAudioTapPropertyUID,
};
use objc2_foundation::{NSArray, NSNumber, NSString};

use super::{GLOBAL, ObjectId, Result, check, get_string};

/// A private process tap, destroyed on drop.
#[derive(Debug)]
pub struct Tap {
    id: ObjectId,
    uid: String,
}

impl Tap {
    /// A stereo mixdown of every process *except* `exclude` (process objects), muting the
    /// tapped processes only while the tap is read (`MutedWhenTapped`): if hEARo stops reading
    /// or dies, macOS plays their audio normally again.
    pub fn global_excluding(exclude: &[ObjectId], name: &str) -> Result<Self> {
        let ids: Vec<_> = exclude.iter().map(|id| NSNumber::new_u32(*id)).collect();
        let ids = NSArray::from_retained_slice(&ids);
        // SAFETY: standard Objective-C init on a freshly allocated object.
        let desc = unsafe { CATapDescription::initStereoGlobalTapButExcludeProcesses(CATapDescription::alloc(), &ids) };
        Self::create(&desc, name)
    }

    /// Like [`global_excluding`](Self::global_excluding), but tapping what the processes play
    /// into stream `stream` of device `device_uid`: the tap then has that stream's channel count
    /// and, crucially, **runs at the device's sample rate**. A stereo-mixdown tap runs at the
    /// system mix rate (48 kHz) and a tap-backed aggregate on a 44.1 kHz device never starts
    /// (ADR 0010).
    pub fn device_excluding(exclude: &[ObjectId], device_uid: &str, stream: usize, name: &str) -> Result<Self> {
        let ids: Vec<_> = exclude.iter().map(|id| NSNumber::new_u32(*id)).collect();
        let ids = NSArray::from_retained_slice(&ids);
        // SAFETY: standard Objective-C init on a freshly allocated object.
        let desc = unsafe {
            CATapDescription::initExcludingProcesses_andDeviceUID_withStream(
                CATapDescription::alloc(),
                &ids,
                &NSString::from_str(device_uid),
                stream as _,
            )
        };
        Self::create(&desc, name)
    }

    fn create(desc: &CATapDescription, name: &str) -> Result<Self> {
        let mut id: ObjectId = 0;
        // SAFETY: valid description and out-pointer.
        unsafe {
            desc.setName(&NSString::from_str(name));
            desc.setPrivate(true);
            desc.setMuteBehavior(CATapMuteBehavior::MutedWhenTapped);
            check("AudioHardwareCreateProcessTap", AudioHardwareCreateProcessTap(Some(desc), &mut id))?;
        }
        match get_string(id, kAudioTapPropertyUID, GLOBAL) {
            Ok(uid) => Ok(Self { id, uid }),
            Err(e) => {
                // SAFETY: destroying the tap we just created.
                unsafe { AudioHardwareDestroyProcessTap(id) };
                Err(e)
            }
        }
    }

    /// Tap object ID.
    pub fn id(&self) -> ObjectId {
        self.id
    }

    /// Tap UID (goes into the aggregate device's tap list).
    pub fn uid(&self) -> &str {
        &self.uid
    }
}

impl Drop for Tap {
    fn drop(&mut self) {
        // SAFETY: we own this tap.
        unsafe { AudioHardwareDestroyProcessTap(self.id) };
    }
}
