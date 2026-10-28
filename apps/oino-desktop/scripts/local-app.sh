#!/usr/bin/env bash
# Local charter-fallback app lifecycle for the GUI driver (docs/dev/v1/CHARTER.md: GUI
# testing runs on `ti`; only while `ti` is unreachable, one app instance on a private
# Xvfb here). Mirrors `cargo xtask ti start` (xtask/src/ti.rs): private Xvfb :95, own
# dbus session, scratch agent dir + project, lavapipe, reduce motion.
#
#   local-app.sh start <demo|faux|real> <scene|-> [ENV=V ...]
#   local-app.sh stop
set -euo pipefail
root="$(cd "$(dirname "$0")/../../.." && pwd)"   # repo root (script lives in apps/oino-desktop/scripts)
run="$root/target/tmp/local-run"
sock="$run/automation.sock"
display=":95"

stop() {
  for f in app dbus xvfb; do
    [ -f "$run/$f.pid" ] || continue
    pid=$(cat "$run/$f.pid")
    if kill -0 "$pid" 2>/dev/null; then kill "$pid" 2>/dev/null || true; fi
    rm -f "$run/$f.pid"
  done
  rm -f "$sock"
  sleep 1
}

case "${1:-}" in
  stop) stop; echo stopped; exit 0 ;;
  start) ;;
  *) echo "usage: $0 start <demo|faux|real> <scene|-> [ENV=V ...] | stop" >&2; exit 2 ;;
esac
backend="$2"; scene="$3"; shift 3 || true

stop
bin="$root/apps/target/release/oino-desktop"
[ -x "$bin" ] || { echo "no binary at $bin" >&2; exit 1; }
agent="$run/agent"; project="$run/project"
rm -rf "$agent/sessions" "$agent/state" "$agent/desktop.json" "$project/.oino" "$project/docs"
mkdir -p "$agent/logs" "$run" "$project"
printf 'oino test project\n' > "$project/README.md"

args=()
case "$backend" in
  demo) args=(--demo) ;;
  faux)
    # Mirror `cargo xtask ti start --faux` (H-5): the binary has no --faux flag — a faux run
    # is a real session whose settings select the provider-faux fixture, which ships as the
    # directory extensions/dist/oino.fixture.provider-faux (there is no provider-faux.wasm).
    printf '%s\n' '{"defaultProvider":"faux","defaultModel":"faux-1"}' > "$agent/settings.json"
    printf '%s\n' '{"onboarded":true}' > "$agent/desktop.json"
    [ -d "$root/extensions/dist/oino.fixture.provider-faux" ] \
      || { echo "no provider-faux fixture in extensions/dist (run: cargo xtask wasm)" >&2; exit 1; }
    mkdir -p "$agent/extensions"
    rm -rf "$agent/extensions/provider-faux"
    cp -r "$root/extensions/dist/oino.fixture.provider-faux" "$agent/extensions/provider-faux"
    [ -f "$HOME/.oino/auth.json" ] \
      || { echo "no auth file at $HOME/.oino/auth.json" >&2; exit 1; }
    python3 - "$HOME/.oino/auth.json" "$agent/auth.json" <<'PY'
import json, sys
with open(sys.argv[1]) as handle:
    doc = json.load(handle)
if "router" in doc:
    doc.setdefault("oino", doc["router"])
doc.setdefault("faux", {"type": "api_key", "key": "k"})
with open(sys.argv[2], "w") as handle:
    json.dump(doc, handle)
PY
    chmod 600 "$agent/auth.json"
    ;;
  real)
    printf '%s\n' '{"defaultProvider":"oino","defaultModel":"deepseek/deepseek-flash"}' > "$agent/settings.json"
    printf '%s\n' '{"onboarded":true}' > "$agent/desktop.json"
    mkdir -p "$agent/cache/models"
    cp "$HOME/.oino/cache/models/oino.json" "$agent/cache/models/" 2>/dev/null \
      || echo "warning: no cached router model list (~/.oino/cache/models/oino.json)" >&2
    cp "$HOME/.oino/auth.json" "$agent/auth.json" && chmod 600 "$agent/auth.json"
    convert -size 96x96 'xc:#e03018' "$run/attach-red.png" 2>/dev/null \
      || python3 -c "from PIL import Image; Image.new('RGB', (96, 96), (224, 48, 24)).save('$run/attach-red.png')" 2>/dev/null \
      || echo "warning: neither convert nor PIL; the gui-drive client creates attach-red.png on demand" >&2
    ;;
  *) echo "unknown backend $backend" >&2; exit 2 ;;
esac

Xvfb "$display" -screen 0 1600x1000x24 -nolisten tcp > "$run/xvfb.log" 2>&1 &
echo $! > "$run/xvfb.pid"
for _ in $(seq 50); do [ -e "/tmp/.X11-unix/X95" ] && break; sleep 0.2; done
eval "$(dbus-launch --sh-syntax)"
echo "$DBUS_SESSION_BUS_PID" > "$run/dbus.pid"

env_extra=()
if [ "$scene" != "-" ]; then env_extra+=("OINO_DESKTOP_DEMO_SCENE=$scene"); fi
for pair in "$@"; do env_extra+=("$pair"); done

cd "$project"
# The app runs with a minimal PATH: the owner-local tool dirs (~/.local/bin, ~/.cargo/bin)
# stay out, so owner-hosted services behind them (MemPalace daemon) can never leak into a
# scratch run — the unipi memory extension stays in its markdown-only mode, exactly like on
# ti, where no mempalace exists (QA-r4 finding: `/memory-search` once answered from the
# owner's daemon store, and `memory_store` wrote a test drawer into it).
nohup env -u WAYLAND_DISPLAY PATH="/usr/bin:/bin" DISPLAY="$display" DBUS_SESSION_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS" \
  OINO_AGENT_DIR="$agent" OINO_DESKTOP_SIZE=1600x1000 OINO_REDUCE_MOTION=1 \
  OINO_DESKTOP_AUTOMATION="$sock" \
  VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json \
  "${env_extra[@]}" \
  "$bin" ${args[@]+"${args[@]}"} > "$run/app.log" 2>&1 < /dev/null &
echo $! > "$run/app.pid"
sleep 2
kill -0 "$(cat "$run/app.pid")" 2>/dev/null || { echo "app exited:"; tail -20 "$run/app.log"; exit 1; }
echo "app pid $(cat "$run/app.pid") on $display, socket $sock ($backend, scene $scene)"
