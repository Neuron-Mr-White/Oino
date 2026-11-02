#!/usr/bin/env bash
# Runs the perf bench on a fresh Agent37 VM (benches never run on the owner's PC).
#
#   scripts/perf/remote.sh [--deb built.deb] [--keep] -- <run.sh options>
#
# Creates an agent37-ubuntu VM (4 vCPU / 32 GB / 80 GB, auto-sleep OFF), installs the
# toolchain + profilers, builds HEAD's .deb there (unless --deb is given), runs
# scripts/perf/run.sh, copies perf-out/ back to target/perf-out/<vm>/ (under the target GC budget) and deletes the VM
# (always, via trap; --keep leaves it for debugging — delete it with `a37 rm <id>`).
# Needs AGENT37_API_KEY in ~/.bashrc, npx (for the agent37 tunnel) and ~/.ssh/id_ed25519.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
DEB="" KEEP=0
while [ $# -gt 0 ]; do
  case "$1" in --deb) DEB=$2; shift 2 ;; --keep) KEEP=1; shift ;; --) shift; break ;; *) break ;; esac
done
K=$(grep -o 'AGENT37_API_KEY=[^ ]*' ~/.bashrc | head -1 | cut -d= -f2- | tr -d "\"'")
[ -n "$K" ] || { echo "remote: AGENT37_API_KEY missing" >&2; exit 64; }
ID=$(curl -s -X POST -H "Authorization: Bearer $K" -H 'Content-Type: application/json' \
  -d '{"template":"agent37-ubuntu","type":"vm","name":"oino-perf-bench","auto_sleep":false,"resources":{"cpu":4,"memory":32,"disk":80}}' \
  https://api.agent37.com/v1/instances | python3 -c 'import json,sys;print(json.load(sys.stdin)["id"])')
echo "remote: VM $ID"
cleanup() { [ "$KEEP" = 1 ] || curl -s -X DELETE -H "Authorization: Bearer $K" "https://api.agent37.com/v1/instances/$ID" >/dev/null; }
trap cleanup EXIT
export AGENT37_API_KEY=$K
SSHO=(-F /dev/null -o "ProxyCommand=npx -y agent37@latest tunnel %h" -o StrictHostKeyChecking=no
      -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ServerAliveInterval=60 -o IdentityFile="$HOME/.ssh/id_ed25519")
vm() { ssh "${SSHO[@]}" "root@$ID.agent37.app" "$@"; }
for _ in $(seq 30); do vm true 2>/dev/null && break; sleep 10; done
T=$(mktemp -d); trap 'rm -rf "$T"; cleanup' EXIT
git -C "$REPO" bundle create "$T/oino.bundle" HEAD >/dev/null
scp "${SSHO[@]}" "$T/oino.bundle" "$HERE/vm-setup.sh" ${DEB:+"$DEB"} "root@$ID.agent37.app:/root/"
vm "bash /root/vm-setup.sh ${DEB:+--deb /root/$(basename "$DEB")}" 
vm "cd /root/oino && PERF_FORCE=1 bash scripts/perf/run.sh --deb /root/unipi.deb --out /root/perf-out $*"
mkdir -p "$REPO/target/perf-out/$ID"
scp -r "${SSHO[@]}" "root@$ID.agent37.app:/root/perf-out/*" "$REPO/target/perf-out/$ID/"
echo "remote: results in target/perf-out/$ID"
