#!/usr/bin/env bash
set -euo pipefail
# Build a native CPU helper and package its runtime beside it, never into the app.
version=${1:?version required}
target=${2:?target required}
out=${3:-"$PWD"}
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-8} cargo build --locked --release -p oino-zvec --target "$target"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
bin="${CARGO_TARGET_DIR:-target}/$target/release"
exe=oino-zvec; [[ "$target" == *windows* ]] && exe=oino-zvec.exe
cp "$bin/$exe" "$stage/"
case "$target" in
  *linux*)
    lib=${ZVEC_LIB_DIR:+"$ZVEC_LIB_DIR/libzvec_c_api.so"}
    lib=${lib:-$(find "$bin/build" -path '*/zvec-prebuilt/libzvec_c_api.so' -print -quit)}
    test -n "$lib"
    cp "$lib" "$stage/"
    cp "${ORT_DYLIB_PATH:-$bin/libonnxruntime.so.1}" "$stage/libonnxruntime.so.1"
    ;;
  *apple*)
    if [[ -n "${ZVEC_LIB_DIR:-}" ]]; then cp "$ZVEC_LIB_DIR/libzvec_c_api.dylib" "$stage/"; fi
    find "$bin" \( -name 'libzvec_c_api.dylib' -o -name 'libonnxruntime*.dylib' \) -exec cp -L {} "$stage/" \;
    ;;
  *windows*)
    if [[ -n "${ZVEC_LIB_DIR:-}" ]]; then cp "$ZVEC_LIB_DIR/zvec_c_api.dll" "$stage/"; fi
    find "$bin" \( -iname 'zvec_c_api.dll' -o -iname 'onnxruntime*.dll' \) -exec cp {} "$stage/" \;
    ;;
esac
# ORT_DYLIB_PATH tells zg-engine to skip acquisition AND deployment. Include the
# verified runtime (plus DirectML and notices) explicitly when using that override.
if [[ -n "${OINO_ORT_DIR:-}" ]]; then
  cp -R "$OINO_ORT_DIR/." "$stage/"
fi
case "$target" in
  *linux*) test -s "$stage/libzvec_c_api.so" && test -s "$stage/libonnxruntime.so.1" ;;
  *apple*) test -s "$stage/libzvec_c_api.dylib" && test -s "$stage/libonnxruntime.1.dylib" ;;
  *windows*) test -s "$stage/zvec_c_api.dll" && test -s "$stage/onnxruntime.dll" && test -s "$stage/DirectML.dll" ;;
esac
mkdir -p "$out"
(cd "$stage" && tar -czf "$out/oino-zvec-$version-$target.tar.gz" *)
