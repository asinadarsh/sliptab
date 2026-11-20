#!/usr/bin/env bash
# Dev helper: (re)start the desktop app in the background with the edge probe on,
# wait for the probe line, and print where X11 really put the window.
# Usage: desktop/scripts/dev-probe.sh [stop]
set -u
LOG=/tmp/edge-dev.log
stop() { pkill -f 'target/debug/desktop$' ; pkill -f 'node .*vite dev' ; pkill -f 'node .*tauri dev' ; sleep 1; }
stop
[ "${1:-}" = stop ] && exit 0
[ -f "$HOME/.local/share/edge-sysroot/env.sh" ] && . "$HOME/.local/share/edge-sysroot/env.sh"
cd "$(dirname "$0")/.."
: > "$LOG"
EDGE_PROBE=1 setsid npm run tauri dev > "$LOG" 2>&1 < /dev/null &
for _ in $(seq 1 120); do grep -qE 'EDGE_PROBE|^error' "$LOG" && break; sleep 2; done
grep -E 'EDGE_PROBE|^error|panicked' "$LOG" | tail -3
id=$(xdotool search --name '^Sliptab$' 2>/dev/null | head -1)
[ -n "$id" ] && xwininfo -id "$id" | grep -E 'Absolute|Width|Height'
