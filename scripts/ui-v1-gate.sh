#!/usr/bin/env sh
# ui-protocol v2 migration gate (WP-U7, docs/dev/v2/specs/ui-protocol.md §12.3).
#
# Fails when an extension still emits an OUS v1 node tree: a literal `"t": "<lowercase kind>"`
# (v2 tags are capitalised, `"t": "Paragraph"`, and core rejects v1 trees at the registry gate,
# §4.8). `unipi-kit/src/ous.rs` is the transitional v1 builder module; it fails the gate on
# purpose so it disappears with its last caller.
#
# Usage: scripts/ui-v1-gate.sh [root]   (defaults to the repository root)
set -eu
root=${1:-$(cd "$(dirname "$0")/.." && pwd)}
dir="$root/extensions"
pattern='"t"[[:space:]]*:[[:space:]]*"[a-z]'

if command -v rg >/dev/null 2>&1; then
    hits=$(rg -n --glob '*.rs' "$pattern" "$dir" || true)
else
    hits=$(grep -rnE --include='*.rs' "$pattern" "$dir" || true)
fi

if [ -n "$hits" ]; then
    echo "ui-v1-gate: OUS v1 node trees in extensions (ui-protocol v2 §12.1/§12.3):" >&2
    echo "$hits" >&2
    echo "ui-v1-gate: migrate them to the v2 builder (oino_extension::ui::widgets)." >&2
    exit 1
fi
echo "ui-v1-gate: no OUS v1 node trees in extensions"
