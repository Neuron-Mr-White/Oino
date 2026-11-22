#!/usr/bin/env bash
# Decides whether this desktop build signs Tauri updater artifacts.
#
# Output (GITHUB_OUTPUT): artifacts=true|false, merged into the release config
# as bundle.createUpdaterArtifacts. The bundler then signs with
# TAURI_SIGNING_PRIVATE_KEY(_PASSWORD) and writes the .sig files that
# scripts/release/make-feeds.sh puts in https://oino.dev/app/desktop/latest.json.
#
# A tag build without the key FAILS: a release without signatures can never be
# installed by the in-app updater. A workflow_dispatch rehearsal (or a fork)
# builds without updater artifacts instead.
set -euo pipefail

if [ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
  if [ -z "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" ]; then
    echo "::error::TAURI_SIGNING_PRIVATE_KEY is set but TAURI_SIGNING_PRIVATE_KEY_PASSWORD is not (the release key is encrypted)"
    exit 1
  fi
  echo "artifacts=true" >> "$GITHUB_OUTPUT"
  echo "updater artifacts: signed"
elif [ "${GITHUB_REF_TYPE:-}" = tag ]; then
  echo "::error::TAURI_SIGNING_PRIVATE_KEY secret is missing; a tag release must ship signed updater artifacts"
  exit 1
else
  echo "::notice::no TAURI_SIGNING_PRIVATE_KEY secret; building without updater artifacts (rehearsal)"
  echo "artifacts=false" >> "$GITHUB_OUTPUT"
fi
