//! List output devices; optionally make one the default and watch for device changes.
//!
//! ```sh
//! cargo run -p hearo-engine --example devices                 # list
//! cargo run -p hearo-engine --example devices -- --watch      # print each change (Ctrl-C)
//! cargo run -p hearo-engine --example devices -- --set "MacBook Pro Speakers"
//! ```
#![allow(clippy::print_stdout)] // a diagnostic that exists to print

use std::time::Duration;

use hearo_engine::ca::{device, watch::OutputWatcher};

fn list() {
    let default = device::default_output().unwrap_or(0);
    for d in device::outputs().unwrap_or_default() {
        let mark = if d.id == default { "*" } else { " " };
        println!(
            "{mark} {:<28} {:>6.0} Hz  {} ch  latency {:>5.1} ms  uid {}",
            d.name,
            d.sample_rate,
            d.output_channels,
            d.output_latency_frames as f64 * 1000.0 / d.sample_rate.max(1.0),
            d.uid
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(i) = args.iter().position(|a| a == "--set") {
        let name = args.get(i + 1).expect("--set <device name>");
        let d = device::outputs().unwrap_or_default().into_iter().find(|d| &d.name == name).expect("no such output");
        device::set_default_output(d.id).expect("set default output");
    }
    list();
    if args.iter().any(|a| a == "--watch") {
        let w = OutputWatcher::new().expect("listeners");
        let mut seen = w.generation();
        loop {
            let g = w.wait(seen, Duration::from_secs(3600));
            if g != seen {
                seen = g;
                println!("--- change #{g}");
                list();
            }
        }
    }
}
