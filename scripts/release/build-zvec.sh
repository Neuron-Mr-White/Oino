#!/usr/bin/env bash
set -euo pipefail
# Build a native CPU helper and package its runtime beside it, never into the app.
version=${1:?version required}
target=${2:?target required}
out=${3:-"$PWD"}
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-8} cargo build --release -p oino-zvec --target "$target"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
bin="target/$target/release"
exe=oino-zvec; [[ "$target" == *windows* ]] && exe=oino-zvec.exe
cp "$bin/$exe" "$stage/"
case "$target" in
  *linux*)
    lib=$(find "$bin/build" -path '*/zvec-prebuilt/libzvec_c_api.so' -print -quit)
    test -n "$lib"
    cp "$lib" "$stage/"
    cp "$bin/libonnxruntime.so.1" "$stage/"
    ;;
  *apple*)
    find "$bin" \( -name 'libzvec_c_api.dylib' -o -name 'libonnxruntime*.dylib' \) -exec cp -L {} "$stage/" \;
    ;;
  *windows*)
    find "$bin" \( -iname 'zvec_c_api.dll' -o -iname 'onnxruntime*.dll' \) -exec cp {} "$stage/" \;
    ;;
esac
mkdir -p "$out"
(cd "$stage" && tar -czf "$out/oino-zvec-$version-$target.tar.gz" *)
