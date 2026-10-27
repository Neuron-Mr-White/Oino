#!/usr/bin/env bash
# Cron end-to-end verification (round 3). Everything runs in scratch dirs under
# target/tmp: its own HOME, OINO_AGENT_DIR and XDG_CONFIG_HOME, so no user service, session
# or job of the owner is touched (docs/cron.md).
#
#   scripts/round3/cron-e2e.sh [--real] [--skip-service]
#
# Covers: a faux-provider job firing every minute (runs recorded, session files written),
# a one-time `at:` job, pause/resume, the missed-run policy after the daemon was stopped,
# `oino cron install`/`uninstall` in the scratch HOME (systemd --user is usually absent in a
# headless shell — the unit file is what gets verified) and, with --real, one short run on
# oino/deepseek/deepseek-flash (needs ~/.oino/auth.json, copied into the scratch agent dir).
#
# Needs the binary built with the faux provider: `cargo build -p oino --features faux`.
set -uo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
bin="${OINO_BIN:-$root/target/debug/oino}"
work="${CRON_E2E_DIR:-$root/target/tmp/cron-e2e}"
real=0
service=1
for arg in "$@"; do
  case "$arg" in
    --real) real=1 ;;
    --skip-service) service=0 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

[ -x "$bin" ] || { echo "no binary at $bin" >&2; exit 2; }

fail=0
pass() { printf 'PASS  %s\n' "$*"; }
bad() { printf 'FAIL  %s\n' "$*"; fail=$((fail + 1)); }

rm -rf "$work"
mkdir -p "$work/home" "$work/agent" "$work/jobs"
export HOME="$work/home"
export XDG_CONFIG_HOME="$work/home/.config"
export OINO_AGENT_DIR="$work/agent"
export OINO_BUNDLED=0
export OINO_FAUX_API_KEY=faux-key
export OINO_FAUX_RESPONSES='[{"text":"faux: cron run finished"}]'
work="$(cd "$work" && pwd)"

# The scheduler spawns plain `oino`; point it at a wrapper that loads the faux provider.
wrapper="$work/oino-faux"
printf '#!/bin/sh\nexec %q -e builtin:faux -nc -ns -np "$@"\n' "$bin" >"$wrapper"
chmod +x "$wrapper"
export OINO_CRON_BIN="$wrapper"

cron() { "$bin" cron "$@"; }
json() { "$bin" cron "$@" --json; }
daemon=""
cleanup() {
  if [ -n "$daemon" ] && kill -0 "$daemon" 2>/dev/null; then
    kill "$daemon"
    wait "$daemon" 2>/dev/null
  fi
}
trap cleanup EXIT

start_daemon() {
  "$bin" cron serve >>"$work/serve.log" 2>&1 &
  daemon=$!
  for _ in $(seq 1 50); do
    [ -f "$OINO_AGENT_DIR/cron/serve.json" ] && return 0
    sleep 0.2
  done
  echo "daemon did not come up (see $work/serve.log)" >&2
  return 1
}
stop_daemon() {
  [ -n "$daemon" ] || return 0
  kill "$daemon" 2>/dev/null
  wait "$daemon" 2>/dev/null
  daemon=""
}
runs_of() { json runs "$1" | jq -r '.runs | length'; }
wait_finished() { # job, seconds — the newest run is no longer running/queued
  local id="$1" secs="$2" i st
  for i in $(seq 1 "$secs"); do
    st="$(json runs "$id" | jq -r '.runs[0].status // "none"')"
    case "$st" in
      running | queued | none) sleep 1 ;;
      *) return 0 ;;
    esac
  done
  return 1
}
wait_runs() { # job, minimum count, seconds
  local id="$1" want="$2" secs="$3" i
  for i in $(seq 1 "$secs"); do
    [ "$(runs_of "$id")" -ge "$want" ] && return 0
    sleep 1
  done
  return 1
}

echo "== work dir $work"
echo "== binary $bin"

# ---------------------------------------------------------------- 1. daemon + every minute
start_daemon || exit 1
cron add --name "minute heartbeat" --prompt "Say OK and stop." \
  --schedule "0 * * * * *" --cwd "$work/jobs" --overlap skip >/dev/null ||
  { bad "add minute job"; exit 1; }
minute_id="$(json list | jq -r '.jobs[] | select(.name == "minute heartbeat") | .id')"
echo "== minute job $minute_id"

if wait_runs "$minute_id" 3 200; then
  pass "every-minute job fired 3× ($(runs_of "$minute_id") runs in ~3 min)"
else
  bad "every-minute job did not fire 3× in 200 s ($(runs_of "$minute_id") runs)"
fi

statuses="$(json runs "$minute_id" | jq -r '[.runs[].status] | unique | join(",")')"
[ "$statuses" = "ok" ] && pass "every run recorded ok" || bad "run statuses: $statuses"

sessions=0
for id in $(json runs "$minute_id" | jq -r '.runs[].id'); do
  n=$(find "$OINO_AGENT_DIR/sessions" -name "*_${id}.jsonl" 2>/dev/null | wc -l)
  sessions=$((sessions + n))
done
[ "$sessions" -ge 3 ] && pass "$sessions run session file(s) written" ||
  bad "run session files: $sessions"

summaries="$(json runs "$minute_id" | jq -r '[.runs[].summary // ""] | map(select(. != "")) | length')"
[ "$summaries" -ge 1 ] && pass "run summaries captured ($summaries)" ||
  bad "no run summary captured (JSON stream parsing)"

log="$(json log "$(json runs "$minute_id" | jq -r '.runs[0].id')" | jq -r 'if (.log | length) > 0 then "yes" else "no" end')"
[ "$log" = "yes" ] && pass "run log captured" || bad "run log empty"

