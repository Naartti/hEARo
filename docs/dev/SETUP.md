# Developer setup

```sh
brew install mise          # if missing
just bootstrap             # or, without just yet: scripts/bootstrap.sh
just doctor                # verifies everything and prints fixes
```

`bootstrap` installs the pinned toolchains from `.mise.toml`, the Brewfile tools, the Python
venv (`ml/.venv` via uv), and the git hooks (lefthook), then runs `doctor`.

Xcode is optional for most work (ADR 0004): with Command Line Tools only you can build
everything, but Swift tests, previews and Instruments need Xcode. Signing: [SIGNING.md](SIGNING.md).

`just` recipes and git hooks run through `mise x`, so they work even if your shell doesn't
activate mise. To use `cargo`, `uv` etc. directly, add `eval "$(mise activate zsh)"` to
`~/.zshrc`.

Daily loop: edit → `just check` → commit. Before pushing / at milestone end: `just ci`.
Details: [LOCAL_CI.md](LOCAL_CI.md).
