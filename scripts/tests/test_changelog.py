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
    subprocess.run(
        ["git", "-C", str(root), "-c", "user.name=t", "-c", "user.email=t@e", *args], check=True, capture_output=True
    )


def git_repo(tmp_path: Path) -> Path:
    """A repo with CHANGELOG.md and docs/PROGRESS.md on main, and a feature branch feat/x."""
    root = repo(tmp_path)
    (root / "docs").mkdir()
    (root / "docs" / "PROGRESS.md").write_text("# Progress\n")
    git(root, "init", "-q", "-b", "main")
    git(root, "add", "-A")
    git(root, "commit", "-qm", "base")
    git(root, "switch", "-q", "-c", "feat/x")
    (root / "changelog.d" / "feat-x.md").write_text("### Added\n\n- X.\n")
    git(root, "add", "-A")
    git(root, "commit", "-qm", "x")
    return root


def test_feature_branches_may_not_edit_the_shared_files(tmp_path: Path) -> None:
    root = git_repo(tmp_path)
    assert cl.check(root, {}) == []
    # Uncommitted edits count too: a green check must not let the commit hook through.
    (root / "CHANGELOG.md").write_text(CHANGELOG + "- sneaky\n")
    assert [e.split(":")[0] for e in cl.check(root, {})] == ["CHANGELOG.md"]
    (root / "docs" / "PROGRESS.md").write_text("# Progress\n- [x] done\n")
    git(root, "commit", "-qam", "y")
    assert [e.split(":")[0] for e in cl.check(root, {})] == ["CHANGELOG.md", "docs/PROGRESS.md"]
    # Roll-up and release branches are where the shared files change; main has nothing to judge.
    for ok in ("docs/rollup-2026-11", "chore/release-v0.3.0"):
        git(root, "branch", ok)
        git(root, "switch", "-q", ok)
        assert cl.check(root, {}) == [], ok
    git(root, "switch", "-q", "main")
    assert cl.check(root, {}) == []


def test_local_runs_compare_with_origin_main_when_there_is_one(tmp_path: Path) -> None:
    root = git_repo(tmp_path)
    # origin/main moved on (a roll-up edited CHANGELOG.md) and the branch was rebased onto it,
    # but local main is stale: the roll-up's edit is not this branch's.
    git(root, "switch", "-q", "main")
    (root / "CHANGELOG.md").write_text(CHANGELOG + "rolled up\n")
    git(root, "commit", "-qam", "rollup")
    git(root, "update-ref", "refs/remotes/origin/main", "HEAD")
    git(root, "reset", "-q", "--hard", "HEAD~1")
    git(root, "switch", "-q", "feat/x")
    git(root, "rebase", "-q", "origin/main")
    assert cl.check(root, {}) == []


def test_ci_compares_the_pr_merge_commit_with_its_first_parent(tmp_path: Path) -> None:
    root = git_repo(tmp_path)
    (root / "CHANGELOG.md").write_text(CHANGELOG + "- sneaky\n")
    git(root, "commit", "-qam", "sneaky")
    git(root, "switch", "-q", "--detach", "main")
    git(root, "merge", "-q", "--no-ff", "feat/x", "-m", "merge")  # what actions/checkout gives a PR
    env = {"GITHUB_HEAD_REF": "feat/x", "GITHUB_BASE_REF": "main"}
    assert [e.split(":")[0] for e in cl.check(root, env)] == ["CHANGELOG.md"]
    assert cl.check(root, {**env, "GITHUB_HEAD_REF": "docs/rollup-x"}) == []


def test_ci_without_the_parent_commit_fails_loudly(tmp_path: Path) -> None:
    root = repo(tmp_path)
    git(root, "init", "-q", "-b", "main")
    git(root, "add", "-A")
    git(root, "commit", "-qm", "only")  # a depth-1 checkout: no HEAD^1
    with pytest.raises(ValueError, match="fetch-depth"):
        cl.check(root, {"GITHUB_HEAD_REF": "feat/x", "GITHUB_BASE_REF": "main"})


def test_release_validates_version_and_date(tmp_path: Path) -> None:
    root = repo(tmp_path)
    (root / "changelog.d" / "x.md").write_text("### Fixed\n\n- x\n")
    with pytest.raises(ValueError, match="no 'v'"):
        cl.release(root, "v0.3.0", "2026-11-20")
    with pytest.raises(ValueError, match="YYYY-MM-DD"):
        cl.release(root, "0.3.0", "20-11-2026")
    cl.release(root, "0.3.0-rc.1", "2026-11-20")
    assert "## [0.3.0-rc.1] - 2026-11-20" in (root / "CHANGELOG.md").read_text()


def test_bullets_keep_paragraphs_and_tab_indents() -> None:
    parsed = cl.parse("### Fixed\n\n- First line\n\n  Second paragraph.\n- Next\n\tindented by a tab\n")
    assert parsed == {"Fixed": ["- First line\n\n  Second paragraph.", "- Next\n    indented by a tab"]}


def test_main_reports_problems_and_previews(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    assert cl.main(["changelog.py", "preview"]) == 0
    assert cl.main(["changelog.py", "nonsense"]) == 2
    assert cl.main(["changelog.py", "release", "v1", "x"]) == 1
    assert "no 'v'" in capsys.readouterr().err
