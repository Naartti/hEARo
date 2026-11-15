"""Turn an auto-allowed `gh` command back into a permission prompt when it may leave this repo.

Permission rules like `Bash(gh pr comment *)` match the command's prefix, so they can't tell
which repository `--repo`, `-R`, `GH_REPO`, a URL, a `gh api` path or the working directory
selects. This scans the command text with quotes and line continuations removed (no shell
parsing, so nothing can fail to parse) and, when it mentions `gh` and anything points
elsewhere, prints a PreToolUse "ask" decision. False positives cost one prompt. Also asks for
shell expansions, directory changes, mutating `gh api` calls, and checking out a PR that isn't
Dependabot's or the owner's (untrusted fork code would run in `just ci`).

Usage: gh_scope.py <cwd> <project dir>, command on stdin. Called by bash-guard.sh; tested by
scripts/test-hooks.sh.
"""

import json
import os
import re
import subprocess
import sys

REPO = "naartti/hearo"
TRUSTED_AUTHORS = {"app/dependabot", "dependabot[bot]", "dependabot", "naartti"}

GH_WORD = re.compile(r"(?:^|[^\w.-])gh(?:$|[^\w.-])")
STOP = r"""\s"'`;&|()<>"""
REPO_FLAG = re.compile(rf"(?:^|[\s;&|(])(?:--repo|-R)(?:=|\s+|(?=[^\s=-]))[\"']?([^{STOP}]*)")
GH_REPO = re.compile(rf"\bGH_REPO=[\"']?([^{STOP}]*)")
HOST = re.compile(r"--hostname|\bGH_HOST\b|\bGH_ENTERPRISE_TOKEN\b|\bGITHUB_ENTERPRISE_TOKEN\b")
URL = re.compile(r"github\.com[/:]([\w.-]+)/([\w.-]+)", re.I)
GH_API = re.compile(r"(?:^|[^\w.-])gh\s+api\b")
API_PATH = re.compile(r"(?:^|[\s\"'/])repos/([\w.-]+)/([\w.-]+)")
API_METHOD = re.compile(r"(?:^|\s)(?:-X|--method)(?:=|\s*)[\"']?(\w+)")
API_WRITE = re.compile(r"(?:^|\s)(?:-[fF]|--field|--raw-field|--input)|\bgraphql\b")
EXPANSION = re.compile(r"[$`]")
CHDIR = re.compile(r"(?:^|[^\w.-])(?:cd|pushd)(?:\s|$)|\s-C\s|--git-dir|--work-tree|\bGIT_DIR\b|\bGIT_WORK_TREE\b")
CHECKOUT = re.compile(rf"(?:^|[^\w.-])gh\s+pr\s+checkout\b([^;&|\n]*)")


def same_repo(owner: str, name: str) -> bool:
    return f"{owner}/{name.removesuffix('.git')}".lower() == REPO


def repo_value_ok(value: str) -> bool:
    v = re.sub(r"^(https?://)?(www\.)?(github\.com/)?", "", value, flags=re.I)
    return v.lower().removesuffix("/").removesuffix(".git") == REPO


def pr_author(pr: str) -> str:
    try:
        out = subprocess.run(
            ["gh", "pr", "view", pr, "--repo", "Naartti/hEARo", "--json", "author", "--jq", ".author.login"],
            capture_output=True,
            text=True,
            timeout=10,
            check=True,
        )
        return out.stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return ""


def inside(cwd: str, project: str) -> bool:
    cwd, project = os.path.realpath(cwd or "."), os.path.realpath(project or ".")
    return cwd == project or cwd.startswith(project + os.sep)


def reasons(raw: str, cwd: str, project: str) -> list[str]:
    # Quotes and backslashes only regroup words; dropping them defeats `--re''po`-style tricks.
    cmd = re.sub(r"\\\n", " ", raw).replace("\\", "").replace("'", "").replace('"', "")
    if not GH_WORD.search(cmd):
        return []
    found = []
    if not inside(cwd, project):
        found.append(f"working directory {cwd} is outside the project")
    if CHDIR.search(cmd):
        found.append("changes directory (gh picks the repo from the git remote there)")
    if EXPANSION.search(cmd):
        found.append("shell expansion ($ or `) can't be checked")
    found += [f"--repo/-R {v or '(empty)'}" for v in REPO_FLAG.findall(cmd) if not repo_value_ok(v)]
    found += [f"GH_REPO={v}" for v in GH_REPO.findall(cmd) if not repo_value_ok(v)]
    if HOST.search(cmd):
        found.append("another GitHub host")
    found += [f"URL to {o}/{n}" for o, n in URL.findall(cmd) if not same_repo(o, n)]
    if GH_API.search(cmd):
        found += [f"gh api repos/{o}/{n}" for o, n in API_PATH.findall(cmd) if not same_repo(o, n)]
        found += [f"gh api -X {m}" for m in API_METHOD.findall(cmd) if m.upper() != "GET"]
        if API_WRITE.search(cmd):
            found.append("gh api with a request body or GraphQL")
    for args in CHECKOUT.findall(cmd):
        target = next((a for a in args.split() if not a.startswith("-")), "")
        if not target.isdigit():
            found.append(f"gh pr checkout {target or '(current branch)'}")
        elif (author := pr_author(target)).lower() not in TRUSTED_AUTHORS:
            found.append(f"checkout of PR #{target} by {author or 'unknown author'}")
    return found


def main() -> None:
    cwd = sys.argv[1] if len(sys.argv) > 1 else ""
    project = sys.argv[2] if len(sys.argv) > 2 else ""
    found = reasons(sys.stdin.read(), cwd, project)
    if found:
        why = "; ".join(dict.fromkeys(found))
        print(
            json.dumps(
                {
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "ask",
                        "permissionDecisionReason": f"gh may reach beyond Naartti/hEARo: {why}",
                    }
                }
            )
        )


if __name__ == "__main__":
    main()
