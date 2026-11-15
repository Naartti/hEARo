"""Exit 2 if a shell command points `gh` at any repository but Naartti/hEARo.

Permission rules like `Bash(gh pr comment *)` match the command's prefix, so they can't tell
which repository `--repo`, `-R`, `GH_REPO` or a PR URL selects. Reads the command on stdin;
called by bash-guard.sh. Tested by scripts/test-hooks.sh.
"""

import re
import shlex
import sys

OURS = re.compile(r"^(https?://)?(www\.)?(github\.com/)?naartti/hearo(\.git)?([/#?].*)?$", re.I)
GITHUB_URL = re.compile(r"^(https?://)?(www\.)?github\.com/", re.I)
SEPARATORS = {";", "&", "&&", "|", "||", "(", ")"}
# Flags whose value is free text (a comment body may link anywhere).
TEXT_FLAGS = {"--body", "-b", "--title", "-t", "--message", "-m", "--subject"}


def block(why: str) -> None:
    print(f"Blocked: {why}. gh may only target Naartti/hEARo; ask the human.", file=sys.stderr)
    sys.exit(2)


def segments(tokens: list[str]) -> list[list[str]]:
    out: list[list[str]] = [[]]
    for tok in tokens:
        if tok in SEPARATORS:
            out.append([])
        else:
            out[-1].append(tok)
    return [s for s in out if s]


def check_gh_args(args: list[str]) -> None:
    i = 0
    while i < len(args):
        a = args[i]
        value = None
        if a in ("--repo", "-R"):
            value = args[i + 1] if i + 1 < len(args) else ""
            i += 1
        elif a.startswith("--repo="):
            value = a.removeprefix("--repo=")
        elif a.startswith("-R") and len(a) > 2:
            value = a[2:].removeprefix("=")
        elif a == "--hostname" or a.startswith("--hostname="):
            host = a.partition("=")[2] or (args[i + 1] if i + 1 < len(args) else "")
            if host.lower() != "github.com":
                block(f"gh --hostname {host}")
        elif GITHUB_URL.match(a) and (i == 0 or args[i - 1] not in TEXT_FLAGS):
            value = a
        if value is not None and not OURS.match(value):
            block(f"gh targets {value or '(empty repo)'}")
        i += 1


def main() -> None:
    cmd = sys.stdin.read()
    if not re.search(r"(^|[^\w./-])gh\s", cmd) and "GH_" not in cmd:
        return
    try:
        lexer = shlex.shlex(cmd.replace("\n", ";"), posix=True, punctuation_chars=";&|()")
        lexer.whitespace_split = True
        tokens = list(lexer)
    except ValueError:
        block("couldn't parse a command that runs gh")
    for tok in tokens:
        name, eq, value = tok.partition("=")
        if eq and name == "GH_REPO" and not OURS.match(value):
            block(f"GH_REPO={value}")
        if eq and name in ("GH_HOST", "GH_ENTERPRISE_TOKEN", "GITHUB_ENTERPRISE_TOKEN"):
            if name != "GH_HOST" or value.lower() != "github.com":
                block(f"{name} is set")
    for seg in segments(tokens):
        i = 0
        while i < len(seg) and (seg[i] == "env" or re.match(r"^\w+=", seg[i])):
            i += 1
        if i < len(seg) and seg[i].rsplit("/", 1)[-1] == "gh":
            check_gh_args(seg[i + 1 :])


if __name__ == "__main__":
    main()