# ------------------------------------------------------------------------- 2. at: job
at_time="$(date -u -d '+45 seconds' +%Y-%m-%dT%H:%M:%SZ)"
cron add --name "one shot" --prompt "Say OK and stop." --at "$at_time" --cwd "$work/jobs" >/dev/null ||
  bad "add at: job"
at_id="$(json list | jq -r '.jobs[] | select(.name == "one shot") | .id')"
if wait_runs "$at_id" 1 90 && wait_finished "$at_id" 120; then
  pass "at: job fired ($(json runs "$at_id" | jq -r '.runs[0].status'))"
  sleep 30
  [ "$(runs_of "$at_id")" = "1" ] && pass "at: job fired exactly once" ||
    bad "at: job fired $(runs_of "$at_id") times"
else
  bad "at: job did not fire"
fi

# -------------------------------------------------------------- 3. pause / resume
before="$(runs_of "$minute_id")"
cron pause >/dev/null && pass "pause accepted"
sleep 70
[ "$(runs_of "$minute_id")" = "$before" ] && pass "no run started while paused" ||
  bad "a run started while paused ($before → $(runs_of "$minute_id"))"
cron resume >/dev/null && pass "resume accepted"
wait_runs "$minute_id" $((before + 1)) 80 && pass "runs start again after resume" ||
  bad "no run after resume"

# ------------------------------------------------------ 4. missed-run policy
stop_daemon
missed_before="$(runs_of "$minute_id")"
echo "== daemon stopped for 70 s (one fire is missed)"
sleep 70
start_daemon || { bad "daemon restart"; exit 1; }
if wait_runs "$minute_id" $((missed_before + 1)) 40; then
  missed="$(json runs "$minute_id" | jq -r '.runs[0].missed')"
  delay="$(json runs "$minute_id" | jq -r '(.runs[0].started_at // 0) - .runs[0].fired_at')"
  [ "$missed" = "true" ] && pass "missed run caught up on restart (missed=true, ${delay} ms late)" ||
    bad "catch-up run not flagged missed (missed=$missed)"
else
  bad "missed run was not caught up after restart"
fi
missed_policy="$(cron settings --missed-window-s 60 >/dev/null && echo ok)"
[ "$missed_policy" = "ok" ] && pass "settings write (missed window) accepted" ||
  bad "settings write failed"
cron settings --missed-window-s 21600 >/dev/null

# ----------------------------------------------------- 5. install / uninstall
if [ "$service" = 1 ]; then
  unit="$XDG_CONFIG_HOME/systemd/user/oino-cron.service"
  out="$(cron install 2>&1)"
  if [ -f "$unit" ]; then
    pass "install wrote the unit file ($unit)"
    grep -q 'cron serve' "$unit" && pass "unit ExecStart runs 'cron serve'" ||
      bad "unit has no 'cron serve'"
  else
    bad "install wrote no unit file: $out"
  fi
  echo "      install said: $out"
  out="$(cron uninstall 2>&1)"
  [ ! -f "$unit" ] && pass "uninstall removed the unit file" ||
    bad "uninstall left the unit file: $out"
  echo "      uninstall said: $out"
  [ -f "$HOME/.config/systemd/user/oino-cron.service" ] &&
    bad "install leaked into \$HOME/.config" || pass "no service file outside XDG_CONFIG_HOME"
fi

# --------------------------------------------------------------- 6. real run (opt-in)
# The real run needs the bundled provider extensions (the faux phase turns them off), so the
# daemon is restarted for it — and the faux jobs are disabled first, so nothing keeps
# calling a real provider on a schedule.
if [ "$real" = 1 ]; then
  stop_daemon
  cron disable "$minute_id" >/dev/null 2>&1
  cron disable "$at_id" >/dev/null 2>&1
  src="${REAL_AUTH:-/home/oi/.oino/auth.json}"
  if [ -f "$src" ]; then
    cp "$src" "$OINO_AGENT_DIR/auth.json"
    env -u OINO_BUNDLED -u OINO_CRON_BIN "$bin" cron serve >>"$work/serve.log" 2>&1 &
    daemon=$!
    for _ in $(seq 1 50); do
      [ -f "$OINO_AGENT_DIR/cron/serve.json" ] && break
      sleep 0.2
    done
    at_time="$(date -u -d '+10 seconds' +%Y-%m-%dT%H:%M:%SZ)"
    cron add --name "real run" --prompt "Reply with exactly: cron-e2e-ok" \
      --at "$at_time" --cwd "$work/jobs" --model oino/deepseek/deepseek-flash \
      --time-limit-s 300 >/dev/null || bad "add real job"
    real_id="$(json list | jq -r '.jobs[] | select(.name == "real run") | .id')"
    if wait_finished "$real_id" 300; then
      ok="$(json runs "$real_id" | jq -r '.runs[0].status + " | " + (.runs[0].summary // "") + " | cost " + ((.runs[0].cost_usd // 0) | tostring)')"
      case "$ok" in
        ok*) pass "real run on oino/deepseek/deepseek-flash: $ok" ;;
        *) bad "real run: $ok" ;;
      esac
    else
      bad "real run did not finish"
    fi
  else
    bad "no auth.json at $src — real run skipped"
  fi
fi

stop_daemon
echo
if [ "$fail" = 0 ]; then
  echo "RESULT: PASS (all cron e2e checks)"
else
  echo "RESULT: FAIL ($fail check(s))"
fi
echo "artifacts: $work (serve.log, agent/cron/, agent/sessions/)"
exit $((fail > 0))
