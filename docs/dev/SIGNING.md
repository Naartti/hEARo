# Signing

Why it matters: macOS ties the System Audio Recording permission to the app's signature.
Ad-hoc builds lose it on every rebuild and process taps then silently return zeros
([ADR 0005](../adr/0005-stable-dev-signing.md), deep dive 13).

## Create a stable identity (once)

- **With Xcode:** Settings → Accounts → add Apple ID → Manage Certificates → "+" → Apple Development.
- **Without Xcode:** Keychain Access → Certificate Assistant → Create a Certificate… → Name
  `hEARo Signing`, Identity Type *Self Signed Root*, Certificate Type *Code Signing*.

```sh
security find-identity -v -p codesigning
echo 'SIGN_IDENTITY="hEARo Signing"' > .env.local   # use the exact name shown; git-ignored
just doctor
```

**Troubleshooting:** `security find-identity -p codesigning` lists the cert but
`-v` says "0 valid identities" → the Apple WWDR **G3** intermediate is missing (common when an
old, expired WWDR cert from a previous Xcode is all that's installed). Install
<https://www.apple.com/certificateauthority/AppleWWDRCAG3.cer> into the login keychain.

## One app, one copy

Every build is the same app: **hEARo**, bundle ID `io.github.naartti.hearo`, installed at
`~/Applications/hEARo.app`. Debug and release differ only in build configuration (the About
panel shows "· debug" and the git hash for `just app` builds), never in name, so the System
Audio Recording permission and the login item belong to one app.

- `just app` builds (debug), signs (`scripts/sign.sh`, Hardened Runtime) and installs it via
  `scripts/install-app.sh`: a running hEARo is quit first and reopened afterwards.
- `just release` builds `dist/hEARo.app` + the DMG; `just install` puts that build at the
  same path (replacing the debug one). The DMG's `/Applications` is for *other* Macs.
- Keep one copy per Mac. Two copies with the same bundle ID would confuse Launch Services and
  the login item, and two running engines would each tap the other's output. The install
  script warns if it finds `/Applications/hEARo.app` or the old `~/Applications/hEARo Dev.app`.
- `HEARO_APP_DEST=/tmp/x/hEARo.app scripts/build-app.sh debug` builds a bundle elsewhere
  without touching the installed app (for inspecting `Info.plist`, signature, icon).

The fixed bundle ID, path and identity keep the permission across rebuilds. Ad-hoc only with
`ALLOW_ADHOC=1` (expect re-prompts).

**Migrating from `dev.hearo.app`** (builds before 2026-11-15): run `just app` (or install the
new release), allow **System Audio Recording** again when asked, and re-enable *Launch at
login* if you used it — both belong to the bundle ID. Preferences (auto-save, Review options,
window positions) are copied once from the old ID at launch (`DefaultsMigration`); module
settings live in `settings.json` and need nothing. Remove the stale entries in System Settings
→ Privacy & Security and → General → Login Items & Extensions.

**Migrating from "hEARo Dev"** (builds before 2026-11-14 used `dev.hearo.app.dev`): quit
*hEARo Dev*, delete `~/Applications/hEARo Dev.app`, run `just app`, allow **System Audio
Recording** again when asked (it is a new bundle ID), and re-enable *Launch at login* if you
used it. Optionally remove the stale entry in System Settings → Privacy & Security.

## App icon

Source: `app/Resources/Icon/hEARo.svg` (master, sizes ≥ 64 px) and `hEARo-small.svg` (16 and
32 px: the ear alone, heavier strokes). `scripts/make-icon.swift` renders both with AppKit's
SVG support into an `.iconset` and packs it with `iconutil`; `just app`/`just release` do this
automatically when a source changed, `just icon` does it on demand (PNGs in
`target/icon/hEARo.iconset/` for a look). Only the SVGs are committed: the `.icns` is 1 MB
of build output and takes ~2 s to regenerate. The DMG uses the same `.icns` as its volume icon.
Edit the SVG in any vector editor; keep the 824 × 824 body at (100, 100) on the 1024 canvas.
