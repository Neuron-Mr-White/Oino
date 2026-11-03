#!/usr/bin/env bash
# phone-sandbox.sh — Unipi Android QA on an Agent37 emulator VM, never on this PC.
#
# One agent37-android VM (Android 14 x86_64 + ARM translation, so the arm64 release APK runs) holds both
# the phone and a throwaway PC host (static unipi-oino-host + bundled extensions from a .deb) with
# isolated HOME / OINO_AGENT_DIR / OINO_HOST_DIR under /root/qa, a fake OpenAI-compatible model
# (phone-fakellm.py) and no owner secrets. The phone pairs over iroh by manual link entry (no camera).
#
#   scripts/qa/phone-sandbox.sh up                 create the VM (auto-sleep off, emulated cameras), wait for Android
#   scripts/qa/phone-sandbox.sh install APK DEB    host + extensions + fake model + APK + current WebView
#   scripts/qa/phone-sandbox.sh host               (re)start the fake model and the host
#   scripts/qa/phone-sandbox.sh cdp                restart the app, forward its WebView devtools to localhost:9333
#   scripts/qa/phone-sandbox.sh pair               pair the app with the VM host (types the link into Add a PC)
#   scripts/qa/phone-sandbox.sh shot NAME [DIR]    1080x2400 PNG (CDP fallback when the emulator frame is black)
#   scripts/qa/phone-sandbox.sh tap 'Text|aria|css' [nth]   real touch on an element's centre
#   scripts/qa/phone-sandbox.sh measure            targets < 44 px, font sizes, tiny text, horizontal overflow
#   scripts/qa/phone-sandbox.sh crash              Add a PC → Pair with QR → print `logcat -b crash`
#   scripts/qa/phone-sandbox.sh adb ARGS… | ssh CMD… | url (live screen) | status
#   scripts/qa/phone-sandbox.sh down               DELETE the VM (always run this; also on failure)
#
# State: ~/.cache/oino-phone-sandbox/id. Key: AGENT37_API_KEY from ~/.bashrc (never printed).
# Costs about $0.10/h (4 vCPU / 16 GB VM). Defaults: APK ~/oino-qa-release/v9/UniPi-Dev_arm64-*.apk,
# DEB ~/oino-qa-release/v9/unipi-oino_amd64-*.deb, faux fixture from ~/.cache/oino-qa-build-wt.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
ST=${PHONE_SANDBOX_STATE:-$HOME/.cache/oino-phone-sandbox}
mkdir -p "$ST"
KEY="${AGENT37_API_KEY:-$(grep -o 'AGENT37_API_KEY=[^ ]*' ~/.bashrc | head -1 | cut -d= -f2- | tr -d "\"'")}"
export AGENT37_API_KEY="$KEY"
API=https://api.agent37.com/v1
PKG=${UNIPI_ANDROID_PKG:-dev.unipi.oino.dev}
CDP_PORT=${CDP_PORT:-9333}
SHOTS=${PHONE_SHOTS:-$HERE/../../docs/dev/v2/design/shots/phone-audit}
id() { cat "$ST/id" 2>/dev/null || { echo "no VM: run up" >&2; exit 1; }; }
ssh_vm() {
  ssh -F /dev/null -o "ProxyCommand=npx -y agent37@latest tunnel %h" -o StrictHostKeyChecking=no \
    -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ServerAliveInterval=60 \
    -o IdentityFile="$HOME/.ssh/id_ed25519" "root@$(id).agent37.app" "$@"
}
scp_vm() {
  scp -F /dev/null -o "ProxyCommand=npx -y agent37@latest tunnel %h" -o StrictHostKeyChecking=no \
    -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o IdentityFile="$HOME/.ssh/id_ed25519" "$@"
}
adb_vm() { ssh_vm "PATH=\$PATH:/opt/android-sdk/platform-tools adb $*"; }
cdp() { CDP_PORT=$CDP_PORT node "$HERE/phone-cdp.mjs" "$@"; }
forward() {  # the app's WebView devtools socket (name changes with the pid) -> localhost:$CDP_PORT
  local sock; sock=$(adb_vm shell cat /proc/net/unix | grep -a -o 'webview_devtools_remote_[0-9]*' | tail -1)
  adb_vm forward tcp:9222 "localabstract:$sock" >/dev/null
  if ! curl -s "localhost:$CDP_PORT/json" >/dev/null; then
    (setsid ssh -F /dev/null -o "ProxyCommand=npx -y agent37@latest tunnel %h" -o StrictHostKeyChecking=no \
      -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o IdentityFile="$HOME/.ssh/id_ed25519" \
      -N -L "$CDP_PORT:127.0.0.1:9222" "root@$(id).agent37.app" >"$ST/tunnel.log" 2>&1 &)
    sleep 5
  fi
  curl -s "localhost:$CDP_PORT/json" | grep -o '"url": "[^"]*"'
}

