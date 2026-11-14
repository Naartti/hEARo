#!/usr/bin/env bash
# `just ffi` — build the Rust engine as a static library, generate the UniFFI Swift bindings and
# package them as app/RustBridge/HearoFFI.xcframework + app/RustBridge/generated/hearo_ffi.swift
# (both generated, git-ignored, never hand-edited). Incremental: cargo does the heavy lifting.
# Usage: scripts/build-ffi.sh [release|debug]   (default: release — the audio path must be optimized)
. "$(dirname "$0")/lib.sh"
profile="${1:-release}"
target=aarch64-apple-darwin
flag=$([[ "$profile" == release ]] && echo --release || true)

cargo build $flag -q -p hearo-ffi --lib --target "$target"
lib_dir="target/$target/$profile"
out="app/RustBridge"
stamp="$out/.built-from"
# Skip regeneration when the library hasn't changed (keeps `just check` fast).
sig="$profile $(stat -f '%m %z' "$lib_dir/libhearo_ffi.a")"
if [[ -f "$stamp" && "$(cat "$stamp")" == "$sig" && -d "$out/HearoFFI.xcframework" && -f "$out/generated/hearo_ffi.swift" ]]; then
  exit 0
fi

gen="target/ffi/gen"
rm -rf "$gen" && mkdir -p "$gen"
cargo run -q -p hearo-bindgen --bin uniffi-bindgen -- generate --library "$lib_dir/libhearo_ffi.dylib" \
  --language swift --out-dir "$gen" >/dev/null

headers="target/ffi/headers"
rm -rf "$headers" && mkdir -p "$headers"
cp "$gen"/*.h "$headers/"
cp "$gen"/*.modulemap "$headers/module.modulemap"

mkdir -p "$out/generated"
xcf="$out/HearoFFI.xcframework"
rm -rf "$xcf"
dev="$(xcode_dev_dir)" || { echo "✘ Xcode is needed to build the XCFramework (xcodebuild -create-xcframework)"; exit 1; }
DEVELOPER_DIR="$dev" xcodebuild -create-xcframework -library "$lib_dir/libhearo_ffi.a" -headers "$headers" \
  -output "$xcf" >/dev/null
cp "$gen"/*.swift "$out/generated/"
echo "$sig" > "$stamp"
echo "✔ $xcf + $out/generated/$(cd "$gen" && ls *.swift)"
