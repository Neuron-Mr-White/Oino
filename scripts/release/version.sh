#!/usr/bin/env sh
# Read, verify or bump the UniPi Oino distribution version across every
# manifest and lockfile that release.yml and scripts/unipi-build.sh ship.
#
# Usage:
#   scripts/release/version.sh [--check] [--tag vX.Y.Z] [--site <dir>] [--root <dir>]
#   scripts/release/version.sh <X.Y.Z> [--site <dir>] [--root <dir>]
#
# Checked / bumped files (under --root, default repo root):
#   apps/unipi/src-tauri/tauri.conf.json       "version"
#   apps/unipi/src-tauri/Cargo.toml            [package] unipi-mobile version
#   apps/unipi/src-tauri/Cargo.lock            [[package]] unipi-mobile version
#   apps/unipi/crates/unipi-host/Cargo.toml    [package] unipi-host version
#   apps/unipi/Cargo.lock                      [[package]] unipi-host version
#   apps/unipi/package.json                    "version"
#   apps/unipi/package-lock.json               "version" + packages[""].version
#
# With --site <oino-landing-page>:
#   --check verifies <site>/src/data/release.json matches the manifest version;
#   <X.Y.Z> runs `node <site>/scripts/fill-release.mjs --bump <X.Y.Z>`.
set -eu

root="$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)"
mode="check"
target=""
tag=""
site=""

while [ $# -gt 0 ]; do
  case "$1" in
    --check) mode="check"; shift ;;
    --tag) tag=${2:?--tag needs vX.Y.Z}; shift 2 ;;
    --site) site=${2:?--site needs a path}; shift 2 ;;
    --root) root=${2:?--root needs a path}; shift 2 ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    -*) printf 'version.sh: unknown option %s\n' "$1" >&2; exit 2 ;;
    *)
      [ -z "$target" ] || { printf 'version.sh: unexpected extra argument %s\n' "$1" >&2; exit 2; }
      mode="bump"
      target="$1"
      shift
      ;;
  esac
done

