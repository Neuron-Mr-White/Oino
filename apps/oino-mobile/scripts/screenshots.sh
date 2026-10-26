#!/usr/bin/env bash
# Phone screenshots of the design.md §6 screens from the demo backend (design-mobile M-P6).
#
#   apps/oino-mobile/scripts/screenshots.sh [--serial <adb serial>] [--out <dir>]
#                                           [--only <name-substring>] [--wait <s>]
#
# Needs a debug build installed (`scripts/build.sh android --demo`): each shot writes the
# demo marker `files/oino/mobile-demo` (scene / theme for the demo backend + a launch:
# `open=`, `sheet=`, `drawer=`, …), restarts the app, waits and saves
# `adb exec-out screencap -p` to <out>/<name>.png (default docs/dev/v1/design/shots/mobile).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
out="$root/docs/dev/v1/design/shots/mobile"
serial="${ANDROID_SERIAL:-}"
only=""
wait_s=14
while [[ $# -gt 0 ]]; do
  case "$1" in
    --serial) serial="$2"; shift ;;
    --out) out="$2"; shift ;;
    --only) only="$2"; shift ;;
    --wait) wait_s="$2"; shift ;;
    -h|--help) sed -n '2,10p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done
adb=(adb)
[[ -n $serial ]] && adb=(adb -s "$serial")
pkg=dev.oino.mobile
mkdir -p "$out"

# name | extra wait (s) | marker lines (`;`-separated)
shots=(
  "m01-sessions-dark|8|theme=dark;scene=approval;open=new;home=1"
  "m01-sessions-light|8|theme=light;scene=approval;open=new;home=1"
  "m01b-sessions-swipe|0|theme=dark;swipe=1"
  "m01c-sessions-row-menu|0|theme=dark;menu=1"
  "m02-welcome-dark|0|theme=dark;scene=welcome;open=new"
  "m02-conversation-dark|10|theme=dark;scene=run;approve=1500;open=new"
  "m02-conversation-light|10|theme=light;scene=run;approve=1500;open=new"
  "m03-approval-dark|8|theme=dark;scene=approval;open=new"
  "m03-approval-light|8|theme=light;scene=approval;open=new"
  "m03-question-dark|6|theme=dark;scene=question;open=new"
  "m03-plan-sheet-dark|8|theme=dark;scene=plan;open=new"
  "m04-drawer-dark|12|theme=dark;scene=run;approve=1500;open=new;drawer=1"
  "m05-info-page-dark|6|theme=dark;scene=info;open=new"
  "m06-model-sheet-dark|0|theme=dark;scene=welcome;open=new;sheet=model"
  "m06-model-sheet-light|0|theme=light;scene=welcome;open=new;sheet=model"
  "m06-actions-sheet-dark|0|theme=dark;scene=welcome;open=new;sheet=actions"
  "m06-status-sheet-dark|10|theme=dark;scene=run;open=new;sheet=status"
  "m07-search-dark|0|theme=dark;open=search;query=fix"
  "m08-settings-dark|0|theme=dark;open=settings"
  "m08-settings-appearance-light|0|theme=light;open=settings:appearance"
  "m09-board-dark|6|theme=dark;scene=board;open=board"
  "m09-board-light|6|theme=light;scene=board;open=board"
  "m10-hosts-dark|0|theme=dark;open=hosts"
  "m10-pair-dark|0|theme=dark;open=pair"
)

for shot in "${shots[@]}"; do
  IFS='|' read -r name extra marker <<<"$shot"
  [[ -n $only && $name != *"$only"* ]] && continue
  printf '==> %s\n' "$name"
  lines="${marker//;/\\n}"
  "${adb[@]}" shell am force-stop "$pkg"
  "${adb[@]}" shell "run-as $pkg sh -c 'mkdir -p files/oino && printf \"$lines\\n\" > files/oino/mobile-demo'"
  "${adb[@]}" shell am start -n "$pkg/dev.gpui.mobile.GpuiActivity" >/dev/null
  sleep $((wait_s + extra))
  "${adb[@]}" exec-out screencap -p > "$out/$name.png"
done
"${adb[@]}" shell "run-as $pkg sh -c 'printf \"\" > files/oino/mobile-demo'"
echo "shots in $out"
