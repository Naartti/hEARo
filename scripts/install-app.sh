#!/usr/bin/env bash
# Install a built hEARo.app as THE copy on this Mac: ~/Applications/hEARo.app. Debug (`just app`)
# and release (`just install`) builds share one name, bundle id and path, so there is never a
# second engine tapping the first one's output, and permission + login item stick
# (docs/dev/SIGNING.md). Quits a running hEARo first and reopens it if it was running.
# Usage: scripts/install-app.sh path/to/hEARo.app [--open]
. "$(dirname "$0")/lib.sh"
src="${1:?usage: install-app.sh hEARo.app [--open]}"
open_after="${2:-}"
name="hEARo"
dest="$HOME/Applications/$name.app"

was_running=0
if pgrep -x "$name" >/dev/null; then
  was_running=1
  pkill -x "$name" || true
  for _ in $(seq 50); do
    pgrep -x "$name" >/dev/null || break
    sleep 0.1
  done
fi
mkdir -p "$(dirname "$dest")"
rm -rf "$dest"
ditto "$src" "$dest"
echo "✔ installed $dest"

if [[ -d "/Applications/$name.app" ]]; then
  warn "/Applications/$name.app also exists (same bundle id): delete it and keep one copy (docs/dev/SIGNING.md)"
fi
if [[ -d "$HOME/Applications/hEARo Dev.app" ]]; then
  warn "old dev build ~/Applications/hEARo Dev.app found: quit it and delete it (docs/dev/SIGNING.md)"
fi
if [[ "$was_running" == 1 || "$open_after" == --open ]]; then
  open "$dest"
fi