cmd=${1:-help}; shift || true
case "$cmd" in
up)
  [ -f "$ST/id" ] && { echo "VM $(id) already recorded; run down first" >&2; exit 1; }
  body='{"template":"agent37-android","type":"vm","name":"oino-phone-qa","auto_sleep":false,
         "resources":{"cpu":4,"memory":16,"disk":30},
         "env":{"ANDROID_EMULATOR_ARGS":"-camera-back emulated -camera-front emulated"}}'
  new=$(curl -s -X POST -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' -d "$body" $API/instances |
    python3 -c 'import json,sys;d=json.load(sys.stdin);print(d.get("id") or "");print(d,file=sys.stderr) if not d.get("id") else None')
  [ -n "$new" ] || exit 1
  echo "$new" > "$ST/id"; echo "VM $new (delete with: $0 down)"
  until ~/.local/bin/a37 exec "$new" 'android-wait 600 && adb shell getprop ro.product.cpu.abilist' 2>/dev/null; do sleep 10; done
  # the template passes -camera-* none first; our env flags come later and win
  ~/.local/bin/a37 exec "$new" 'ps aux | grep -o "camera-back [a-z]*" | sort -u' || true ;;
install)
  APK=${1:-$(ls ~/oino-qa-release/v9/UniPi-Dev_arm64-*.apk | head -1)}
  DEB=${2:-$(ls ~/oino-qa-release/v9/unipi-oino_amd64-*.deb | head -1)}
  FAUX=${FAUX:-$HOME/.cache/oino-qa-build-wt/extensions/dist/oino.fixture.provider-faux}
  T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
  dpkg-deb -x "$DEB" "$T/deb"
  mkdir -p "$T/up/bin" "$T/up/share/oino/extensions"
  cp "$T/deb/usr/bin/unipi-oino-host" "$T/deb/usr/bin/oino" "$T/up/bin/"
  cp -r "$T/deb/usr/share/oino/extensions/." "$T/up/share/oino/extensions/"
  [ -d "$FAUX" ] && cp -r "$FAUX" "$T/up/share/oino/extensions/"
  cp "$APK" "$T/up/app.apk"; cp "$HERE/phone-fakellm.py" "$T/up/fakellm.py"
  tar czf "$T/up.tgz" -C "$T/up" .
  scp_vm "$T/up.tgz" "root@$(id).agent37.app:/root/"
  ssh_vm 'bash -s' <<'VM'
