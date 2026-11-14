# 0008. Hardened Runtime on, App Sandbox off, no network

Date: 2026-11-13 · Status: accepted

## Context

hEARo ships outside the App Store. Process taps and aggregate devices are simpler without the
sandbox; notarization requires Hardened Runtime.

## Decision

Hardened Runtime enabled, no App Sandbox in v1, no network entitlements or network code.
Entitlements file is empty; re-verify in M2 whether any entitlement is needed for taps.

## Alternatives considered

- **Sandboxed** — only needed for the App Store (a non-goal); complicates Application Support
  paths and device access.

## Consequences (incl. how we'd know it was wrong)

- Wrong if: macOS requires a sandbox or specific entitlement for process taps of other apps.