VERSION_OUT=$(ROOT="$root" MODE="$mode" TARGET="$target" TAG="$tag" SITE="$site" python3 - <<'PY'
import json
import os
import pathlib
import re
import sys

root = pathlib.Path(os.environ["ROOT"]).resolve()
mode = os.environ["MODE"]
target = os.environ["TARGET"].removeprefix("v")
tag = os.environ["TAG"]
site = os.environ["SITE"]

SEMVER = re.compile(r"^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$")

FILES = {
    "tauri_conf": root / "apps/unipi/src-tauri/tauri.conf.json",
    "tauri_toml": root / "apps/unipi/src-tauri/Cargo.toml",
    "tauri_lock": root / "apps/unipi/src-tauri/Cargo.lock",
    "host_toml": root / "apps/unipi/crates/unipi-host/Cargo.toml",
    "host_lock": root / "apps/unipi/Cargo.lock",
    "pkg_json": root / "apps/unipi/package.json",
    "pkg_lock": root / "apps/unipi/package-lock.json",
}


def rel(p: pathlib.Path) -> str:
    try:
        return str(p.relative_to(root))
    except ValueError:
        return str(p)


def toml_pkg_version(text: str, pkg_name: str, header: str) -> str:
    pat = re.compile(
        rf"(?ms)^({re.escape(header)}\n(?:(?!\[).)*?^name = \"{re.escape(pkg_name)}\"\n(?:(?!\[).)*?^version = \")([^\"]+)(\")"
    )
    m = pat.search(text)
    if not m:
        sys.exit(f"version.sh: could not find {header} {pkg_name} version")
    return m.group(2)


def replace_toml_pkg_version(text: str, pkg_name: str, header: str, new_v: str) -> str:
    pat = re.compile(
        rf"(?ms)^({re.escape(header)}\n(?:(?!\[).)*?^name = \"{re.escape(pkg_name)}\"\n(?:(?!\[).)*?^version = \")([^\"]+)(\")"
    )
    out, n = pat.subn(rf"\g<1>{new_v}\g<3>", text, count=1)
    if n != 1:
        sys.exit(f"version.sh: failed updating {header} {pkg_name} version")
    return out


def replace_top_json_version(text: str, new_v: str, label: str) -> str:
    out, n = re.subn(r'("version"\s*:\s*")([^"]+)(")', rf"\g<1>{new_v}\g<3>", text, count=1)
    if n != 1:
        sys.exit(f"version.sh: failed updating version in {label}")
    return out


def replace_pkg_lock_version(text: str, new_v: str) -> str:
    # Top-level "version" and packages[""]."version".
    out, n1 = re.subn(r'^(\s*"version"\s*:\s*")([^"]+)(")', rf"\g<1>{new_v}\g<3>", text, count=1, flags=re.M)
    out, n2 = re.subn(
        r'(""\s*:\s*\{\s*\n\s*"name"\s*:\s*"[^"]+"\s*,\s*\n\s*"version"\s*:\s*")([^"]+)(")',
        rf"\g<1>{new_v}\g<3>",
        out,
        count=1,
    )
    if n1 != 1 or n2 != 1:
        sys.exit("version.sh: failed updating apps/unipi/package-lock.json")
    return out


def read_all() -> dict[str, str]:
    for p in FILES.values():
        if not p.is_file():
            sys.exit(f"version.sh: missing {rel(p)}")
    pkg_lock = json.loads(FILES["pkg_lock"].read_text("utf-8"))
    return {
        rel(FILES["tauri_conf"]): json.loads(FILES["tauri_conf"].read_text("utf-8")).get("version", ""),
        rel(FILES["tauri_toml"]): toml_pkg_version(FILES["tauri_toml"].read_text("utf-8"), "unipi-mobile", "[package]"),
        rel(FILES["tauri_lock"]): toml_pkg_version(FILES["tauri_lock"].read_text("utf-8"), "unipi-mobile", "[[package]]"),
        rel(FILES["host_toml"]): toml_pkg_version(FILES["host_toml"].read_text("utf-8"), "unipi-host", "[package]"),
        rel(FILES["host_lock"]): toml_pkg_version(FILES["host_lock"].read_text("utf-8"), "unipi-host", "[[package]]"),
        rel(FILES["pkg_json"]): json.loads(FILES["pkg_json"].read_text("utf-8")).get("version", ""),
        f"{rel(FILES['pkg_lock'])}#version": pkg_lock.get("version", ""),
        f"{rel(FILES['pkg_lock'])}#packages['']": pkg_lock.get("packages", {}).get("", {}).get("version", ""),
    }


if mode == "bump":
    if not SEMVER.fullmatch(target):
        sys.exit(f"version.sh: invalid semver {target!r} (expected X.Y.Z)")
    FILES["tauri_conf"].write_text(
        replace_top_json_version(FILES["tauri_conf"].read_text("utf-8"), target, rel(FILES["tauri_conf"])), "utf-8"
    )
    FILES["tauri_toml"].write_text(
        replace_toml_pkg_version(FILES["tauri_toml"].read_text("utf-8"), "unipi-mobile", "[package]", target), "utf-8"
    )
    FILES["tauri_lock"].write_text(
        replace_toml_pkg_version(FILES["tauri_lock"].read_text("utf-8"), "unipi-mobile", "[[package]]", target), "utf-8"
    )
    FILES["host_toml"].write_text(
        replace_toml_pkg_version(FILES["host_toml"].read_text("utf-8"), "unipi-host", "[package]", target), "utf-8"
    )
    FILES["host_lock"].write_text(
        replace_toml_pkg_version(FILES["host_lock"].read_text("utf-8"), "unipi-host", "[[package]]", target), "utf-8"
    )
    FILES["pkg_json"].write_text(
        replace_top_json_version(FILES["pkg_json"].read_text("utf-8"), target, rel(FILES["pkg_json"])), "utf-8"
    )
    FILES["pkg_lock"].write_text(replace_pkg_lock_version(FILES["pkg_lock"].read_text("utf-8"), target), "utf-8")
    print(f"bumped UniPi Oino manifests to {target}", file=sys.stderr)

versions = read_all()
canonical = versions[rel(FILES["tauri_conf"])]
if not SEMVER.fullmatch(canonical):
    sys.exit(f"version.sh: {rel(FILES['tauri_conf'])} version {canonical!r} is not valid semver")

bad = [f"  {k} = {v!r} (expected {canonical!r})" for k, v in versions.items() if v != canonical]
if bad:
    sys.exit(
        "version.sh: UniPi Oino version mismatch across manifests:\n"
        + "\n".join(bad)
        + f"\nrun: scripts/release/version.sh {canonical}"
    )

if tag:
    if not tag.startswith("v"):
        sys.exit(f"version.sh: --tag {tag!r} must start with 'v' (e.g. v{canonical})")
    want_v = tag[1:]
    if want_v != canonical:
        sys.exit(
            f"version.sh: git tag {tag} disagrees with manifest version {canonical} "
            f"(run `scripts/release/version.sh {want_v}` before tagging)"
        )

if mode == "check" and site:
    site_rel = pathlib.Path(site).resolve() / "src/data/release.json"
    if not site_rel.is_file():
        sys.exit(f"version.sh: missing {site_rel}")
    site_v = json.loads(site_rel.read_text("utf-8")).get("version", "")
    if site_v != canonical:
        sys.exit(
            f"version.sh: website {site_rel} is at {site_v!r}, manifests are at {canonical!r} "
            f"(run `node scripts/fill-release.mjs` in {site})"
        )

print(canonical)
PY
)

if [ "$mode" = "bump" ] && [ -n "$site" ]; then
  (cd "$site" && node scripts/fill-release.mjs --bump "$VERSION_OUT") >&2
fi

printf '%s\n' "$VERSION_OUT"
