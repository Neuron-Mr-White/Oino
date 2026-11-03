#!/usr/bin/env bash
# Bench VM setup (Ubuntu agent37-ubuntu image, run as root in /root by remote.sh):
# toolchain, profilers, Xvfb + WebKitWebDriver + tauri-driver, then the HEAD .deb
# (built here unless --deb FILE is given) at /root/unipi.deb, plus a glibc host with
# line tables at /root/unipi-oino-host-gnu for heaptrack (it cannot preload into the
# static musl host the .deb ships).
set -euxo pipefail
DEB=""; [ "${1:-}" = --deb ] && DEB=$2
export DEBIAN_FRONTEND=noninteractive
apt-get update -q
apt-get install -y -q build-essential curl wget file git pkg-config libssl-dev libwebkit2gtk-4.1-dev \
  libxdo-dev libayatana-appindicator3-dev librsvg2-dev musl-tools python3-pip xz-utils >/dev/null
apt-get install -y -q heaptrack linux-tools-common linux-tools-generic "linux-tools-$(uname -r)" xvfb \
  webkitgtk-webdriver sysstat jq >/dev/null
cd /root
export CARGO_HOME=/root/.cargo RUSTUP_HOME=/root/.rustup
[ -x /root/.cargo/bin/cargo ] || curl -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.99.0 --profile minimal
. /root/.cargo/env
rustup target add x86_64-unknown-linux-musl wasm32-wasip2
[ -x /root/node/bin/node ] || { curl -sSL https://nodejs.org/dist/v24.22.1/node-v24.22.1-linux-x64.tar.xz | tar -xJ && mv node-v24.22.1-linux-x64 node; }
ln -sf /root/node/bin/node /usr/local/bin/node
pip3 install --break-system-packages -q ziglang==0.13.0 cargo-zigbuild
mkdir -p /root/.local/bin && ln -sf "$(command -v WebKitWebDriver)" /root/.local/bin/WebKitWebDriver
[ -d oino ] || git clone -q oino.bundle oino
cd oino
command -v tauri-driver >/dev/null || cargo install tauri-driver --locked
if [ -n "$DEB" ]; then
  cp "$DEB" /root/unipi.deb
else
  export PATH=/root/node/bin:$PATH
  cargo xtask wasm
  cargo install tauri-cli --version '^2.12' --locked
  UNIPI_RELEASE_IDENTITY=1 bash scripts/unipi-build.sh
  cp apps/unipi/src-tauri/target/release/bundle/deb/*.deb /root/unipi.deb
fi
(cd apps/unipi && CARGO_PROFILE_RELEASE_DEBUG=line-tables-only CARGO_PROFILE_RELEASE_STRIP=none \
  CARGO_TARGET_DIR=/root/tgt-gnu cargo build --release -p unipi-host) &&
  cp /root/tgt-gnu/release/unipi-host /root/unipi-oino-host-gnu || echo "WARN: no glibc host (heaptrack unavailable)"
echo 0 > /proc/sys/kernel/yama/ptrace_scope || true  # heaptrack -p (runtime attach)
git clone -q --depth 1 https://github.com/brendangregg/FlameGraph /root/FlameGraph 2>/dev/null || true
echo VM-SETUP-DONE
