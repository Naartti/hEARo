# Documentation

hEARo keeps its documentation next to the code and changes it in the same commits. Start with
the project [README](../README.md); this page maps the rest.

## Understand the system

| Document | What it covers |
|---|---|
| [ARCHITECTURE.md](ARCHITECTURE.md) | Crates, the processing chain, real-time threading, fail-safes, output-device following |
| [DESIGN.md](DESIGN.md) | Product and UI decisions: menu bar icon, popover, labeling hotkey, copy |
| [adr/](adr/README.md) | Architecture Decision Records — why things are the way they are |
| [deep-dives/](deep-dives/README.md) | Learning-oriented explanations: Core Audio taps, real-time Rust, filters, dynamics, click repair |

## Use it

| Document | What it covers |
|---|---|
| [user/](user/README.md) | User guide: installing a build on another Mac, permissions, snapshots |

## Work on it

| Document | What it covers |
|---|---|
| [dev/SETUP.md](dev/SETUP.md) | Fresh clone to working environment (`just bootstrap`, `just doctor`) |
| [dev/LOCAL_CI.md](dev/LOCAL_CI.md) | CI: the `just check` / `just ci` gates, GitHub Actions, coverage, releasing, git hooks |
| [dev/RELEASING.md](dev/RELEASING.md) | Keeping the changelog current and publishing a new version (tag → GitHub release → DMG) |
| [dev/SIGNING.md](dev/SIGNING.md) | Why the app needs a stable signing identity and how to create one |
| [testing/STRATEGY.md](testing/STRATEGY.md) | Test layers: unit, property, real-time safety, golden audio, budgets |
| [testing/HARDWARE_CHECKLIST.md](testing/HARDWARE_CHECKLIST.md) | Manual checks that need real devices and a human ear |
| [ml/](ml/README.md) | Machine-learning pipeline (data, training, evaluation) — planned, not built yet |
| [../CONTRIBUTING.md](../CONTRIBUTING.md) | How to contribute |

## Project management (agent workflow)

Much of hEARo is developed together with AI coding agents (see [AGENTS.md](../AGENTS.md)).
These files are their working documents. They are kept in the repository on purpose, so the
history of decisions stays visible, but they are written for the maintainer and the agents
rather than for outside readers:

| Document | What it is |
|---|---|
| [HANDOVER.md](HANDOVER.md) | The original project brief and full specification the work started from. Later decisions in ADRs and DESIGN.md supersede it where they differ. |
| [PROGRESS.md](PROGRESS.md) | Milestones, task checklist and a running work log — the resume point for each session. |
| [OPEN_QUESTIONS.md](OPEN_QUESTIONS.md) | Questions and hands-on tasks waiting for the maintainer (listening tests, hardware checks). |
