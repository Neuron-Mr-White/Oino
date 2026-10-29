#!/usr/bin/env bash
# Oino Desktop launcher for `mise run start|stop|desktop` (app-convention: `start` restarts
# cleanly when run twice, `stop` stops everything it started).
#
#   scripts/desktop.sh start [--demo] [--cwd DIR]   build (release) and start in the background
#   scripts/desktop.sh run   [--demo] [--cwd DIR]   build and run in the foreground
#   scripts/desktop.sh stop                          stop the instance started by this script
#   scripts/desktop.sh status
#
# Env: OINO_DESKTOP_AGENT_DIR (default ~/.oino-desktop-test, seeded once with ~/.oino/auth.json
# so your real ~/.oino is untouched; set it to ~/.oino to use your real config),
# OINO_DESKTOP_CWD (default: the current directory).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
state="$root/target/desktop"
pidfile="$state/oino-desktop.pid"
logfile="$state/oino-desktop.log"
bin="$root/apps/target/release/oino-desktop"
agent_dir="${OINO_DESKTOP_AGENT_DIR:-$HOME/.oino-desktop-test}"
mkdir -p "$state"

cmd="${1:-run}"; shift || true

is_running() {
  [ -f "$pidfile" ] && kill -0 "$(cat "$pidfile")" 2>/dev/null
}

stop() {
  if is_running; then
    local pid; pid="$(cat "$pidfile")"
    kill "$pid" 2>/dev/null || true
    for _ in $(seq 1 50); do kill -0 "$pid" 2>/dev/null || break; sleep 0.1; done
    kill -9 "$pid" 2>/dev/null || true
    echo "oino-desktop: stopped (pid $pid)"
  else
    echo "oino-desktop: not running"
  fi
  rm -f "$pidfile"
}

build() {
  (cd "$root/apps" && cargo build --release -p oino-desktop)
}

# oino-desktop is a GPUI (Wayland/X11) app: launched from a shell that lost the desktop
# session's env (agent terminals, old tmux panes, SSH), it starts but can never open a
# window. Require a display target instead of failing silently.
check_display_env() {
  if [ -z "${WAYLAND_DISPLAY:-}" ] && [ -z "${DISPLAY:-}" ]; then
    echo "oino-desktop: neither WAYLAND_DISPLAY nor DISPLAY is set — the window would never appear." >&2
    echo "  Run from a terminal inside the desktop session, or e.g." >&2
    echo "  WAYLAND_DISPLAY=wayland-0 XDG_RUNTIME_DIR=/run/user/\$(id -u) mise run desktop" >&2
    exit 1
  fi
}

seed_agent_dir() {
  if [ ! -d "$agent_dir" ]; then
    mkdir -p "$agent_dir"
    if [ -f "$HOME/.oino/auth.json" ]; then
      install -m 600 "$HOME/.oino/auth.json" "$agent_dir/auth.json"
    fi
    echo "oino-desktop: new test config at $agent_dir (auth.json copied from ~/.oino)"
  fi
}

args=()
cwd="${OINO_DESKTOP_CWD:-$PWD}"
while [ $# -gt 0 ]; do
  case "$1" in
    --demo) args+=(--demo); shift ;;
    --cwd) cwd="$2"; shift 2 ;;
    *) args+=("$1"); shift ;;
  esac
done
args+=(--cwd "$cwd")

case "$cmd" in
  start)
    stop >/dev/null
    check_display_env
    build
    seed_agent_dir
    OINO_AGENT_DIR="$agent_dir" nohup "$bin" "${args[@]}" >"$logfile" 2>&1 &
    echo $! >"$pidfile"
    sleep 1
    if is_running; then
      echo "oino-desktop: started (pid $(cat "$pidfile")), config $agent_dir, log $logfile"
    else
      echo "oino-desktop: exited at start; last log lines:" >&2
      tail -20 "$logfile" >&2
      exit 1
    fi
    ;;
  run)
    check_display_env
    build
    seed_agent_dir
    OINO_AGENT_DIR="$agent_dir" exec "$bin" "${args[@]}"
    ;;
  stop) stop ;;
  status)
    if is_running; then echo "oino-desktop: running (pid $(cat "$pidfile"))"; else echo "oino-desktop: not running"; fi
    ;;
  *) echo "usage: $0 start|run|stop|status [--demo] [--cwd DIR]" >&2; exit 2 ;;
esac
