#!/usr/bin/env sh
# Regression test for scripts/release/version.sh: check mode on the real tree,
# drift detection on each manifest/lockfile and tag mismatch, and bump mode in
# an isolated temp tree.
set -eu

root="$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)"
ver_sh="$root/scripts/release/version.sh"

# 1. Real repo must pass --check right now.
current=$(sh "$ver_sh" --check --root "$root")
[ -n "$current" ] || { echo "FAIL: empty version" >&2; exit 1; }
sh "$ver_sh" --check --root "$root" --tag "v$current" >/dev/null

# 2. Isolated fixture copy of the 7 manifests/lockfiles (only the package blocks needed).
tmp=$(mktemp -d "${TMPDIR:-/tmp}/oino-version-test.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

mkdir -p "$tmp/apps/unipi/src-tauri" "$tmp/apps/unipi/crates/unipi-host"
cp "$root/apps/unipi/src-tauri/tauri.conf.json" "$tmp/apps/unipi/src-tauri/tauri.conf.json"
cp "$root/apps/unipi/src-tauri/Cargo.toml" "$tmp/apps/unipi/src-tauri/Cargo.toml"
cp "$root/apps/unipi/crates/unipi-host/Cargo.toml" "$tmp/apps/unipi/crates/unipi-host/Cargo.toml"
cp "$root/apps/unipi/package.json" "$tmp/apps/unipi/package.json"
cat > "$tmp/apps/unipi/src-tauri/Cargo.lock" <<EOF
version = 4

[[package]]
name = "other-crate"
version = "0.1.0"

[[package]]
name = "unipi-mobile"
version = "$current"
EOF
cat > "$tmp/apps/unipi/Cargo.lock" <<EOF
version = 4

[[package]]
name = "unipi-host"
version = "$current"
EOF
cat > "$tmp/apps/unipi/package-lock.json" <<EOF
{
  "name": "@unipi/mobile",
  "version": "$current",
  "lockfileVersion": 3,
  "packages": {
    "": {
      "name": "@unipi/mobile",
      "version": "$current"
    }
  }
}
EOF

# 3. Tag mismatch must fail.
if sh "$ver_sh" --check --root "$tmp" --tag "v9.9.9" 2>/dev/null; then
  echo "FAIL: expected --tag v9.9.9 mismatch to fail" >&2
  exit 1
fi

# 4. Bump to 2.3.4 updates all 7 files and passes --check --tag v2.3.4.
bumped=$(sh "$ver_sh" 2.3.4 --root "$tmp" 2>/dev/null)
[ "$bumped" = "2.3.4" ] || { echo "FAIL: expected 2.3.4, got $bumped" >&2; exit 1; }
sh "$ver_sh" --check --root "$tmp" --tag v2.3.4 >/dev/null

# 5. Drift in unipi-host/Cargo.toml must fail --check.
python3 -c '
import pathlib
p = pathlib.Path("'"$tmp"'/apps/unipi/crates/unipi-host/Cargo.toml")
p.write_text(p.read_text().replace("version = \"2.3.4\"", "version = \"2.3.3\"", 1))
'
if sh "$ver_sh" --check --root "$tmp" 2>/dev/null; then
  echo "FAIL: expected drifted unipi-host/Cargo.toml to fail --check" >&2
  exit 1
fi

# 6. If the sibling oino-landing-page checkout exists, test --site bump + check in temp.
site_repo="$root/../oino-landing-page"
if [ -f "$site_repo/scripts/fill-release.mjs" ] && command -v node >/dev/null 2>&1; then
  mkdir -p "$tmp/site/src/data" "$tmp/site/src/lib" "$tmp/site/scripts"
  cp "$site_repo/package.json" "$tmp/site/package.json"
  cp "$site_repo/src/data/release.json" "$tmp/site/src/data/release.json"
  cp "$site_repo/src/lib/release.mjs" "$tmp/site/src/lib/release.mjs"
  cp "$site_repo/scripts/fill-release.mjs" "$tmp/site/scripts/fill-release.mjs"
  sh "$ver_sh" 2.3.5 --root "$tmp" --site "$tmp/site" >/dev/null 2>&1
  sh "$ver_sh" --check --root "$tmp" --tag v2.3.5 --site "$tmp/site" >/dev/null
fi

echo "version-test passed ($current)"
