# 0013. License exception: UniFFI (MPL-2.0)

Date: 2026-11-13 · Status: accepted (confirmed by the human 2026-11-14: personal use; any distribution would be fully open source, which MPL-2.0 allows)

## Context

HANDOVER §2 allows only permissive licenses (MIT, Apache-2.0, BSD, ISC, Zlib); §6 prescribes
UniFFI for the Swift ↔ Rust bridge. UniFFI 0.32.2 is **MPL-2.0**, so `cargo deny` rejected it
once the bridge landed. Its bindings generator also pulls `basic-toml`, which is unmaintained
(RUSTSEC-2026-0374, no fix available).

MPL-2.0 is a *file-level* copyleft: if you modify MPL-covered files you must make those files'
source available. Using the library unmodified — in a personal or proprietary app — imposes no
obligation on our own code beyond keeping the notice.

## Decision

- Allow MPL-2.0 **only** for the eight `uniffi*` crates (explicit `deny.toml` exceptions, not a
  blanket allow). We use them unmodified; THIRD_PARTY_NOTICES.md names them.
- Move the bindings generator into its own build-tool crate (`hearo-bindgen`); the shipped
  `hearo-ffi` library no longer enables UniFFI's `cli` feature.
- Ignore RUSTSEC-2026-0374 with a reason: `basic-toml` is reached only via the build-time
  generator (an unmaintained TOML parser reading our own `uniffi.toml`, never untrusted input).

## Alternatives considered

- **Hand-written C ABI + Swift wrappers** — permissive, but more unsafe code and no generated,
  typed API; revisit if the human prefers strict permissive-only.
- **swift-bridge** (MIT/Apache) — smaller ecosystem, would mean rewriting the bridge.

## Consequences (incl. how we'd know it was wrong)

- `just ci` is green again with a narrowly scoped exception.
- Wrong if: the human wants strict permissive-only (then switch to swift-bridge or a C ABI), or
  we ever need to patch UniFFI itself (then publish those patched files).
