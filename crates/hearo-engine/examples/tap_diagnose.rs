//! Diagnose live-audio problems: permission state, tap format vs output device, and whether a
//! plain device / aggregate with a bogus tap / aggregate with real taps run their IOProc.
//! Play some audio while running it — tap-backed aggregates are idle while nothing plays.
//! Run: `just diagnose` (or `cargo run -p hearo-engine --example tap_diagnose`).
#![allow(clippy::print_stdout, unsafe_code)]
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};

use hearo_engine::ca::{GLOBAL, aggregate::Aggregate, device, get, tap::Tap};
use objc2_core_audio::*;
use objc2_core_audio_types::*;

static N: AtomicU64 = AtomicU64::new(0);
unsafe extern "C-unwind" fn cb(
    _: AudioObjectID,
    _: NonNull<AudioTimeStamp>,
    _: NonNull<AudioBufferList>,
    _: NonNull<AudioTimeStamp>,
    _: NonNull<AudioBufferList>,
    _: NonNull<AudioTimeStamp>,
    _: *mut c_void,
) -> i32 {
    N.fetch_add(1, Ordering::Relaxed);
    0
}

fn run(label: &str, dev: AudioObjectID) {
    N.store(0, Ordering::Relaxed);
    let mut id: AudioDeviceIOProcID = None;
    unsafe {
        AudioDeviceCreateIOProcID(dev, Some(cb), std::ptr::null_mut(), NonNull::from(&mut id));
        let st = AudioDeviceStart(dev, id);
        std::thread::sleep(std::time::Duration::from_millis(700));
        println!("  {label:<44} start={st:<3} callbacks in 700 ms = {}", N.load(Ordering::Relaxed));
        AudioDeviceStop(dev, id);
        AudioDeviceDestroyIOProcID(dev, id);
    }
}

fn main() {
    println!("System Audio Recording permission: {:?}", hearo_engine::ca::permission::audio_capture());
    println!("NOTE: taps only run while a tapped app plays audio — start something playing first!");
    let out = device::info(device::default_output().unwrap()).unwrap();
    println!("output: {} @ {} Hz, {} ch, buffer {}", out.name, out.sample_rate, out.output_channels, out.buffer_frames);

    let t = Tap::global_excluding(&[], "diagnose").unwrap();
    match get::<AudioStreamBasicDescription>(t.id(), kAudioTapPropertyFormat, GLOBAL) {
        Ok(f) => println!(
            "tap format: {} Hz, {} ch, flags {:#x}, {} bits",
            f.mSampleRate, f.mChannelsPerFrame, f.mFormatFlags, f.mBitsPerChannel
        ),
        Err(e) => println!("tap format: {e}"),
    }

    println!("IO tests:");
    run("plain output device", out.id);
    let a = Aggregate::create("diag-bogus", "io.github.naartti.hearo.diag.bogus", &out.uid, "no-such-tap").unwrap();
    run("aggregate + bogus tap", a.id());
    drop(a);
    let a = Aggregate::create("diag-real", "io.github.naartti.hearo.diag.real", &out.uid, t.uid()).unwrap();
    let agg = device::info(a.id()).unwrap();
    println!(
        "  (aggregate opened @ {} Hz, buffer {}, {} input streams)",
        agg.sample_rate, agg.buffer_frames, agg.input_streams
    );
    run("aggregate + real tap (stereo mixdown)", a.id());
    drop(a);
    drop(t);

    let t = Tap::device_excluding(&[], &out.uid, 0, "diagnose-dev").unwrap();
    if let Ok(f) = get::<AudioStreamBasicDescription>(t.id(), kAudioTapPropertyFormat, GLOBAL) {
        println!("device-stream tap format: {} Hz, {} ch", f.mSampleRate, f.mChannelsPerFrame);
    }
    let a = Aggregate::create("diag-dev", "io.github.naartti.hearo.diag.dev", &out.uid, t.uid()).unwrap();
    run("aggregate + real tap (device stream)", a.id());
}
