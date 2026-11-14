# Installing hEARo

Requirements: Apple silicon, macOS 26 or later.

## Download

1. Download `hEARo-<version>.dmg` from the
   [latest release](https://github.com/Naartti/hEARo/releases/latest), open it and drag
   **hEARo** onto **Applications**.
2. **First launch.** hEARo isn't notarized by Apple (that needs a paid developer account), so
   the first double-click shows *"hEARo" Not Opened* — click **Done**. Then open System
   Settings → Privacy & Security, scroll to *Security*, click **Open Anyway** next to the hEARo
   message and confirm. You only do this once per version.
3. The ear appears in the menu bar. Play some audio: macOS asks to allow **System Audio
   Recording** for hEARo — click **Allow**. (If audio doesn't start, quit and reopen hEARo once.)
4. Press `§` after an annoying sound to save a snapshot ("missed it"). The ear shows a
   checkmark when it's saved. Snapshots stay on that Mac in
   `~/Library/Application Support/hEARo/snapshots/` until you export them.

**Updating:** quit hEARo, replace it in Applications with the new version and repeat step 2.
Release builds are signed ad-hoc, so macOS treats each version as a new app: it asks for System
Audio Recording again, and an old *hEARo* entry may linger in System Settings → Privacy &
Security → Screen & System Audio Recording (remove it with **−**).

**Check the download (optional):** each release has a `.sha256` file and a build-provenance
attestation that proves GitHub Actions built the DMG from this repository:

```sh
shasum -a 256 -c hEARo-<version>.dmg.sha256
gh attestation verify hEARo-<version>.dmg --repo Naartti/hEARo
```

## Build it yourself

Clone the repository and follow *Build from source* in the [README](../../README.md#build-from-source).
`just release` builds `dist/hEARo-<version>-<sha>.dmg` signed with your own identity; copy it
to another Mac you own (AirDrop, USB stick…) and install it as above.

## Notes

- Keep one copy of hEARo per Mac (on a development Mac that is `~/Applications/hEARo.app`,
  installed by `just app` / `just install`; don't also drag a DMG's copy into Applications).
  Two running copies would each process the other's output.
- **About hEARo** (right-click the ear, or the popover's ⋯ menu) shows the version and build.
- To move snapshots between Macs, use **Export snapshots…** / **Import snapshots…** in the
  popover (or `hearo dataset export|import` on the dev Mac). Importing merges and skips
  duplicates.
- Uninstall: quit hEARo, delete it from Applications; optionally delete
  `~/Library/Application Support/hEARo`.
