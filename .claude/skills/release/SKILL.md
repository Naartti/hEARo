---
name: release
description: Steps for cutting a hEARo release - version bump, changelog, tag, the GitHub release workflow, local DMGs. Use when preparing a release or DMG.
---

# Release

The procedure lives in docs/dev/RELEASING.md (written for the human too). Read it and follow
it; this skill only adds what is specific to an agent doing it.

1. **Changelog first.** Compare `git log --oneline v<last>..main` with *Unreleased* in
   CHANGELOG.md. Add a user-facing line for every `feat` and user-visible `fix` that lacks one
   (read the commit or PR to word it for a user; merge lines about the same thing). A missing
   entry means a PR skipped the AGENTS.md step 5 rule. Note it in the report, don't just
   paper over it.
2. **Release PR.** On `chore/release-v<version>`: bump `Cargo.toml` + `ml/pyproject.toml` +
   `ml/hearo_ml/__init__.py`,
   `(cd ml && uv lock)`, rename *Unreleased*, add a fresh empty one, `just ci`, commit
   `chore(release): v<version>`, PR with the full template body. The human merges. Pick the
   version by RELEASING.md's rule and say why in the PR.
3. **Ask the human before tagging.** A pushed tag publishes a public release. Only tag when
   they say so, and only after CI on the merged `main` commit is green; then watch with
   `gh run watch` and verify the published DMG (RELEASING.md step 5).
4. **Never upload a locally built DMG.** `just release` signs with the owner's personal
   certificate, which carries their name.
