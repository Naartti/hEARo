## What and why

<!-- Link the issue this addresses. -->

## Checklist

- [ ] `just ci` passes locally (the PR check re-runs it; DSP changes: `just bench` on a Mac too)
- [ ] Tests written first; no test weakened or deleted
- [ ] Real-time code (anything reachable from `Module::process`, `Chain::process` or the
      IOProc) does not allocate, lock, block, log or panic
- [ ] Golden snapshot changes (if any) are explained in the commit message
- [ ] Docs updated in the same commits as the code; ADR added for non-trivial decisions
- [ ] Conventional Commits, one logical change each

## How to verify

<!-- Steps a reviewer can follow, e.g. in the app or with `just cli`. -->
