## What and why

<!-- Link the issue this addresses. -->

## Checklist

- [ ] `just ci` passes locally (the PR check re-runs it; DSP changes: `just bench` on a Mac too)
- [ ] `just patch-coverage` passes (new Rust code is tested; Codecov checks the same)
- [ ] Tests written first; no test weakened or deleted
- [ ] Real-time code (anything reachable from `Module::process`, `Chain::process` or the
      IOProc) does not allocate, lock, block, log or panic
- [ ] Golden snapshot changes (if any) are explained in the commit message
- [ ] Docs updated in the same commits as the code; ADR added for non-trivial decisions
- [ ] User-visible change: a line under *Unreleased* in CHANGELOG.md (none needed for internal
      changes; see docs/dev/RELEASING.md)
- [ ] Conventional Commits, one logical change each

## How to verify

<!-- Steps a reviewer can follow, e.g. in the app or with `just cli`. -->
