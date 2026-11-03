#!/usr/bin/env bash
# mise start | stop | status — the dev services of this repo, run in the background
# (app-convention: `mise run start` / `mise run stop`).
#
# "The services" are the dev desktop app (`scripts/unipi-dev.sh`: vite on the
# devUrl port + `cargo tauri dev`, DEV identity dev.unipi.app.dev) and the dev
# agent host that app spawns in its isolated OINO_HOST_DIR.
#
#   start   stop whatever is left of a previous run (our process group, the dev
#           host holding $OINO_HOST_DIR/host.lock, and a process of THIS repo
#           still listening on the dev port), then start fresh in the
#           background. Running it twice leaves exactly one clean set running.
#   stop    stop all of the above. Idempotent.
#   status  print what is running.
#
# Safety: only ever touches processes it can tie to this repo — the pid it
# recorded, the host whose lock lives in the dev host dir, and a port holder
# whose working directory is inside this checkout. A foreign process on the
# dev port is reported, never killed. The installed app (dev.unipi.oino) and
# its host dir (~/.oino/host) are never touched.
#
# Env: OINO_HOST_DIR (default apps/unipi/target/unipi-dev/host),
#      UNIPI_DEV_PORT (default 1420, tauri.conf.json devUrl),
#      UNIPI_DEV_CMD  (default scripts/unipi-dev.sh; tests point it at a stub).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
state="$root/apps/unipi/target/unipi-dev"
export OINO_HOST_DIR="${OINO_HOST_DIR:-$state/host}"
port="${UNIPI_DEV_PORT:-1420}"
cmd="${UNIPI_DEV_CMD:-$root/scripts/unipi-dev.sh}"
pidfile="$state/dev.pid"
log="$state/dev.log"
# The log is truncated on each start and capped (app-convention: bounded files).
log_max_bytes=$((5 * 1024 * 1024))

alive() { [ -n "${1:-}" ] && kill -0 "$1" 2>/dev/null; }

# Wait up to $2 seconds for pid $1 to exit; then SIGKILL it.
reap() {
  local pid="$1" secs="${2:-10}" i
  for ((i = 0; i < secs * 10; i++)); do alive "$pid" || return 0; sleep 0.1; done
  kill -KILL "$pid" 2>/dev/null || true
}

# Same for a whole process group.
reap_group() {
  local pgid="$1" secs="${2:-10}" i
  for ((i = 0; i < secs * 10; i++)); do kill -0 -- "-$pgid" 2>/dev/null || return 0; sleep 0.1; done
  kill -KILL -- "-$pgid" 2>/dev/null || true
}

# pid listening on TCP $port (first one), or empty.
port_pid() {
  ss -ltnpH "sport = :$port" 2>/dev/null | sed -n 's/.*pid=\([0-9]*\).*/\1/p' | head -n1
}

port_busy() { [ -n "$(ss -ltnH "sport = :$port" 2>/dev/null)" ]; }

# Is pid $1's working directory inside this checkout?
ours() {
  local cwd
  cwd="$(readlink "/proc/$1/cwd" 2>/dev/null || true)"
  [ -n "$cwd" ] && case "$cwd/" in "$root"/*) return 0 ;; *) return 1 ;; esac
}

host_pid() {
  local lock="$OINO_HOST_DIR/host.lock"
  [ -f "$lock" ] || return 0
  sed -n 's/.*"pid"[[:space:]]*:[[:space:]]*\([0-9]*\).*/\1/p' "$lock" | head -n1
}

stop() {
  local pid hpid ppid
  if [ -f "$pidfile" ]; then
    pid="$(cat "$pidfile")"
    if alive "$pid"; then
      echo "==> stopping dev app (process group $pid)"
      kill -TERM -- "-$pid" 2>/dev/null || kill -TERM "$pid" 2>/dev/null || true
      reap_group "$pid" 15
    fi
    rm -f "$pidfile"
  fi
  hpid="$(host_pid)"
  # Guard against pid reuse: the lock's pid must still be a unipi host binary.
  if alive "$hpid" && tr '\0' ' ' <"/proc/$hpid/cmdline" 2>/dev/null | grep -q 'unipi'; then
    echo "==> stopping dev host (pid $hpid, $OINO_HOST_DIR)"
    kill -TERM "$hpid" 2>/dev/null || true
    reap "$hpid" 10
  fi
  if port_busy; then
    ppid="$(port_pid)"
    if [ -n "$ppid" ] && ours "$ppid"; then
      echo "==> stopping leftover dev server on :$port (pid $ppid)"
      kill -TERM "$ppid" 2>/dev/null || true
      reap "$ppid" 5
    elif [ -n "$ppid" ]; then
      echo "error: port $port is held by pid $ppid outside this repo; not touching it" >&2
      return 1
    else
      echo "error: port $port is held by a process this user can't see; not touching it" >&2
      return 1
    fi
  fi
}

start() {
  stop
  mkdir -p "$state" "$OINO_HOST_DIR"
  : >"$log"
  echo "==> starting dev app in the background (log: $log)"
  # setsid: own process group, so `stop` takes vite + cargo + the app down together.
  # The log keeps the first $log_max_bytes; the rest is drained, never blocks the app.
  # shellcheck disable=SC2016 # expanded by the inner shell
  setsid bash -c '"$1" 2>&1 | { head -c "$2" >"$3"; cat >/dev/null; }' _ "$cmd" "$log_max_bytes" "$log" </dev/null &
  echo $! >"$pidfile"
  sleep 1
  if ! alive "$(cat "$pidfile")"; then
    echo "error: the dev app exited at once; see $log" >&2
    tail -n 20 "$log" >&2 || true
    rm -f "$pidfile"
    return 1
  fi
  echo "==> running (pid $(cat "$pidfile")); first start compiles the host + app, follow $log"
}

status() {
  local pid hpid ppid
  pid="$(cat "$pidfile" 2>/dev/null || true)"
  if alive "$pid"; then echo "dev app:  running (pid $pid)"; else echo "dev app:  stopped"; fi
  hpid="$(host_pid)"
  if alive "$hpid"; then echo "dev host: running (pid $hpid)"; else echo "dev host: stopped"; fi
  ppid="$(port_pid)"
  if port_busy; then echo "port $port: in use${ppid:+ (pid $ppid)}"; else echo "port $port: free"; fi
}

case "${1:-}" in
  start) start ;;
  stop) stop ;;
  status) status ;;
  *) echo "usage: $0 start|stop|status" >&2; exit 2 ;;
esac
