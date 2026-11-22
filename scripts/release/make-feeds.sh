#!/usr/bin/env bash
# Build the two app update feeds from a release's assets:
#
#   <out>/app/desktop/latest.json   Tauri v2 updater static JSON, polled by the
#                                   desktop app (tauri.conf.json plugins.updater
#                                   endpoints = https://oino.dev/app/desktop/latest.json)
#   <out>/app/android/latest.json   the Android feed (apps/unipi/src/lib/update.ts
#                                   parseFeed + UpdateBridge.kt), polled from
#                                   https://oino.dev/app/android/latest.json
#
# Usage:
#   scripts/release/make-feeds.sh --dir <assets dir> [--tag vX.Y.Z] [--out <dir>]
#   scripts/release/make-feeds.sh --tag vX.Y.Z [--out <dir>]   # fetches the small
#                                   assets (*.sig, SHA256SUMS) with `gh release download`
#
# Inputs (file names as release.yml uploads them):
#   UniPi_<v>_amd64.AppImage(.sig)          linux-x86_64
#   UniPi_<v>_x64-setup.exe(.sig)           windows-x86_64
#   UniPi_<v>_universal.app.tar.gz(.sig)    darwin-aarch64 + darwin-x86_64
#   UniPi_<v>_universal.apk                 Android (sha256 from SHA256SUMS or the file)
#   SHA256SUMS
#
# Desktop update URLs point at the GitHub release (the Tauri updater follows
# the redirect and checks the minisign signature, so the host does not matter).
# The Android APK URL must be on https://oino.dev/ (both update.ts and
# UpdateBridge.kt allow only that host), so the site must ALSO mirror the APK at
# $APK_BASE/UniPi_<v>_universal.apk; the script prints that list.
#
# Environment:
#   REPO       GitHub repo (default Neuron-Mr-White/Oino)
#   APK_BASE   where the site mirrors the APK (default https://oino.dev/download)
#   NOTES      release notes for both feeds (default "UniPi Oino <v>")
#   PUB_DATE   RFC 3339 date (default now, UTC)
#   STRICT     1 (default) = every desktop platform and the APK must be present
#              and every .sig must verify against tauri.conf.json's pubkey;
#              0 = skip missing platforms with a warning (rehearsals)
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
dir="" tag="" out="$root/target/feeds"
while [ $# -gt 0 ]; do
  case "$1" in
    --dir) dir=${2:?--dir needs a path}; shift 2 ;;
    --tag) tag=${2:?--tag needs vX.Y.Z}; shift 2 ;;
    --out) out=${2:?--out needs a path}; shift 2 ;;
    -h|--help) sed -n '2,36p' "$0"; exit 0 ;;
    *) echo "make-feeds: unknown argument $1" >&2; exit 2 ;;
  esac
done
[ -n "$dir" ] || [ -n "$tag" ] || { echo "make-feeds: need --dir or --tag" >&2; exit 2; }

export REPO="${REPO:-Neuron-Mr-White/Oino}"
export APK_BASE="${APK_BASE:-https://oino.dev/download}"
export STRICT="${STRICT:-1}"
case "$APK_BASE" in
  https://oino.dev/*) ;;
  *) echo "make-feeds: APK_BASE must be on https://oino.dev/ (the app refuses other hosts)" >&2; exit 2 ;;
esac

tmp=""
if [ -z "$dir" ]; then
  tmp=$(mktemp -d "${TMPDIR:-/tmp}/make-feeds.XXXXXX")
  trap 'rm -rf "$tmp"' EXIT
  # Only the small files: signatures + checksums (sha256 of the APK comes from SHA256SUMS).
  gh release download "$tag" -R "$REPO" -D "$tmp" -p '*.sig' -p 'SHA256SUMS'
  dir=$tmp
fi
if [ -z "$tag" ]; then
  tag=$(ls "$dir" | sed -n 's/^UniPi_\([0-9][^_]*\)_.*/v\1/p' | head -1)
  [ -n "$tag" ] || { echo "make-feeds: no UniPi_<version>_* asset in $dir; pass --tag" >&2; exit 2; }
fi

export DIR="$dir" TAG="$tag" OUT="$out" CONF="$root/apps/unipi/src-tauri/tauri.conf.json"
python3 - <<'PY'
import base64, datetime, hashlib, json, os, re, sys
from pathlib import Path

d, tag, out = Path(os.environ["DIR"]), os.environ["TAG"], Path(os.environ["OUT"])
strict = os.environ["STRICT"] == "1"
version = tag.removeprefix("v")
if not re.fullmatch(r"\d+\.\d+\.\d+", version.split("-")[0].split("+")[0]):
    sys.exit(f"make-feeds: bad version {version!r}")
base = f"https://github.com/{os.environ['REPO']}/releases/download/{tag}"
notes = os.environ.get("NOTES") or f"UniPi Oino {version}"
pub_date = os.environ.get("PUB_DATE") or datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
problems = []

def problem(msg):
    problems.append(msg)
    print(("error: " if strict else "warning: ") + msg, file=sys.stderr)

