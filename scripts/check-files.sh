#!/usr/bin/env bash
# Repo hygiene: no files > 5 MB, no audio/model files outside allowed paths, no secrets files.
# Default: staged files. `--all`: every tracked file.
. "$(dirname "$0")/lib.sh"
if [[ "${1:-}" == "--all" ]]; then
  files="$(git ls-files)"
else
  files="$(git diff --cached --name-only --diff-filter=ACM 2>/dev/null || true)"
fi
bad=0
while IFS= read -r f; do
  [[ -z "$f" || ! -f "$f" ]] && continue
  size=$(stat -f%z "$f")
  if (( size > 5 * 1024 * 1024 )); then echo "✘ $f is $((size / 1024 / 1024)) MB (limit 5 MB)"; bad=1; fi
  case "$f" in
    testdata/*|models/*) ;;
    *.wav|*.flac|*.mp3|*.m4a|*.aac|*.opus|*.onnx|*.pt|*.pth|*.ckpt)
      echo "✘ $f: audio/model files belong in testdata/ or models/ only"; bad=1 ;;
  esac
  case "$f" in
    .env.example) ;;
    .env|.env.*|*.p12|*.cer|*.mobileprovision|*.pem) echo "✘ $f: secrets/certificates must never be committed"; bad=1 ;;
  esac
  case "$f" in
    .github/workflows/*) echo "✘ $f: no hosted CI in this project (ADR 0009)"; bad=1 ;;
  esac
done <<< "$files"
exit $bad
