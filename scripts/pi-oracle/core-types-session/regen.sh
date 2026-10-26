#!/usr/bin/env bash
# Regenerates crates/oino-session/tests/fixtures/pi from pi 1.4.0 (needs Node 24).
# Real-session inputs (real-sessions.txt, relative to ~/.pi/agent/sessions) are only
# re-scrubbed when SCRUB_REAL=1 because they are machine specific.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
fixtures="$here/../../../crates/oino-session/tests/fixtures/pi"
mkdir -p "$fixtures/generated" "$fixtures/real" "$fixtures/legacy-pi"
node "$here/generate.mjs" "$fixtures/generated"
if [[ "${SCRUB_REAL:-0}" == 1 ]]; then
  mapfile -t pairs < <(sed "s|=|=$HOME/.pi/agent/sessions/|" "$here/real-sessions.txt")
  node "$here/scrub.mjs" "$fixtures/real" "${pairs[@]}"
fi
for d in generated real legacy-pi docs; do
  [[ -d "$fixtures/$d" ]] && node "$here/context.mjs" "$fixtures/$d"
done
