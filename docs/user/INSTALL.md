# Installing hEARo on another Mac

Requirements: Apple Silicon, macOS 26 or later.

1. Copy `hEARo-<version>.dmg` to the Mac (AirDrop, USB stick, iCloud Drive…) and open it.
2. Drag **hEARo** onto **Applications**.
3. First launch: the app is signed with a personal developer certificate but **not notarized**,
   so macOS refuses a plain double-click. Either right-click **hEARo** in Applications →
   **Open** → **Open**, or open it once, then System Settings → Privacy & Security →
   **Open Anyway**.
4. The ear appears in the menu bar. Play some audio: macOS asks to allow **System Audio
   Recording** for hEARo — click **Allow**. (If audio doesn't start, quit and reopen hEARo once.)
5. Press `§` after an annoying sound to save a snapshot ("missed it"). The ear shows a
   checkmark when it's saved. Snapshots stay on that Mac in
   `~/Library/Application Support/hEARo/snapshots/` until you export them.

Notes

- Keep one copy of hEARo per Mac (on the development Mac that is `~/Applications/hEARo.app`,
  installed by `just app` / `just install`; don't also drag the DMG's copy into Applications).
  Two running copies would each process the other's output.
- **About hEARo** (right-click the ear, or the popover's ⋯ menu) shows the version and build.
- To move snapshots between Macs, use **Export snapshots…** / **Import snapshots…** in the
  popover (or `hearo dataset export|import` on the dev Mac). Importing merges and skips
  duplicates.
- Uninstall: quit hEARo, delete it from Applications; optionally delete
  `~/Library/Application Support/hEARo`.