set -e
Q=/root/qa; mkdir -p $Q && tar xzf /root/up.tgz -C $Q && rm /root/up.tgz
mkdir -p $Q/agent $Q/hostdir $Q/home/projects/demo-app/src $Q/home/projects/website
command -v git >/dev/null || (apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq git >/dev/null)
cd $Q/home/projects/demo-app
printf '[package]\nname = "demo-app"\nversion = "0.1.0"\nedition = "2021"\n' > Cargo.toml
printf 'fn main() {\n    println!("Hello");\n}\n' > src/main.rs
printf '# Demo app\n\nA tiny project for phone QA.\n' > README.md
python3 -c "
import zlib,struct
w,h=320,200
raw=b''.join(b'\x00'+b''.join(bytes([x*255//w,y*255//h,140]) for x in range(w)) for y in range(h))
ch=lambda t,d: struct.pack('>I',len(d))+t+d+struct.pack('>I',zlib.crc32(t+d)&0xffffffff)
open('shot.png','wb').write(b'\x89PNG\r\n\x1a\n'+ch(b'IHDR',struct.pack('>IIBBBBB',w,h,8,2,0,0,0))+ch(b'IDAT',zlib.compress(raw))+ch(b'IEND',b''))"
[ -d .git ] || { git init -q && git add -A && git -c user.email=qa@x -c user.name=qa commit -qm init; echo '// todo' >> src/main.rs; }
echo '{"providers":{"ollama":{"baseUrl":"http://localhost:11434/v1","api":"openai-completions","apiKey":"ollama","models":[{"id":"fake-1","name":"Fake QA model","contextWindow":128000,"maxTokens":8192,"input":["text","image"]}]}}}' > $Q/agent/models.json
echo '{"defaultProvider":"ollama","defaultModel":"fake-1"}' > $Q/agent/settings.json
echo '{"faux":{"type":"api_key","key":"k"}}' > $Q/agent/auth.json
export PATH=$PATH:/opt/android-sdk/platform-tools
adb install -r $Q/app.apk
# The image ships WebView 113, which fails to repaint some layers (e.g. the Files editor). Use a current one.
if ! adb shell dumpsys webviewupdate | grep -q 'com.android.webview, 1[2-9][0-9]'; then
  REV=$(curl -s https://storage.googleapis.com/chromium-browser-snapshots/AndroidDesktop_x64/LAST_CHANGE)
  curl -s -o /root/cad.zip "https://storage.googleapis.com/chromium-browser-snapshots/AndroidDesktop_x64/$REV/chrome-android-desktop.zip"
  unzip -o -q /root/cad.zip chrome-android-desktop/apks/SystemWebView.apk -d /root && rm /root/cad.zip
  adb install -r /root/chrome-android-desktop/apks/SystemWebView.apk
  adb shell cmd webviewupdate set-webview-implementation com.android.webview
fi
adb shell dumpsys webviewupdate | grep 'Current WebView'
VM
  "$0" host ;;
host)
  ssh_vm 'bash -s' <<'VM'
Q=/root/qa
export HOME=$Q/home OINO_AGENT_DIR=$Q/agent OINO_HOST_DIR=$Q/hostdir RUST_LOG=info
pkill -f fakellm.py; pkill -f 'unipi-oino-host serve'; sleep 1
setsid nohup python3 $Q/fakellm.py >$Q/fakellm.out 2>&1 < /dev/null &
setsid nohup $Q/bin/unipi-oino-host serve --mode manual >$Q/host.log 2>&1 < /dev/null &
sleep 6; $Q/bin/unipi-oino-host status; $Q/bin/oino --list-models 2>&1 | head -4
VM
  ;;
cdp)
  adb_vm shell am force-stop $PKG
  adb_vm shell monkey -p $PKG -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1
  sleep "${1:-12}"; forward ;;
pair)
  # (fed on stdin: a pkill -f pattern on the ssh command line would match the ssh session itself)
  line=$(ssh_vm 'bash -s' <<'VM'
Q=/root/qa; export HOME=$Q/home OINO_AGENT_DIR=$Q/agent OINO_HOST_DIR=$Q/hostdir
pkill -f "unipi-oino-host pair"; pkill -f "sleep 900"; rm -f $Q/pair.out
setsid nohup sh -c "sleep 900 | $Q/bin/unipi-oino-host pair --json --ttl 15m" > $Q/pair.out 2>&1 < /dev/null &
for i in $(seq 1 20); do grep -q link $Q/pair.out 2>/dev/null && break; sleep 1; done; head -1 $Q/pair.out
VM
)
  link=$(python3 -c 'import json,sys;print(json.loads(sys.argv[1])["link"])' "$line")
  # Cold-start the app on the deep link (works on a current WebView; `install` provides one), then confirm.
  adb_vm shell am force-stop $PKG
  adb_vm shell "\"am start -W -a android.intent.action.VIEW -d '$link' $PKG\"" >/dev/null
  sleep 12; forward >/dev/null
  "$0" tap 'Pair'; sleep 6
  # Fallback by hand: Add a PC -> Paste pairing link -> type "<t> <c>" (adb: "<t>%s<c>") -> Enter -> Pair.
  ssh_vm 'tail -1 /root/qa/pair.out' ;;
shot)
  mkdir -p "${2:-$SHOTS}"; f="${2:-$SHOTS}/$1.png"
  for i in 1 2 3; do
    adb_vm exec-out screencap -p > "$f"
    python3 -c "import sys
from PIL import Image
im=Image.open('$f').convert('L'); w,h=im.size; im=im.crop((0,150,w,h-150))
sys.exit(0 if sum(im.histogram()[40:])>3000 else 1)" && { echo "$f"; exit 0; }
    sleep 1.5
  done
  cdp shot "$f" >/dev/null; echo "$f (CDP capture: the emulator presented a black frame)" ;;
tap)
  p=$(cdp where "$1" "${2:-0}"); [ "$p" = NOTFOUND ] && { echo "not found: $1" >&2; exit 1; }
  adb_vm shell input tap $p; echo "tap $1 @ $p" ;;
measure) cdp measure ;;
crash)
  adb_vm logcat -b crash -c
  cdp eval "location.hash='/settings'" >/dev/null; sleep 2   # Add a PC lives in Settings › Hosts once paired
  "$0" tap 'Add a PC'; sleep 2; "$0" tap 'Pair with QR'; sleep 3
  adb_vm shell pm grant $PKG android.permission.CAMERA 2>/dev/null || true
  sleep 8; adb_vm logcat -b crash -d ;;
adb) adb_vm "$@" ;;
ssh) ssh_vm "$@" ;;
url)
  curl -s -X POST -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' \
    -d '{"port":6901,"ttl_seconds":3600}' "$API/instances/$(id)/signed-url" |
    python3 -c 'import json,sys;u=json.load(sys.stdin)["url"];print(u.replace("/?","/vnc.html?",1)+"&autoconnect=1&resize=scale")' ;;
status) curl -s -H "Authorization: Bearer $KEY" "$API/instances/$(id)" | python3 -c 'import json,sys;d=json.load(sys.stdin);print(d["id"],d["status"],d["resources"])' ;;
down)
  i=$(id); pkill -f "$CDP_PORT:127.0.0.1:9222" 2>/dev/null || true
  curl -s -X DELETE -H "Authorization: Bearer $KEY" "$API/instances/$i"; echo; rm -f "$ST/id"; echo "deleted $i" ;;
*) sed -n 2,22p "$0" ;;
esac
