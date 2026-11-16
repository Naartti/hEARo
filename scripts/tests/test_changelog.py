"""scripts/changelog.py: per-PR changelog/progress fragments instead of shared-file edits."""

from __future__ import annotations

import importlib.util
import subprocess
from pathlib import Path

import pytest

spec = importlib.util.spec_from_file_location("changelog", Path(__file__).parents[1] / "changelog.py")
assert spec and spec.loader
cl = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cl)

CHANGELOG = """# Changelog

Intro text.

## Unreleased

Pending entries live in `changelog.d/`.

## [0.2.0] - 2026-11-15

### Added

- Old thing.
"""


def repo(tmp_path: Path) -> Path:
    (tmp_path / "changelog.d").mkdir()
    (tmp_path / "CHANGELOG.md").write_text(CHANGELOG)
    return tmp_path


def test_valid_fragments_pass_and_bad_ones_are_named(tmp_path: Path) -> None:
    root = repo(tmp_path)
    (root / "changelog.d" / "fix-a.md").write_text("### Fixed\n\n- A thing works again.\n")
    (root / "changelog.d" / "README.md").write_text("# not a fragment\n")
    assert cl.check_fragments(root / "changelog.d") == []
    (root / "changelog.d" / "bad-heading.md").write_text("### Improved\n\n- Nope.\n")
    (root / "changelog.d" / "no-bullet.md").write_text("### Added\n\nJust prose.\n")
    (root / "changelog.d" / "empty.md").write_text("\n")
    errors = cl.check_fragments(root / "changelog.d")
    assert len(errors) == 3
    assert any("bad-heading.md" in e and "Improved" in e for e in errors)
    assert any("no-bullet.md" in e for e in errors)
    assert any("empty.md" in e for e in errors)


def test_preview_groups_by_heading_in_keep_a_changelog_order(tmp_path: Path) -> None:
    root = repo(tmp_path)
    (root / "changelog.d" / "b-fix.md").write_text("### Fixed\n\n- Second fix.\n")
    (root / "changelog.d" / "a-feat.md").write_text("### Fixed\n\n- First fix.\n\n### Added\n\n- New\n  wrapped.\n")
    text = cl.render(cl.read_fragments(root / "changelog.d"))
    assert text == "### Added\n\n- New\n  wrapped.\n\n### Fixed\n\n- First fix.\n- Second fix.\n"


def test_release_folds_fragments_into_a_version_section(tmp_path: Path) -> None:
    root = repo(tmp_path)
    (root / "changelog.d" / "x.md").write_text("### Changed\n\n- Bundle id.\n")
    (root / "changelog.d" / "README.md").write_text("# keep me\n")
    cl.release(root, "0.3.0", "2026-11-20")
    text = (root / "CHANGELOG.md").read_text()
    expected = (
        "## Unreleased\n\nPending entries live in `changelog.d/`.\n\n"
        "## [0.3.0] - 2026-11-20\n\n### Changed\n\n- Bundle id.\n\n## [0.2.0]"
    )
    assert expected in text
    assert not (root / "changelog.d" / "x.md").exists()
    assert (root / "changelog.d" / "README.md").exists()
    with pytest.raises(ValueError, match="no fragments"):
        cl.release(root, "0.3.1", "2026-11-21")


def test_release_refuses_bad_fragments_and_a_duplicate_version(tmp_path: Path) -> None:
    root = repo(tmp_path)
    (root / "changelog.d" / "bad.md").write_text("### Nope\n\n- x\n")
    with pytest.raises(ValueError, match=r"bad\.md"):
        cl.release(root, "0.3.0", "2026-11-20")
    (root / "changelog.d" / "bad.md").write_text("### Fixed\n\n- x\n")
    with pytest.raises(ValueError, match="already"):
        cl.release(root, "0.2.0", "2026-11-20")


def git(root: Path, *args: str) -> None:
    subprocess.run(["git", "-C", str(root), *args], check=True, capture_output=True)


def test_feature_branches_may_not_edit_the_shared_files(tmp_path: Path) -> None:
    root = repo(tmp_path)
    (root / "docs").mkdir()
    (root / "docs" / "PROGRESS.md").write_text("# Progress\n")
    git(root, "init", "-q", "-b", "main")
    git(root, "-c", "user.name=t", "-c", "user.email=t@e", "commit", "-qam", "init", "--allow-empty")
    git(root, "add", "-A")
    git(root, "-c", "user.name=t", "-c", "user.email=t@e", "commit", "-qm", "base")
    git(root, "switch", "-q", "-c", "feat/x")
    (root / "changelog.d" / "feat-x.md").write_text("### Added\n\n- X.\n")
    (root / "src.txt").write_text("code\n")
    git(root, "add", "-A")
    git(root, "-c", "user.name=t", "-c", "user.email=t@e", "commit", "-qm", "x")
    assert cl.shared_file_edits(root, "feat/x", "main") == []
    (root / "CHANGELOG.md").write_text(CHANGELOG + "- sneaky\n")
    (root / "docs" / "PROGRESS.md").write_text("# Progress\n- [x] done\n")
    git(root, "-c", "user.name=t", "-c", "user.email=t@e", "commit", "-qam", "y")
    assert cl.shared_file_edits(root, "feat/x", "main") == ["CHANGELOG.md", "docs/PROGRESS.md"]
    # Roll-up and release branches are where the shared files change.
    assert cl.shared_file_edits(root, "docs/rollup-2026-11", "main") == []
    assert cl.shared_file_edits(root, "chore/release-v0.3.0", "main") == []
    # On main, or without a main to compare with (shallow CI checkout), there's nothing to judge.
    assert cl.shared_file_edits(root, "main", "main") == []
    assert cl.shared_file_edits(root, "feat/x", "no-such-ref") == []
