# Releasing

How to keep the changelog current and how to publish a new version of hEARo. Public releases
are built by GitHub Actions ([`release.yml`](../../.github/workflows/release.yml),
[ADR 0016](../adr/0016-github-actions-ci-and-releases.md)) from a `v<version>` tag. They are
ad-hoc signed and not notarized, and ship as a DMG with a `.sha256` file and a build-provenance
attestation. Nothing is built on your Mac for a public release.

## Between releases: one changelog fragment per PR

[CHANGELOG.md](../../CHANGELOG.md) follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Every PR with a change a user would notice adds its entry as a **fragment**,
`changelog.d/<branch>.md` (format: [changelog.d/README.md](../../changelog.d/README.md)), in the
same commit as the change. Use the heading that fits: *Added*, *Changed*, *Deprecated*,
*Removed*, *Fixed* or *Security*. `just changelog` previews the next release's section.

**Why fragments:** when every PR inserted its line under the same `## Unreleased` heading, any
two open PRs conflicted, and GitHub's rebase merge can't resolve that (PRs #15–#17,
2026-11-16). A new file per PR never conflicts. The same goes for
[PROGRESS.md](../PROGRESS.md): feature PRs write `docs/progress.d/<branch>.md` instead. Only
`chore/release-*` and `docs/rollup-*` branches may edit CHANGELOG.md or PROGRESS.md;
`just check` (`scripts/changelog.py check`) fails any other branch that does.

- **Needs an entry:** anything in the app, the engine's behaviour, DSP results, the `hearo` CLI
  and dataset format, install steps, and dependency bumps that fix a user-facing bug or
  security advisory.
- **No entry:** refactors with no visible effect, tests, CI and tooling, agent instructions,
  and internal docs.
- **How to write it:** one line per change, written for the user ("De-smack leaves quiet and
  dull bursts alone"), not as a commit subject. A later PR that changes the same behaviour
  before the release ships edits that PR's fragment rather than adding a contradicting one.

At release time the fragments should already be complete. The release PR only folds them in.

## Cut a release

1. **Check the fragments against the history.** `git fetch --tags origin`, then
   `git log --oneline v<last>..origin/main` lists every commit since the last tag, and
   `just changelog` shows the fragments. Every `feat` and every user-visible `fix` should have
   a line. Add any that are missing as new fragments. Merged Dependabot PRs touch only manifests and lockfiles
   ([ADR 0019](../adr/0019-dependency-update-routine.md)), so a security fix among them gets
   its *Security* line here.
2. **Bump the version on a branch** (`chore/release-v<version>`). Before 1.0, new features
   bump the minor version (0.1.0 → 0.2.0) and fixes alone bump the patch (0.2.0 → 0.2.1);
   from 1.0 on, [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Change `version` in
   [Cargo.toml](../../Cargo.toml) (the workspace version, which the app bundle also uses) and
   [ml/pyproject.toml](../../ml/pyproject.toml) plus `__version__` in
   `ml/hearo_ml/__init__.py`, then run `(cd ml && uv lock)`; `just check`
   updates `Cargo.lock` but runs uv with `--frozen`, which doesn't. Then
   `scripts/changelog.py release <version> <YYYY-MM-DD>` writes the
   `## [<version>] - <YYYY-MM-DD>` section from the fragments and deletes them. Commit
   `chore(release): v<version>`, open a PR and wait for green checks; the maintainer merges
   it.
3. **Optional dry run.** `gh workflow run release.yml --ref main` builds without publishing.
   Download the artifact and check it as in step 5.
4. **Tag.** Wait until CI on the merged `main` commit is green, then
   `git switch main && git pull --ff-only && git tag v<version> && git push origin v<version>`,
   and follow the run with `gh run watch`. The workflow refuses a tag that doesn't match
   `Cargo.toml`, isn't on `main`, or has no successful CI run. A version with a pre-release
   suffix (`0.2.0-rc.1`) becomes a GitHub pre-release and never "latest".
5. **Verify the published DMG** on a Mac:

   ```sh
   gh release download v<version> --pattern 'hEARo-*'
   shasum -a 256 -c hEARo-<version>.dmg.sha256
   gh attestation verify hEARo-<version>.dmg --repo Naartti/hEARo
   ```

   Mount it: the volume is "hEARo" with the icon, the app, an Applications link and the
   read-me. `codesign -dv` on the app shows `Signature=adhoc` and no Authority line (no
   personal certificate). Install it as described in [user/INSTALL.md](../user/INSTALL.md).
   About hEARo shows the new version without "· debug".

## Local builds are not releases

`just release` builds a DMG in `dist/` signed with your personal identity from `.env.local`
([SIGNING.md](SIGNING.md)). That certificate carries your name, so **never upload it**. It is
for your own Macs; `just install` installs it as the Mac's one copy at
`~/Applications/hEARo.app`.

Notarized releases need a paid Developer ID (H5 in [OPEN_QUESTIONS.md](../OPEN_QUESTIONS.md)).
Until then, users allow the app once per version via System Settings → Privacy & Security →
Open Anyway.
