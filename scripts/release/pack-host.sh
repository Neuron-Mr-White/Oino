#!/usr/bin/env bash
# Pack the GUI-less host archive that `scripts/install.sh --host` (and
# `install.ps1 -HostInstall`) installs:
#
#   unipi-oino-host-<version>-<target>.tar.gz   (Linux / macOS)
#     bin/unipi-oino-host  bin/oino  share/oino/extensions/<pkg>/
#   unipi-oino-host-<version>-<target>.zip      (Windows)
#     bin\unipi-oino-host.exe  bin\oino.exe  bin\extensions\<pkg>\
#
# The extension tree is the release copy (scripts/release/stage-dist.sh: no
# fixtures, no checksums.json); both binaries find it through their
# exe-relative candidate (dist.md §5).
#
#   scripts/release/pack-host.sh <version> <target> <unipi-host binary> <oino binary> [<out dir>]
#
# Environment: HOST_STAGE_EXTENSIONS = an already staged extensions dir to use
# instead of running stage-dist.sh (the release workflow passes the copy it
# just added the kanboard CLI to).
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
version=${1:?usage: pack-host.sh <version> <target> <host bin> <oino bin> [out dir]}
target=${2:?target triple}
host_bin=${3:?unipi-host binary}
oino_bin=${4:?oino binary}
out=${5:-$root/target/release-artifacts}

[ -x "$host_bin" ] || { echo "pack-host: $host_bin is not an executable" >&2; exit 1; }
[ -x "$oino_bin" ] || { echo "pack-host: $oino_bin is not an executable" >&2; exit 1; }

stage=$(mktemp -d "${TMPDIR:-/tmp}/pack-host.XXXXXX")
trap 'rm -rf "$stage"' EXIT
exts="${HOST_STAGE_EXTENSIONS:-}"
if [ -z "$exts" ]; then
  bash "$root/scripts/release/stage-dist.sh" "$stage/dist" >/dev/null
  exts="$stage/dist/extensions"
fi

mkdir -p "$stage/out/bin" "$out"
name="unipi-oino-host-$version-$target"
case "$target" in
  *windows*)
    cp "$host_bin" "$stage/out/bin/unipi-oino-host.exe"
    cp "$oino_bin" "$stage/out/bin/oino.exe"
    cp -R "$exts" "$stage/out/bin/extensions"
    rm -f "$out/$name.zip"
    (cd "$stage/out" && if command -v zip >/dev/null; then zip -qr "$out/$name.zip" bin
      else 7z a -tzip -bd "$out/$name.zip" bin >/dev/null; fi) # 7z: Windows runners
    echo "$out/$name.zip"
    ;;
  *)
    cp "$host_bin" "$stage/out/bin/unipi-oino-host"
    cp "$oino_bin" "$stage/out/bin/oino"
    chmod 755 "$stage/out/bin/"*
    mkdir -p "$stage/out/share/oino"
    cp -R "$exts" "$stage/out/share/oino/extensions"
    tar -czf "$out/$name.tar.gz" -C "$stage/out" bin share
    echo "$out/$name.tar.gz"
    ;;
esac
