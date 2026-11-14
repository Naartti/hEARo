//! Audio-producing process enumeration (`hearo procs`).

use objc2_core_audio::{
    kAudioHardwarePropertyProcessObjectList, kAudioHardwarePropertyTranslatePIDToProcessObject,
    kAudioProcessPropertyBundleID, kAudioProcessPropertyIsRunningOutput, kAudioProcessPropertyPID,
};

use super::{GLOBAL, ObjectId, Result, SYSTEM, get, get_qualified, get_string, get_vec};

/// One process known to Core Audio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    /// Core Audio process object.
    pub object: ObjectId,
    /// Unix PID.
    pub pid: i32,
    /// Bundle ID (may be empty for command-line tools).
    pub bundle_id: String,
    /// Currently sending audio to an output device.
    pub running_output: bool,
}

/// Every process Core Audio knows about.
pub fn list() -> Result<Vec<ProcessInfo>> {
    let objs: Vec<ObjectId> = get_vec(SYSTEM, kAudioHardwarePropertyProcessObjectList, GLOBAL)?;
    Ok(objs
        .into_iter()
        .map(|object| ProcessInfo {
            object,
            pid: get::<i32>(object, kAudioProcessPropertyPID, GLOBAL).unwrap_or(-1),
            bundle_id: get_string(object, kAudioProcessPropertyBundleID, GLOBAL).unwrap_or_default(),
            running_output: get::<u32>(object, kAudioProcessPropertyIsRunningOutput, GLOBAL).is_ok_and(|v| v != 0),
        })
        .collect())
}

/// Whether any process other than `own_pid` and the `excluded` objects is sending audio to an
/// output right now (the watchdog's "should the tap be running?"). Cheaper than [`list`]:
/// reads the PID only of running processes.
pub fn others_running_output(own_pid: i32, excluded: &[ObjectId]) -> Result<bool> {
    let objs: Vec<ObjectId> = get_vec(SYSTEM, kAudioHardwarePropertyProcessObjectList, GLOBAL)?;
    Ok(objs.into_iter().any(|o| {
        !excluded.contains(&o)
            && get::<u32>(o, kAudioProcessPropertyIsRunningOutput, GLOBAL).is_ok_and(|v| v != 0)
            && get::<i32>(o, kAudioProcessPropertyPID, GLOBAL).is_ok_and(|pid| pid != own_pid)
    }))
}

/// The process object for a PID, if Core Audio knows it.
pub fn object_for_pid(pid: i32) -> Result<Option<ObjectId>> {
    let obj: ObjectId = get_qualified(SYSTEM, kAudioHardwarePropertyTranslatePIDToProcessObject, GLOBAL, Some(&pid))?;
    Ok((obj != 0).then_some(obj))
}

/// A friendlier name for helper bundle IDs (WebKit/Chromium/Electron helpers).
pub fn display_name(bundle_id: &str) -> String {
    let known = [
        ("com.apple.WebKit", "Safari (WebKit)"),
        ("com.microsoft.teams", "Microsoft Teams"),
        ("com.tinyspeck.slackmacgap", "Slack"),
        ("com.google.Chrome", "Google Chrome"),
        ("com.spotify.client", "Spotify"),
        ("com.apple.Music", "Music"),
        ("com.apple.podcasts", "Podcasts"),
    ];
    known.iter().find(|(p, _)| bundle_id.starts_with(p)).map_or_else(
        || if bundle_id.is_empty() { "(no bundle id)".into() } else { bundle_id.into() },
        |(_, n)| (*n).into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friendly_names() {
        assert_eq!(display_name("com.apple.WebKit.GPU"), "Safari (WebKit)");
        assert_eq!(display_name("com.tinyspeck.slackmacgap.helper"), "Slack");
        assert_eq!(display_name("org.example"), "org.example");
        assert_eq!(display_name(""), "(no bundle id)");
    }

    #[test]
    fn can_list_processes_and_find_ourselves() {
        let procs = list().expect("process list");
        // Our own test process may not be registered with Core Audio unless it did audio,
        // so only check the call works and PIDs are sane.
        assert!(procs.iter().all(|p| p.object != 0));
        let _ = object_for_pid(std::process::id() as i32).expect("translate pid");
        let all: Vec<ObjectId> = procs.iter().map(|p| p.object).collect();
        assert!(!others_running_output(-1, &all).expect("query"), "everything excluded → nothing plays");
    }
}
