//! Output-device queries.

use objc2_core_audio::{
    kAudioDevicePropertyBufferFrameSize, kAudioDevicePropertyDeviceUID, kAudioDevicePropertyLatency,
    kAudioDevicePropertyNominalSampleRate, kAudioDevicePropertySafetyOffset, kAudioDevicePropertyStreamConfiguration,
    kAudioDevicePropertyStreams, kAudioHardwarePropertyDefaultOutputDevice, kAudioObjectPropertyName,
};

use super::{GLOBAL, INPUT, OUTPUT, ObjectId, Result, SYSTEM, get, get_raw, get_string, get_vec};

/// What we need to know about an output device.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceInfo {
    /// Object ID.
    pub id: ObjectId,
    /// Persistent UID.
    pub uid: String,
    /// Display name.
    pub name: String,
    /// Nominal sample rate.
    pub sample_rate: f64,
    /// Output channels across all output streams.
    pub output_channels: u32,
    /// Number of input streams (an aggregate's own inputs come before its taps).
    pub input_streams: u32,
    /// IO buffer size in frames.
    pub buffer_frames: u32,
    /// Device output latency + safety offset, in frames.
    pub output_latency_frames: u32,
}

/// The current default output device.
pub fn default_output() -> Result<ObjectId> {
    get(SYSTEM, kAudioHardwarePropertyDefaultOutputDevice, GLOBAL)
}

/// Every device with output channels (for diagnostics and the device-switch test).
pub fn outputs() -> Result<Vec<DeviceInfo>> {
    let ids: Vec<ObjectId> = get_vec(SYSTEM, objc2_core_audio::kAudioHardwarePropertyDevices, GLOBAL)?;
    Ok(ids.into_iter().filter_map(|id| info(id).ok()).filter(|d| d.output_channels > 0).collect())
}

/// Make `id` the system default output (as choosing it in Control Center does).
pub fn set_default_output(id: ObjectId) -> Result<()> {
    super::set(SYSTEM, kAudioHardwarePropertyDefaultOutputDevice, GLOBAL, &id)
}

/// What the engine is bound to: the default output's UID and nominal sample rate. If either
/// changes, tap + aggregate must be rebuilt.
pub fn default_output_key() -> Result<(String, f64)> {
    let id = default_output()?;
    Ok((
        get_string(id, kAudioDevicePropertyDeviceUID, GLOBAL)?,
        get(id, kAudioDevicePropertyNominalSampleRate, GLOBAL)?,
    ))
}

/// Total channels in a device's stream configuration for `scope`.
pub fn channels(id: ObjectId, scope: u32) -> Result<u32> {
    let raw = get_raw(id, kAudioDevicePropertyStreamConfiguration, scope)?;
    // AudioBufferList: u32 mNumberBuffers, (pad), then AudioBuffer { u32 channels, u32 size, *mut data }.
    let words: &[u32] = bytemuck_u32(&raw);
    let n = words[0] as usize;
    Ok((0..n).map(|i| words.get(2 + i * 4).copied().unwrap_or(0)).sum())
}

fn bytemuck_u32(v: &[u64]) -> &[u32] {
    // SAFETY: u64 storage is valid and aligned for u32 reads; length doubles.
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u32>(), v.len() * 2) }
}

/// Collect device info.
pub fn info(id: ObjectId) -> Result<DeviceInfo> {
    let latency: u32 = get(id, kAudioDevicePropertyLatency, OUTPUT).unwrap_or(0);
    let safety: u32 = get(id, kAudioDevicePropertySafetyOffset, OUTPUT).unwrap_or(0);
    Ok(DeviceInfo {
        id,
        uid: get_string(id, kAudioDevicePropertyDeviceUID, GLOBAL)?,
        name: get_string(id, kAudioObjectPropertyName, GLOBAL).unwrap_or_default(),
        sample_rate: get(id, kAudioDevicePropertyNominalSampleRate, GLOBAL)?,
        output_channels: channels(id, OUTPUT).unwrap_or(0),
        input_streams: get_vec::<ObjectId>(id, kAudioDevicePropertyStreams, INPUT).map_or(0, |v| v.len() as u32),
        buffer_frames: get(id, kAudioDevicePropertyBufferFrameSize, GLOBAL).unwrap_or(0),
        output_latency_frames: latency + safety,
    })
}
