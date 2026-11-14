# Shared helpers for hEARo scripts. Source it: `. "$(dirname "$0")/lib.sh"`.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
unset VIRTUAL_ENV  # a globally active venv (pyenv) would confuse uv

# Use the pinned toolchains from .mise.toml even when the shell isn't mise-activated.
if command -v mise >/dev/null 2>&1; then
  eval "$(mise env -s bash 2>/dev/null)" || true
fi

if [[ -t 1 ]]; then
  BOLD=$'\e[1m'; RED=$'\e[31m'; GREEN=$'\e[32m'; YELLOW=$'\e[33m'; DIM=$'\e[2m'; RESET=$'\e[0m'
else
  BOLD=""; RED=""; GREEN=""; YELLOW=""; DIM=""; RESET=""
fi

FAILED_STEPS=()
WARNINGS=()

# step "name" cmd args... — run a step, time it, record failure but keep going.
step() {
  local name="$1"; shift
  local start=$SECONDS
  printf '%s▶ %s%s\n' "$BOLD" "$name" "$RESET"
  if "$@"; then
    printf '%s✔ %s%s %s(%ss)%s\n' "$GREEN" "$name" "$RESET" "$DIM" "$((SECONDS - start))" "$RESET"
  else
    printf '%s✘ %s%s\n' "$RED" "$name" "$RESET"
    FAILED_STEPS+=("$name")
  fi
}

warn() {
  printf '%s⚠ %s%s\n' "$YELLOW" "$*" "$RESET"
  WARNINGS+=("$*")
}

# finish "gate name" — print summary, exit non-zero if any step failed.
finish() {
  echo
  for w in "${WARNINGS[@]+"${WARNINGS[@]}"}"; do printf '%s⚠ %s%s\n' "$YELLOW" "$w" "$RESET"; done
  if ((${#FAILED_STEPS[@]})); then
    printf '%s%s FAILED:%s %s\n' "$RED$BOLD" "$1" "$RESET" "${FAILED_STEPS[*]}"
    exit 1
  fi
  printf '%s%s passed%s in %ss\n' "$GREEN$BOLD" "$1" "$RESET" "$SECONDS"
}

# Xcode's developer dir if Xcode is installed (even if xcode-select still points at CLT).
xcode_dev_dir() {
  local sel; sel="$(xcode-select -p 2>/dev/null || true)"
  if [[ "$sel" == *Xcode*.app* ]]; then echo "$sel"; return 0; fi
  for app in /Applications/Xcode.app /Applications/Xcode-beta.app; do
    if [[ -d "$app/Contents/Developer" ]]; then echo "$app/Contents/Developer"; return 0; fi
  done
  return 1
}

# load_sign_identity — set SIGN_IDENTITY for sign.sh / package-dmg.sh. A value already in the
# environment wins over .env.local: public releases in GitHub Actions pass
# `ALLOW_ADHOC=1 SIGN_IDENTITY=-` so no personal certificate ends up in them (ADR 0016).
load_sign_identity() {
  if [[ -n "${SIGN_IDENTITY:-}" ]]; then
    echo "${DIM}signing identity from the environment (not .env.local)${RESET}"
  elif [[ -f .env.local ]]; then
    set -a && . ./.env.local && set +a
  fi
}

# Fingerprint of the tree being gated, taken when the gate starts (see tree-fingerprint.sh).
GATE_TREE="$("$ROOT/scripts/tree-fingerprint.sh")"

# stamp_green — after `finish` passed: record that this exact tree passed the gate.
stamp_green() {
  mkdir -p "$ROOT/target/ci"
  echo "$GATE_TREE" >"$ROOT/target/ci/check.ok"
}