# ── signature check against the pubkey the shipped apps carry ──────────────
pub = None
try:
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
    key_b64 = json.loads(Path(os.environ["CONF"]).read_text())["plugins"]["updater"]["pubkey"]
    raw = base64.b64decode(base64.b64decode(key_b64).decode().splitlines()[1])
    pub = (raw[2:10], Ed25519PublicKey.from_public_bytes(raw[10:]))
except ImportError:
    problem("python3-cryptography is missing; cannot verify the .sig files (pip install cryptography)")

def verify_sig(sig_text, asset):
    """minisign: line 2 = alg(2) keyid(8) sig(64); line 4 = global sig over sig+trusted comment."""
    if pub is None:
        return True
    lines = base64.b64decode(sig_text).decode().splitlines()
    blob = base64.b64decode(lines[1])
    if blob[2:10] != pub[0]:
        problem(f"{asset.name}.sig is signed by key {blob[2:10][::-1].hex().upper()}, not tauri.conf.json's pubkey")
        return False
    if not asset.exists():
        return True  # --tag mode downloads only the .sig; key id matched above
    data = asset.read_bytes()
    msg = hashlib.blake2b(data).digest() if blob[:2] == b"ED" else data
    try:
        pub[1].verify(blob[10:], msg)
        trusted = lines[2].split("trusted comment: ", 1)[1].encode()
        pub[1].verify(base64.b64decode(lines[3]), blob[10:] + trusted)
    except Exception:
        problem(f"{asset.name}.sig does not verify")
        return False
    return True

# ── desktop: Tauri v2 static JSON ──────────────────────────────────────────
platforms = {}
for keys, name in [
    (["linux-x86_64"], f"UniPi_{version}_amd64.AppImage"),
    (["windows-x86_64"], f"UniPi_{version}_x64-setup.exe"),
    (["darwin-aarch64", "darwin-x86_64"], f"UniPi_{version}_universal.app.tar.gz"),
]:
    sig = d / f"{name}.sig"
    if not sig.exists():
        problem(f"missing {sig.name} (desktop {', '.join(keys)})")
        continue
    text = sig.read_text().strip()
    if not verify_sig(text, d / name):
        continue  # a bad signature would make the updater refuse the platform
    for k in keys:
        platforms[k] = {"signature": text, "url": f"{base}/{name}"}
if not platforms:
    problem("no desktop platform has a signature; refusing an empty feed")

# ── android ───────────────────────────────────────────────────────────────
sums = {}
if (d / "SHA256SUMS").exists():
    for line in (d / "SHA256SUMS").read_text().splitlines():
        parts = line.split()
        if len(parts) == 2:
            sums[parts[1].lstrip("*")] = parts[0].lower()
apk = f"UniPi_{version}_universal.apk"
android = None
if any(n.endswith("-unsigned-debug.apk") for n in list(sums) + [p.name for p in d.iterdir()]):
    problem("the release carries a debug-signed APK; Android cannot update over it")
sha = sums.get(apk)
size = None
if (d / apk).exists():
    data = (d / apk).read_bytes()
    size = len(data)
    got = hashlib.sha256(data).hexdigest()
    if sha and sha != got:
        problem(f"{apk}: SHA256SUMS says {sha}, the file is {got}")
    sha = got
if not sha:
    problem(f"missing {apk} (and no SHA256SUMS entry)")
else:
    core = [int(x) for x in version.split("-")[0].split("+")[0].split(".")]
    # Tauri's versionCode: major*1000000 + minor*1000 + patch (gen/android tauri.properties)
    build = core[0] * 1_000_000 + core[1] * 1_000 + core[2]
    app = {"version": version, "build": build, "url": f"{os.environ['APK_BASE'].rstrip('/')}/{apk}",
           "sha256": sha, "notes": notes, "date": pub_date}
    if size is not None:
        app["size"] = size
    android = {"app": app, "host": {"version": version}}

if strict and problems:
    sys.exit(f"make-feeds: {len(problems)} problem(s); nothing written (STRICT=0 to skip)")

(out / "app/desktop").mkdir(parents=True, exist_ok=True)
(out / "app/desktop/latest.json").write_text(json.dumps(
    {"version": version, "notes": notes, "pub_date": pub_date, "platforms": platforms}, indent=2) + "\n")
print(f"wrote {out}/app/desktop/latest.json ({', '.join(platforms) or 'no platforms'})")
if android:
    (out / "app/android").mkdir(parents=True, exist_ok=True)
    (out / "app/android/latest.json").write_text(json.dumps(android, indent=2) + "\n")
    print(f"wrote {out}/app/android/latest.json (build {android['app']['build']})")
    print("site must publish:")
    print(f"  {out}/app/desktop/latest.json  -> https://oino.dev/app/desktop/latest.json")
    print(f"  {out}/app/android/latest.json  -> https://oino.dev/app/android/latest.json")
    print(f"  {base}/{apk}  -> {android['app']['url']}  (same bytes, sha256 {sha})")
PY
