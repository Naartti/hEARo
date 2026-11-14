---
name: write-adr
description: Template and rules for hEARo Architecture Decision Records in docs/adr/. Use for any non-trivial design choice (dependencies, threading, formats, tooling, deviations from the brief).
---

# Writing an ADR

- File: `docs/adr/NNNN-kebab-title.md`, next free number (`ls docs/adr`).
- Copy `docs/adr/0000-template.md`. Keep it under a page.
- **Context**: the forces, constraints and facts (with versions you verified).
- **Decision**: one paragraph, active voice ("We use X").
- **Alternatives considered**: each with why not.
- **Consequences**: what gets easier/harder, and *how we'd know it was wrong* (a measurable signal).
- Status `accepted` when committed with the code; superseding an ADR = new ADR + update the old
  one's status line to `superseded by NNNN`.
- Add it to the index in docs/adr/README.md. Commit as `docs(adr): NNNN title` together with
  the change it describes.
