#!/usr/bin/env bash
# Real-app QA rig: the INSTALLED UniPi Oino build (the .deb file tree, real
# unipi-oino-host, real bundled extensions) inside a private Xvfb display,
# driven over WebDriver (tauri-driver + WebKitWebDriver). Never the mock harness.
#
#   scripts/qa/rig.sh up [deb]   start Xvfb + tauri-driver + the app (isolated QA data)
#   scripts/qa/rig.sh down            stop everything
#   node scripts/qa/rig.mjs <cmd> …   drive it (shot/click/type/eval/…; see rig.mjs)
#
# QA data: $QA_HOME (default ~/.cache/oino-qa) with its own agent dir and host
# dir; ~/.oino/auth.json is copied in so providers work. Your ~/.oino, ~/.unipi
# and running apps are never touched.
set -euo pipefail
QA_HOME=${QA_HOME:-$HOME/.cache/oino-qa}
DISPLAY_NUM=${QA_DISPLAY:-:147}
SIZE=${QA_SIZE:-1440x900}
PORT=${QA_PORT:-4444}
cmd=${1:-}
case "$cmd" in
  up)
    pkg=${2:-$HOME/oino-qa-release/v2/unipi-oino_1.0.0_amd64.deb}
    mkdir -p "$QA_HOME"/{agent,run,shots}
    # The .deb's exact file tree (usr/bin/unipi-oino, unipi-oino-host, oino,
    # usr/share/oino/extensions) on the system WebKitGTK, as `apt install` lays it out.
    if [ ! -f "$QA_HOME/app/.from" ] || [ "$(cat "$QA_HOME/app/.from")" != "$(sha256sum "$pkg" | cut -c1-16)" ]; then
      rm -rf "$QA_HOME/app"; mkdir -p "$QA_HOME/app"
      dpkg-deb -x "$pkg" "$QA_HOME/app"
      sha256sum "$pkg" | cut -c1-16 > "$QA_HOME/app/.from"
    fi
    [ -f "$QA_HOME/agent/auth.json" ] || cp "$HOME/.oino/auth.json" "$QA_HOME/agent/" 2>/dev/null || true
    [ -f "$QA_HOME/agent/settings.json" ] || cp "$HOME/.oino/settings.json" "$QA_HOME/agent/" 2>/dev/null || true
    pgrep -f "Xvfb $DISPLAY_NUM" >/dev/null || { setsid nohup Xvfb "$DISPLAY_NUM" -screen 0 "${SIZE}x24" >"$QA_HOME/run/xvfb.log" 2>&1 & sleep 1; }
    pgrep -f "tauri-driver --port $PORT" >/dev/null || {
      env -u WAYLAND_DISPLAY DISPLAY="$DISPLAY_NUM" GDK_BACKEND=x11 \
        OINO_AGENT_DIR="$QA_HOME/agent" OINO_HOST_DIR="$QA_HOME/agent/host" \
        WEBKIT_DISABLE_COMPOSITING_MODE=1 \
        XDG_DATA_HOME="$QA_HOME/xdg/data" XDG_CONFIG_HOME="$QA_HOME/xdg/config" \
        XDG_CACHE_HOME="$QA_HOME/xdg/cache" XDG_STATE_HOME="$QA_HOME/xdg/state" \
        setsid nohup tauri-driver --port "$PORT" --native-driver "$HOME/.local/bin/WebKitWebDriver" \
        >"$QA_HOME/run/driver.log" 2>&1 &
      sleep 1; }
    echo "$QA_HOME/app/usr/bin/unipi-oino" > "$QA_HOME/run/app-path"
    node "$(dirname "$0")/rig.mjs" start
    ;;
  down)
    node "$(dirname "$0")/rig.mjs" stop 2>/dev/null || true
    pkill -f "tauri-driver --port $PORT" || true
    pkill -f "Xvfb $DISPLAY_NUM" || true
    pkill -f "$QA_HOME/app/usr/bin/unipi-oino" || true
    ;;
  *) sed -n '2,14p' "$0"; exit 1 ;;
esac
