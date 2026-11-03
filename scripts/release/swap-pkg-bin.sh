#!/usr/bin/env bash
# Swap a native binary inside a STAGED extension package for another target's
# build and refresh its sha256 in the package's extension.json `checksums`.
#
#   scripts/release/swap-pkg-bin.sh <package dir> <file name in bin/> <new binary>
#   e.g. swap-pkg-bin.sh target/dist-stage/extensions/unipi.kanboard unipi-kanboard \
#          apps/unipi/target/x86_64-unknown-linux-musl/release/unipi-kanboard
#
# `cargo xtask wasm` builds a package's `binaries` for the build host and
# records their hashes; the loader refuses a package whose recorded hash does
# not match (Manifest::verify_files), so an archive that ships another target's
# build (static musl, Windows .exe) must rewrite the entry or the whole package
# is skipped at load. The other spelling of the same binary (`<stem>` vs
# `<stem>.exe`) and its checksum entry are removed.
set -euo pipefail

pkg=${1:?usage: swap-pkg-bin.sh <package dir> <file name in bin/> <new binary>}
name=${2:?file name in bin/}
src=${3:?new binary}
[ -f "$pkg/extension.json" ] || { echo "swap-pkg-bin: $pkg has no extension.json" >&2; exit 1; }
[ -f "$src" ] || { echo "swap-pkg-bin: $src is missing" >&2; exit 1; }
case "$name" in */*|"") echo "swap-pkg-bin: bad file name $name" >&2; exit 2 ;; esac

stem=${name%.exe}
rm -f "$pkg/bin/$stem" "$pkg/bin/$stem.exe"
mkdir -p "$pkg/bin"
cp "$src" "$pkg/bin/$name"
chmod 755 "$pkg/bin/$name"

py=python3; command -v python3 >/dev/null || py=python # Windows runners
PKG="$pkg" NAME="$name" STEM="$stem" "$py" - <<'PY'
import hashlib, json, os
from pathlib import Path
pkg, name, stem = Path(os.environ["PKG"]), os.environ["NAME"], os.environ["STEM"]
path = pkg / "extension.json"
manifest = json.loads(path.read_text(encoding="utf-8"))
sums = manifest.get("checksums") or {}
for key in (f"bin/{stem}", f"bin/{stem}.exe"):
    sums.pop(key, None)
sums[f"bin/{name}"] = "sha256:" + hashlib.sha256((pkg / "bin" / name).read_bytes()).hexdigest()
manifest["checksums"] = sums
path.write_text(json.dumps(manifest, indent=2, ensure_ascii=False), encoding="utf-8")
print(f"swap-pkg-bin: {pkg.name}/bin/{name} -> {sums[f'bin/{name}']}")
PY
