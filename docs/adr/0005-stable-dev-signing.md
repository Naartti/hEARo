# 0005. Stable signing identity for dev builds

Date: 2026-11-13 · Status: accepted (amended 2026-11-14)

## Context

TCC permissions (System Audio Recording) are tied to bundle ID + code signature. Ad-hoc
signatures pin the grant to a cdhash, so every rebuild silently loses permission.

## Decision

Dev builds are signed with a stable identity named in `.env.local` (`SIGN_IDENTITY`, git-ignored):
an "Apple Development" certificate or a self-signed code-signing certificate. Fixed bundle IDs
(`dev.hearo.app.dev` dev, `dev.hearo.app` release) and a fixed install path
(`~/Applications/hEARo Dev.app`). `scripts/sign.sh` refuses ad-hoc unless `ALLOW_ADHOC=1`.

**Amendment (2026-11-14):** the human asked to drop the "Dev" suffix. Debug and release are now
one app, `hEARo` (`dev.hearo.app`), installed at `~/Applications/hEARo.app` by
`scripts/install-app.sh` (one copy per Mac; see docs/dev/SIGNING.md). The stable-identity rule is
unchanged.

**Amendment (2026-11-15):** the owner didn't want the `dev` prefix (it implies owning
`hearo.dev`). The bundle ID is now `io.github.naartti.hearo`: reverse DNS of the GitHub Pages
namespace the owner controls, the usual choice for an open-source project without its own
domain. Users re-grant System Audio Recording once; preferences are copied from the old ID at
first launch. If the project gets its own domain, rename again the same way.

## Alternatives considered

- **Ad-hoc + re-grant every build** — silent failures (taps return zeros), awful loop.

## Consequences (incl. how we'd know it was wrong)

- One-time human setup (H1). Wrong if: permission is still lost across rebuilds with a stable
  identity (verify in M3 by rebuilding twice and checking the tap isn't silent).
