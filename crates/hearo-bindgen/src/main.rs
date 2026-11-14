//! The UniFFI bindings generator, pinned to the same uniffi version as the library
//! (they must match exactly). Used by `scripts/build-ffi.sh`.
fn main() {
    uniffi::uniffi_bindgen_main()
}
