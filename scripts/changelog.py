#!/usr/bin/env python3
"""Changelog and progress fragments, so parallel PRs don't conflict (docs/dev/RELEASING.md).

A feature PR never edits CHANGELOG.md or docs/PROGRESS.md. It adds its own files instead:
`changelog.d/<branch>.md` for a user-visible change, `docs/progress.d/<branch>.md` for the task
it closes. Only roll-up (`docs/rollup-*`) and release (`chore/release-*`) branches edit the two
shared files.

    scripts/changelog.py check            fragments well-formed; this branch leaves the shared files alone
    scripts/changelog.py preview          what the next release's section will say
    scripts/changelog.py release V DATE   fold changelog.d/ into CHANGELOG.md as `## [V] - DATE`

Fragment format (Keep a Changelog headings, bullets for the user, not commit subjects):

    ### Fixed

    - De-smack: a loud smack no longer raises the bar for the next one.
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
from pathlib import Path

HEADINGS = ("Added", "Changed", "Deprecated", "Removed", "Fixed", "Security")
#: Files only roll-up and release branches may change.
SHARED = ("CHANGELOG.md", "docs/PROGRESS.md")
#: Branch prefixes that may edit them.
SHARED_OK = ("docs/rollup-", "chore/release-")
FRAGMENTS = "changelog.d"
UNRELEASED = "## Unreleased"


def fragment_files(folder: Path) -> list[Path]:
    """Fragment files, sorted (README.md explains the folder and isn't one)."""
    return sorted(p for p in folder.glob("*.md") if p.name != "README.md") if folder.is_dir() else []


def parse(text: str) -> dict[str, list[str]]:
    """{heading: [bullet, …]}; a bullet keeps its wrapped continuation lines. Raises ValueError."""
    sections: dict[str, list[str]] = {}
    current: list[str] | None = None
    for line in text.splitlines():
        if m := re.match(r"^###\s+(.+?)\s*$", line):
            if m[1] not in HEADINGS:
                raise ValueError(f"unknown heading '{m[1]}' (use {', '.join(HEADINGS)})")
            current = sections.setdefault(m[1], [])
        elif line.startswith("- "):
            if current is None:
                raise ValueError("bullet before any ### heading")
            current.append(line)
        elif line.startswith("  ") and current:
            current[-1] += "\n" + line
        elif line.strip():
            raise ValueError(f"unexpected line '{line.strip()[:40]}' (only ### headings and - bullets)")
    if not any(sections.values()):
        raise ValueError("no entries (a ### heading with - bullets)")
    return sections


def check_fragments(folder: Path) -> list[str]:
    errors = []
    for p in fragment_files(folder):
        try:
            parse(p.read_text())
        except ValueError as e:
            errors.append(f"{p.name}: {e}")
    return errors


def read_fragments(folder: Path) -> dict[str, list[str]]:
    merged: dict[str, list[str]] = {}
    for p in fragment_files(folder):
        for heading, bullets in parse(p.read_text()).items():
            merged.setdefault(heading, []).extend(bullets)
    return merged


def render(sections: dict[str, list[str]]) -> str:
    parts = [f"### {h}\n\n" + "\n".join(sections[h]) + "\n" for h in HEADINGS if sections.get(h)]
    return "\n".join(parts)


def release(root: Path, version: str, date: str) -> None:
    """Insert `## [version] - date` with the fragments under *Unreleased*, delete the fragments."""
    folder = root / FRAGMENTS
    if errors := check_fragments(folder):
        raise ValueError("bad fragments: " + "; ".join(errors))
    files = fragment_files(folder)
    if not files:
        raise ValueError(f"no fragments in {FRAGMENTS}/: nothing to release")
    path = root / "CHANGELOG.md"
    text = path.read_text()
    if f"## [{version}]" in text:
        raise ValueError(f"version {version} is already in CHANGELOG.md")
    at = text.find(UNRELEASED)
    nxt = text.find("\n## ", at + len(UNRELEASED))
    if at < 0 or nxt < 0:
        raise ValueError("CHANGELOG.md needs '## Unreleased' followed by a version section")
    section = f"## [{version}] - {date}\n\n{render(read_fragments(folder))}\n"
    path.write_text(text[: nxt + 1] + section + text[nxt + 1 :])
    for p in files:
        p.unlink()


def shared_file_edits(root: Path, branch: str, base: str = "main") -> list[str]:
    """Shared files this branch changed since it left `base` (empty where the rule doesn't apply:
    on `base` itself, on roll-up/release branches, or when `base` isn't available)."""
    if not branch or branch == base or branch.startswith(SHARED_OK):
        return []
    mb = subprocess.run(["git", "-C", str(root), "merge-base", base, "HEAD"], capture_output=True, text=True)
    if mb.returncode != 0:
        return []
    diff = subprocess.run(
        ["git", "-C", str(root), "diff", "--name-only", mb.stdout.strip(), "HEAD", "--", *SHARED],
        capture_output=True,
        text=True,
        check=True,
    )
    return sorted(diff.stdout.split())


def current_branch(root: Path) -> str:
    # GitHub Actions checks out a PR as a detached merge commit; the branch is in GITHUB_HEAD_REF.
    if head := os.environ.get("GITHUB_HEAD_REF"):
        return head
    out = subprocess.run(["git", "-C", str(root), "branch", "--show-current"], capture_output=True, text=True)
    return out.stdout.strip()


def main(argv: list[str]) -> int:
    root = Path(__file__).resolve().parents[1]
    cmd = argv[1] if len(argv) > 1 else "check"
    if cmd == "check":
        errors = check_fragments(root / FRAGMENTS)
        base = "origin/main" if os.environ.get("GITHUB_HEAD_REF") else "main"
        branch = current_branch(root)
        for f in shared_file_edits(root, branch, base):
            errors.append(
                f"{f}: feature branches don't edit it (parallel PRs would conflict). Add "
                f"{FRAGMENTS}/<branch>.md or docs/progress.d/<branch>.md instead — docs/dev/RELEASING.md"
            )
        for e in errors:
            print(f"✘ {e}", file=sys.stderr)
        return 1 if errors else 0
    if cmd == "preview":
        print(render(read_fragments(root / FRAGMENTS)) or "(no fragments)")
        return 0
    if cmd == "release" and len(argv) == 4:
        release(root, argv[2], argv[3])
        print(f"CHANGELOG.md: added [{argv[2]}] - {argv[3]}; {FRAGMENTS}/ emptied")
        return 0
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
