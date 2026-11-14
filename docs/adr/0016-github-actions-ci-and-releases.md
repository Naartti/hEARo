# 0016. GitHub Actions for CI and releases, local gate stays first

Date: 2026-11-14 · Status: accepted (supersedes [0009](0009-local-ci-no-hosted-ci.md))

## Context

ADR 0009 ruled out hosted CI for one reason: cost of macOS runner minutes on a personal
project. The repository is now public on GitHub (`Naartti/hEARo`), and standard GitHub-hosted
runners, macOS included, are free and unmetered for public repositories. The `macos-26` arm64
image is generally available (GitHub changelog, 2026-02-26) with Xcode 26.x installed, which
covers our `swift-tools-version: 6.2` / macOS 26 target. The owner wants a normal open-source
flow: pull requests with a visible check, and a downloadable app for other Macs without
cloning the repo. ADR 0009 also named its own weakness: no clean-machine verification.

## Decision

We use GitHub Actions, with `just ci` staying the single definition of the gate.

- **`ci.yml`** runs on every pull request and push to `main` on `macos-26`: `mise` installs the
  pinned toolchains from `.mise.toml`, then `just ci` runs unchanged. Under `GITHUB_ACTIONS`
  `scripts/ci.sh` skips only the CPU-budget step (timing on shared runners is noise; budgets
  stay enforced locally). A separate job measures Rust line coverage with `cargo-llvm-cov`
  (`scripts/coverage.sh`, `just coverage`) and uploads it to Codecov via OIDC (no secret).
- **`release.yml`** runs on a `v*` tag: checks the tag matches the workspace version, builds the
  release app and DMG with the same scripts as `just release`, signs **ad-hoc**, and publishes a
  GitHub Release with the DMG, a SHA-256 file and a build-provenance attestation.
- Actions are pinned by commit SHA (with the tag as a comment), permissions default to
  read-only, and Dependabot proposes updates for actions and Cargo dependencies.
- The local gate stays first: `just check` before every commit and `just ci` on pre-push. The
  workflow is branch → PR → green check → rebase-merge on GitHub.

Release builds are ad-hoc signed on purpose. The only certificate available is a personal
Apple Development certificate, which carries the owner's legal name in the signature (`codesign
-dv` shows it) and is not meant for distribution. Ad-hoc keeps public binaries anonymous.
Locally built DMGs (`just release`) are still signed with the personal identity and are for
the owner's own Macs only.

## Alternatives considered

- **Keep local-only CI (0009)** — the cost reason is gone, and PRs without a visible check
  look unmaintained to contributors.
- **Developer ID + notarization in CI** — the smoothest install (no Gatekeeper override), but
  needs the paid Apple Developer Program, and an individual account's Developer ID also shows
  the legal name. Open question for the owner (H5 in OPEN_QUESTIONS.md).
- **Upload locally built DMGs to Releases** — no provenance, and they are signed with the
  personal certificate.
- **Self-hosted runner on this Mac** — CI would only run while the Mac is awake, and running
  untrusted PR code on a personal machine is a risk.

## Consequences (incl. how we'd know it was wrong)

- Every PR gets a clean-machine `just ci`; the fresh-clone run at milestone end becomes a
  sanity check rather than the only one.
- Users of a release must allow the app once in System Settings → Privacy & Security
  ("Open Anyway"), and macOS asks for System Audio Recording again after each update, because
  an ad-hoc signature changes with every build (docs/user/INSTALL.md).
- Hardware, live-tap and CPU-budget tests still run only on a real Mac.
- Wrong if: hosted runs are flaky for reasons unrelated to the code (more than one in ten red
  runs pass on re-run), or the runner image lags the macOS SDK we need — then gate on a
  self-hosted runner or drop back to local-only for the affected step.
