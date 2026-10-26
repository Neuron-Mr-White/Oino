#!/usr/bin/env bash
# Headless screenshots of the --demo scenes (Xvfb + lavapipe software Vulkan + PIL ImageGrab).
#
#   apps/oino-desktop/scripts/screenshot.sh [out-dir] [binary] [shot...]
#
# Shots (design.md §10.1 P9): 01-welcome 02-streaming 03-approval-docked 04-question
# 05-palette 06-settings 07-board 08-light-theme 09-info-overlay, plus 10-plan-review
# 11-side-review 12-model-menu 13-settings-hub 14-extensions 15-split-panes 16-narrow, and
# 17-30 for the review fix-up (terminal, browser, light theme, find, completion, toasts, bottom
# frame, side tabs, menus, confirm dialog, onboarding, 720 px).
# Needs: Xvfb, python3-pil (XCB), mesa Vulkan (lvp). Each shot runs its own app instance.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
apps="$(cd "$here/../.." && pwd)"
repo="$(cd "$apps/.." && pwd)"
out="${1:-$repo/docs/dev/v1/design/shots/desktop}"
bin="${2:-$apps/target/debug/oino-desktop}"
shift $(( $# > 2 ? 2 : $# )) || true
mkdir -p "$out"
out="$(cd "$out" && pwd)"
W=1400; H=900

shoot() { # name wait-seconds env...
  local name="$1" wait="$2"; shift 2
  local display=":$((90 + RANDOM % 9))" w="${SHOT_W:-$W}" h="${SHOT_H:-$H}"
  Xvfb "$display" -screen 0 "${w}x${h}x24" -nolisten tcp >/dev/null 2>&1 &
  local xvfb=$!
  sleep 1
  local agent; agent="$(mktemp -d "${TMPDIR:-$HOME/.cache}/oino-shot.XXXX")"
  env -u WAYLAND_DISPLAY DISPLAY="$display" OINO_AGENT_DIR="$agent" \
    VK_ICD_FILENAMES="${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/lvp_icd.json}" \
    OINO_DESKTOP_SIZE="${w}x${h}" OINO_LOCAL_HOUR="${OINO_LOCAL_HOUR:-15}" OINO_REDUCE_MOTION=1 \
    "$@" "$bin" --demo --cwd "$HOME/Projects/json-lite" >/dev/null 2>&1 &
  local app=$!
  sleep "$wait"
  python3 -c "from PIL import ImageGrab; ImageGrab.grab(xdisplay='$display').save('$out/$name.png')"
  echo "$out/$name.png"
  kill "$app" 2>/dev/null || true
  wait "$app" 2>/dev/null || true
  kill "$xvfb" 2>/dev/null || true
  wait "$xvfb" 2>/dev/null || true
  rm -rf "$agent"
}

want() { [ "${#SHOTS[@]}" -eq 0 ] || printf '%s\n' "${SHOTS[@]}" | grep -qx "$1"; }
SHOTS=("$@")

want 01-welcome && shoot 01-welcome 9 OINO_DESKTOP_THEME=dark OINO_DESKTOP_DEMO_SCENE=welcome
want 02-streaming && shoot 02-streaming 10 OINO_DESKTOP_DEMO_PACE=5 OINO_DESKTOP_THEME=dark OINO_DESKTOP_DEMO_SCENE=run OINO_DESKTOP_DEMO_APPROVE_MS=60000
want 03-approval-docked && shoot 03-approval-docked 18 OINO_DESKTOP_THEME=dark OINO_DESKTOP_DEMO_SCENE=approval
want 04-question && shoot 04-question 14 OINO_DESKTOP_THEME=dark OINO_DESKTOP_DEMO_SCENE=question
want 05-palette && shoot 05-palette 10 OINO_DESKTOP_THEME=dark OINO_DESKTOP_DEMO_SCENE=welcome OINO_DESKTOP_OPEN=palette
want 06-settings && shoot 06-settings 10 OINO_DESKTOP_THEME=dark OINO_DESKTOP_DEMO_SCENE=welcome OINO_DESKTOP_OPEN=settings
want 07-board && shoot 07-board 12 OINO_DESKTOP_THEME=dark OINO_DESKTOP_DEMO_SCENE=board OINO_DESKTOP_OPEN=board
want 08-light-theme && shoot 08-light-theme 30 OINO_DESKTOP_THEME=light OINO_DESKTOP_DEMO_SCENE=run OINO_DESKTOP_DEMO_APPROVE_MS=9000
want 09-info-overlay && shoot 09-info-overlay 11 OINO_DESKTOP_THEME=dark OINO_DESKTOP_DEMO_SCENE=info
want 10-plan-review && shoot 10-plan-review 14 OINO_DESKTOP_THEME=dark OINO_DESKTOP_DEMO_SCENE=plan
want 11-side-review && shoot 11-side-review 30 OINO_DESKTOP_THEME=dark OINO_DESKTOP_DEMO_SCENE=run OINO_DESKTOP_DEMO_APPROVE_MS=9000 OINO_DESKTOP_OPEN=side
want 12-model-menu && shoot 12-model-menu 10 OINO_DESKTOP_THEME=dark OINO_DESKTOP_DEMO_SCENE=welcome OINO_DESKTOP_OPEN=model
want 13-settings-hub && shoot 13-settings-hub 11 OINO_DESKTOP_THEME=light OINO_DESKTOP_DEMO_SCENE=welcome OINO_DESKTOP_OPEN=settings:notify
want 14-extensions && shoot 14-extensions 10 OINO_DESKTOP_THEME=dark OINO_DESKTOP_DEMO_SCENE=welcome OINO_DESKTOP_OPEN=extensions
want 15-split-panes && shoot 15-split-panes 14 OINO_DESKTOP_THEME=dark OINO_DESKTOP_DEMO_SCENE=stream OINO_DESKTOP_PANES=4
want 16-narrow && SHOT_W=860 SHOT_H=760 shoot 16-narrow 30 OINO_DESKTOP_THEME=dark OINO_DESKTOP_DEMO_SCENE=run OINO_DESKTOP_DEMO_APPROVE_MS=9000
# Review fix-up evidence (review-desktop.md P1-13, A1.4/A1.5).
want 17-terminal && shoot 17-terminal 12 OINO_DESKTOP_THEME=dark OINO_DESKTOP_DEMO_SCENE=welcome OINO_DESKTOP_OPEN=terminal OINO_DESKTOP_TERM_INPUT='printf "\\033[1;31mred\\033[0m \\033[32mgreen\\033[0m \\033[34mblue\\033[0m\\n"; ls --color=always /'
want 18-browser && shoot 18-browser 14 OINO_DESKTOP_THEME=dark OINO_DESKTOP_DEMO_SCENE=welcome "OINO_DESKTOP_OPEN=browser:file://$here/preview.html"
want 19-light-welcome && shoot 19-light-welcome 9 OINO_DESKTOP_THEME=light OINO_DESKTOP_DEMO_SCENE=welcome
want 20-light-approval && shoot 20-light-approval 18 OINO_DESKTOP_THEME=light OINO_DESKTOP_DEMO_SCENE=approval
want 21-find && shoot 21-find 14 OINO_DESKTOP_THEME=dark OINO_DESKTOP_DEMO_SCENE=plan OINO_DESKTOP_OPEN=find:parser
want 22-completion && shoot 22-completion 10 OINO_DESKTOP_THEME=dark OINO_DESKTOP_DEMO_SCENE=welcome OINO_DESKTOP_OPEN=composer:/
want 23-toasts && shoot 23-toasts 10 OINO_DESKTOP_THEME=dark OINO_DESKTOP_DEMO_SCENE=welcome OINO_DESKTOP_OPEN=toast
want 24-bottom-output && shoot 24-bottom-output 30 OINO_DESKTOP_THEME=dark OINO_DESKTOP_DEMO_SCENE=run OINO_DESKTOP_DEMO_APPROVE_MS=9000 OINO_DESKTOP_OPEN=bottom:output
want 25-side-files && shoot 25-side-files 10 OINO_DESKTOP_THEME=dark OINO_DESKTOP_DEMO_SCENE=welcome OINO_DESKTOP_OPEN=side:files
want 26-side-launcher && shoot 26-side-launcher 10 OINO_DESKTOP_THEME=dark OINO_DESKTOP_DEMO_SCENE=welcome OINO_DESKTOP_OPEN=side:launcher
want 27-session-menu && shoot 27-session-menu 10 OINO_DESKTOP_THEME=dark OINO_DESKTOP_DEMO_SCENE=welcome OINO_DESKTOP_OPEN=session-menu
want 28-delete-confirm && shoot 28-delete-confirm 10 OINO_DESKTOP_THEME=dark OINO_DESKTOP_DEMO_SCENE=welcome OINO_DESKTOP_OPEN=delete
want 29-onboarding && shoot 29-onboarding 8 OINO_DESKTOP_THEME=dark OINO_DESKTOP_DEMO_SCENE=welcome OINO_DESKTOP_ONBOARDING=1
want 30-720 && SHOT_W=720 SHOT_H=600 shoot 30-720 30 OINO_DESKTOP_THEME=dark OINO_DESKTOP_DEMO_SCENE=run OINO_DESKTOP_DEMO_APPROVE_MS=9000
true
