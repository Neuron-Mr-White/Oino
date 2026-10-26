#!/usr/bin/env sh
# Regenerate the settings/resources golden fixtures from pi 1.4.0.
set -eu
cd "$(dirname "$0")"
for script in settings.mjs system-prompt.mjs misc.mjs resources.mjs; do
  node "$script"
done
