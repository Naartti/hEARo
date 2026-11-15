//! `BUNDLE_ID` is copied into the app's Info.plist and XcodeGen spec; a rename must touch all
//! three (ADR 0005), or aggregate UIDs stop matching the app.

use std::path::Path;

fn read(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn info_plist_uses_the_engine_bundle_id() {
    let plist = read("app/Resources/Info.plist");
    let expected = format!("<key>CFBundleIdentifier</key>\n\t<string>{}</string>", hearo_engine::BUNDLE_ID);
    assert!(plist.contains(&expected), "Info.plist CFBundleIdentifier != {}", hearo_engine::BUNDLE_ID);
}

#[test]
fn xcodegen_spec_uses_the_engine_bundle_id() {
    let spec = read("app/project.yml");
    let expected = format!("PRODUCT_BUNDLE_IDENTIFIER: {}", hearo_engine::BUNDLE_ID);
    assert!(spec.contains(&expected), "project.yml PRODUCT_BUNDLE_IDENTIFIER != {}", hearo_engine::BUNDLE_ID);
}
