#!/usr/bin/env bash
# Release build on the current host (quality §6 `release` job). `cargo xtask remote-build`
# runs this locally (`local`) or on a build host after syncing the tree; it can also be run by
# hand from the repository root.
#
#   scripts/release/build-host.sh [--universal] [--no-cli] [--no-smoke]
#
# Steps:
#   1. `cargo xtask checksums`: extensions/dist must be present and current (it is not built
#      here: build hosts have no wasm toolchain; `remote-build` ships the local dist).
#   2. `cargo build --release -p oino-cli` (macOS --universal: aarch64 + x86_64,
#      lipo'd; the thin x86_64 binaries are kept as well). The UniPi kanboard CLI is
#      not a release binary: it ships inside the unipi.kanboard extension package
#      (dist.md §3.4, `cargo xtask wasm`).
#   3. Copies everything into target/release-artifacts/<target>/ with SHA256SUMS,
#      build-info.txt and smoke.txt (smoke tests: --version, a faux-provider print run with
#      the bundled components).
#
# Environment: JOBS (cargo -j, default 2).
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"
universal=0 cli=1 smoke=1
while [[ $# -gt 0 ]]; do
  case "$1" in
    --universal) universal=1 ;;
    --no-cli) cli=0 ;;
    --no-smoke) smoke=0 ;;
    -h|--help) sed -n '2,16p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

export JOBS="${JOBS:-2}"
[[ -d "$HOME/.cargo/bin" ]] && export PATH="$HOME/.cargo/bin:$PATH"
case "$(uname -s)" in
  Linux*) os=linux ;;
  Darwin*) os=macos ;;
  MINGW*|MSYS*|CYGWIN*) os=windows ;;
  *) echo "unsupported host: $(uname -s)" >&2; exit 1 ;;
esac
exe=""; [[ $os == windows ]] && exe=".exe"
host="$(rustc -vV | sed -n 's/^host: //p')"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
target="$host"
[[ $os == macos && $universal == 1 ]] && target=universal-apple-darwin
art="$root/target/release-artifacts"
out="$art/$target"
smoke_log="$out/smoke.txt"

log() { printf '\033[1;35m==>\033[0m %s\n' "$*"; }
# Bounded commands: timeout (coreutils) / gtimeout (Homebrew) when available.
tmo() { local t="$1"; shift
  if command -v timeout >/dev/null; then timeout "$t" "$@"
  elif command -v gtimeout >/dev/null; then gtimeout "$t" "$@"
  else "$@"; fi; }
sum() { if command -v sha256sum >/dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }
note() { printf '%s\n' "$*" | tee -a "$smoke_log"; }
# check <name> <command…>: runs a smoke step, records PASS/FAIL with the first output line.
check() {
  local name="$1"; shift
  local output status=0
  output="$("$@" 2>&1)" || status=$?
  if [[ $status == 0 ]]; then
    note "PASS $name: $(printf '%s' "$output" | head -1)"
  elif [[ $status == 3 ]]; then # partial: e.g. launched but no screenshot possible
    note "PARTIAL $name: $(printf '%s' "$output" | tail -1)"
  else
    note "FAIL $name (exit $status): $(printf '%s' "$output" | tail -3 | tr '\n' ' ')"
  fi
}

rm -rf "$out"
mkdir -p "$out"
: > "$smoke_log"

log "checking extensions/dist (cargo xtask checksums)"
cargo xtask checksums

cli_build() { # $1 = optional target triple
  local args=(build --release -j "$JOBS" -p oino-cli)
  [[ -n "${1:-}" ]] && args+=(--target "$1")
  log "cargo ${args[*]}"
  cargo "${args[@]}"
}

if [[ $cli == 1 ]]; then
  if [[ $target == universal-apple-darwin ]]; then
    rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
    cli_build aarch64-apple-darwin
    cli_build x86_64-apple-darwin
    for bin in oino; do
      lipo -create -output "$out/$bin" "target/aarch64-apple-darwin/release/$bin" \
        "target/x86_64-apple-darwin/release/$bin"
    done
    mkdir -p "$art/x86_64-apple-darwin"
    cp target/x86_64-apple-darwin/release/oino "$art/x86_64-apple-darwin/"
  else
    cli_build ""
    cp "target/release/oino$exe" "$out/"
  fi
fi

# ── smoke tests ──────────────────────────────────────────────────────────────

faux_run() { # $1 = oino binary
  local sandbox; sandbox="$(mktemp -d "${TMPDIR:-/tmp}/oino-smoke.XXXX")"
  mkdir -p "$sandbox/home" "$sandbox/agent" "$sandbox/work"
  printf '{"faux":{"type":"api_key","key":"k"}}' > "$sandbox/agent/auth.json"
  local faux="$root/extensions/dist/oino.fixture.provider-faux"
  [[ $os == windows ]] && faux="$(cygpath -w "$faux")"
  local reply status=0
  # macOS shells (nohup/ssh) default to 256 descriptors, too few for the bundle
  # (EMFILE while instantiating components): raise like a GUI login session.
  [[ $os == macos ]] && ulimit -n 4096 2>/dev/null
  reply="$(cd "$sandbox/work" && HOME="$sandbox/home" USERPROFILE="$sandbox/home" \
    OINO_AGENT_DIR="$sandbox/agent" tmo 300 "$1" -e "$faux" --model faux/faux-1 -p hi 2>&1)" \
    || status=$?
  local bundled; bundled="$(ls "$sandbox/agent/bundled" 2>/dev/null | wc -l | tr -d ' ')"
  rm -rf "$sandbox"
  [[ $status == 0 && $reply == "Hello from faux." ]] || { echo "$reply"; return 1; }
  echo "\"$reply\" ($bundled bundled packages unpacked)"
}

if [[ $smoke == 1 ]]; then
  log "smoke tests"
  if [[ -f "$out/oino$exe" ]]; then
    check "oino --version" "$out/oino$exe" --version
    check "oino faux print run" faux_run "$out/oino$exe"
    if [[ $target == universal-apple-darwin ]]; then
      check "lipo oino" lipo -archs "$out/oino"
      check "x86_64 oino --version" "$art/x86_64-apple-darwin/oino" --version
    fi
  fi
fi

# ── metadata ─────────────────────────────────────────────────────────────────

dirs=("$out")
[[ $target == universal-apple-darwin && $cli == 1 ]] && dirs+=("$art/x86_64-apple-darwin")
for dir in "${dirs[@]}"; do
  {
    echo "target: $(basename "$dir")"
    echo "commit: $(git rev-parse HEAD 2>/dev/null || echo unknown)"
    echo "version: $version"
    echo "rustc: $(rustc -V)"
    echo "built: $(date -u +%Y-%m-%dT%H:%M:%SZ) on $(uname -sm)"
  } > "$dir/build-info.txt"
  (cd "$dir" && for f in *; do
    [[ -f $f && $f != SHA256SUMS ]] && sum "$f"
  done > SHA256SUMS) || true
done
log "artifacts in $out"
ls -la "$out"
if grep -q '^FAIL' "$smoke_log"; then
  log "smoke failures:"; grep '^FAIL' "$smoke_log"
fi
