---
name: release
description: Steps for cutting a hEARo release - version bump, changelog, tag, the GitHub release workflow, local DMGs. Use when preparing a release or DMG.
---

# Release

Public releases are built by GitHub Actions (`.github/workflows/release.yml`, ADR 0016) from a
`v<version>` tag: ad-hoc signed, not notarized, DMG + `.sha256` + provenance attestation.

1. On a branch: bump `version` in `Cargo.toml` (workspace) and `ml/pyproject.toml`; move
   CHANGELOG.md's *Unreleased* items under `## [<version>] - <date>`. Commit
   `chore(release): v<version>`, open a PR, wait for green checks; the human merges.
2. Optional dry run: `gh workflow run release.yml --ref main`, then download the artifact and
   check it (step 4).
3. **Ask the human before tagging** — a pushed tag publishes a release. Then on the merged
   `main`: `git tag v<version> && git push origin v<version>`; watch with `gh run watch`.
4. Verify the published DMG on this Mac: `shasum -a 256 -c`, `gh attestation verify <dmg> --repo
   Naartti/hEARo`, `codesign -dv` shows `Signature=adhoc` and no Authority line (no personal
   certificate), mount it (volume "hEARo" with the icon, app + Applications link + read-me),
   About hEARo shows the new version without "· debug". Install follows docs/user/INSTALL.md.
5. Never upload a locally built DMG: `just release` signs with the personal identity from
   `.env.local` (the certificate carries the owner's name). It's for the owner's own Macs;
   `just install` installs it as this Mac's one copy at `~/Applications/hEARo.app`.

Notarized releases need a paid Developer ID (H5 in docs/OPEN_QUESTIONS.md); until then
users allow the app once via System Settings → Privacy & Security → Open Anyway.
