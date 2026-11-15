#!/usr/bin/env python3
"""Patch coverage of the branch against main, like Codecov's `codecov/patch` check.

Reads target/coverage/lcov.info (from `just coverage`) and the `git diff` against the merge base
with main; prints covered/instrumented added lines per Rust file plus the uncovered line numbers,
and fails when the patch is below the project's line coverage minus THRESHOLD (codecov.yml:
`patch: target auto, threshold 5%`).
"""

import re
import subprocess
import sys
from pathlib import Path

THRESHOLD = 5.0
LCOV = Path("target/coverage/lcov.info")


def lcov() -> dict[str, dict[int, int]]:
    cov: dict[str, dict[int, int]] = {}
    cur = ""
    for line in LCOV.read_text().splitlines():
        if line.startswith("SF:"):
            cur = line[3:]
        elif line.startswith("DA:"):
            n, hits = line[3:].split(",")[:2]
            cov.setdefault(cur, {})[int(n)] = int(hits)
    return cov


def added_lines(base: str, path: str) -> set[int]:
    diff = subprocess.run(["git", "diff", "-U0", base, "--", path], capture_output=True, text=True).stdout
    out: set[int] = set()
    for m in re.finditer(r"^@@ .* \+(\d+)(?:,(\d+))? @@", diff, re.M):
        start, count = int(m[1]), int(m[2] or 1)
        out.update(range(start, start + count))
    return out


def main() -> int:
    if not LCOV.exists():
        print("no target/coverage/lcov.info: run `just coverage` first", file=sys.stderr)
        return 2
    base = subprocess.run(["git", "merge-base", "main", "HEAD"], capture_output=True, text=True).stdout.strip()
    files = subprocess.run(["git", "diff", "--name-only", base, "--", "*.rs"], capture_output=True, text=True)
    cov = lcov()
    hit = total = 0
    for f in files.stdout.split():
        lines = next((v for k, v in cov.items() if k.endswith("/" + f)), {})
        inst = sorted(n for n in added_lines(base, f) if n in lines)
        if not inst:
            continue
        missed = [n for n in inst if lines[n] == 0]
        hit += len(inst) - len(missed)
        total += len(inst)
        print(f"{f}: {len(inst) - len(missed)}/{len(inst)}" + (f"  uncovered: {missed}" if missed else ""))
    proj = sum(c > 0 for v in cov.values() for c in v.values()) / max(1, sum(len(v) for v in cov.values())) * 100
    if total == 0:
        print("no instrumented Rust lines changed")
        return 0
    patch = 100 * hit / total
    need = proj - THRESHOLD
    print(f"patch {patch:.1f} % ({hit}/{total}); project {proj:.1f} % → need ≥ {need:.1f} %")
    if patch < need:
        print("✘ patch coverage below Codecov's bar: test the uncovered lines above", file=sys.stderr)
        return 1
    print("✔ patch coverage")
    return 0


if __name__ == "__main__":
    sys.exit(main())
