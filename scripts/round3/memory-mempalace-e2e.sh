#!/usr/bin/env bash
# unipi.memory ↔ MemPalace end-to-end verification (round 3, mempalace-rs integration).
#
# Every case runs in its own scratch HOME + OINO_AGENT_DIR under target/tmp, against a
# **copy** of the palace (`~/.mempalace` minus `daemon/`, so the live daemon is never
# reached; `~/.cache/chroma` is the read-only ONNX model cache and is symlinked).
#
#   scripts/round3/memory-mempalace-e2e.sh [--bin PATH] [--rust DIR] [--python DIR]
#
# Cases (one at a time):
#   1. rust-path     the Rust port on PATH (`mempalaceBin` empty)
#   2. python-path   the Python install on PATH (`mempalaceBin` empty) — the fallback
#   3. rust-config   `mempalaceBin: "~/mp-bin"` (folder, `~` expanded by the extension)
#   4. stale-config  `mempalaceBin: "~/nope"` → falls back to PATH (Python)
#
# Each case: `/memory` reports the backend, `memory_store` + `memory_search` +
# `memory_delete` run through the tool path with the scripted faux provider (daemon jobs,
# direct `mine`, warm `mempalace-mcp` reader), and the palace copy is checked with the CLI.
# Needs `cargo build -p oino --features faux` (the scripted provider).
set -uo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
bin="${OINO_BIN:-$root/target/debug/oino}"
rust_bin="${MEMPALACE_RS_BIN:-$HOME/Projects/Personal/llm/mempalace-rs/target/release}"
python_bin="${MEMPALACE_PY_BIN:-$HOME/.local/share/uv/tools/mempalace/bin}"
live="${MEMPALACE_LIVE:-$HOME/.mempalace}"
work="${MEMORY_E2E_DIR:-$root/target/tmp/memory-e2e}"

while [ $# -gt 0 ]; do
  case "$1" in
    --bin) bin="$2"; shift 2 ;;
    --rust) rust_bin="$2"; shift 2 ;;
    --python) python_bin="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

[ -x "$bin" ] || { echo "no binary at $bin (build with --features faux)" >&2; exit 2; }
[ -x "$rust_bin/mempalace" ] || { echo "no Rust port at $rust_bin" >&2; exit 2; }
[ -x "$python_bin/mempalace" ] || { echo "no Python install at $python_bin" >&2; exit 2; }
[ -d "$live/palace" ] || { echo "no palace at $live/palace" >&2; exit 2; }

fail=0
pass() { printf 'PASS  %s\n' "$*"; }
bad() { printf 'FAIL  %s\n' "$*"; fail=$((fail + 1)); }

rm -rf "$work"
mkdir -p "$work"

# The live daemon must keep running untouched: record it before and after every case.
live_unit_pid() {
  systemctl --user show -p MainPID mempalace.service 2>/dev/null | sed 's/^MainPID=//' || true
}
live_pid_before="$(live_unit_pid)"

# Copies one palace (no daemon state, no backups).
copy_palace() { # <case home>
  local home="$1"
  mkdir -p "$home/.mempalace" "$home/.cache"
  cp -a "$live/palace" "$home/.mempalace/palace"
  for f in config.json knowledge_graph.sqlite3 hallways.json tunnels.json notes hook_state; do
    [ -e "$live/$f" ] && cp -a "$live/$f" "$home/.mempalace/$f"
  done
  [ -e "$HOME/.cache/chroma" ] && ln -sfn "$HOME/.cache/chroma" "$home/.cache/chroma"
  return 0
}

# RPC driver: `/memory`, then a prompt that the scripted faux provider answers with a tool
# call, three times (store → search → delete). Prints one `notify` line per message.
driver="$work/drive.py"
cat >"$driver" <<'PY'
import json, os, queue, subprocess, sys, threading

bin, home, agent = sys.argv[1], sys.argv[2], sys.argv[3]
env = dict(os.environ, HOME=home, OINO_AGENT_DIR=agent)
env.setdefault("OINO_FAUX_API_KEY", "faux-key")
p = subprocess.Popen(
    [bin, "--mode", "rpc", "-e", "builtin:faux", "--model", "faux/faux-1", "-nc", "-ns", "-np"],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    env=env, text=True, bufsize=1,
)
lines = queue.Queue()


def pump():
    for line in p.stdout:
        lines.put(line)
    lines.put(None)


threading.Thread(target=pump, daemon=True).start()


def send(cmd, i):
    cmd["id"] = f"c{i}"
    p.stdin.write(json.dumps(cmd) + "\n")
    p.stdin.flush()


def until(pred, seconds=180):
    seen = []
    for _ in range(100000):
        try:
            line = lines.get(timeout=seconds)
        except queue.Empty:
            break
        if line is None:
            break
        v = json.loads(line)
        seen.append(v)
        if pred(v):
            return v, seen
    return None, seen


for i, (kind, payload) in enumerate([("command", "/memory"), ("prompt", "store it"), ("prompt", "search it"), ("prompt", "delete it")], 1):
    send({"type": "prompt", "message": payload}, i)
    reply, seen = until(lambda v, i=i: v.get("type") == "response" and v.get("id") == f"c{i}")
    notes = [v.get("message", "") for v in seen if v.get("type") == "extension_ui_request" and v.get("method") == "notify"]
    if reply is None:
        print(json.dumps({"step": kind, "error": "no reply", "notes": notes}))
        break
    if reply.get("data", {}).get("disposition") == "started":
        _, settled = until(lambda v: v.get("type") == "agent_settled")
        notes += [v.get("message", "") for v in settled
                  if v.get("type") == "extension_ui_request" and v.get("method") == "notify"]
    send({"type": "get_messages"}, 100 + i)
    msgs, _ = until(lambda v, i=i: v.get("type") == "response" and v.get("id") == f"c{100 + i}")
    texts = []
    for m in (msgs or {}).get("data", {}).get("messages", []):
        c = m.get("content")
        if isinstance(c, str):
            texts.append(c)
        elif isinstance(c, list):
            for part in c:
                if isinstance(part, dict) and part.get("type") in (None, "text") and part.get("text"):
                    texts.append(part["text"])
    print(json.dumps({"step": kind, "disposition": reply.get("data", {}).get("disposition"), "notes": notes, "texts": texts}))
p.stdin.close()
try:
    p.wait(timeout=30)
except subprocess.TimeoutExpired:
    p.kill()
stderr = p.stderr.read()
if stderr.strip():
    print(json.dumps({"stderr": stderr[-2000:]}))
PY

run_case() { # <name> <bins-setting> <path-prepend> <expect: rust|python>
  local name="$1" setting="$2" prepend="$3" expect="$4"
  local dir="$work/$name" home="$work/$name/home" agent="$work/$name/agent"
  mkdir -p "$agent"
  copy_palace "$home"
  if [ -n "$setting" ]; then
    python3 - "$agent" "$setting" <<'PY'
import json, sys
agent, setting = sys.argv[1], sys.argv[2]
path = agent + "/settings.json"
json.dump({"extensionSettings": {"memory": {
    "mempalaceBin": setting, "recall": "off", "write": True,
    "saveMode": "off", "autoStartDaemon": True, "mempalaceAutoUpdate": False,
}}}, open(path, "w"), indent=2)
PY
  fi
  # Case 3: `~/mp-bin` is a folder holding mempalace + mempalace-mcp.
  if [ -n "$setting" ] && [ "$setting" = "~/mp-bin" ]; then
    ln -sfn "$rust_bin" "$home/mp-bin"
  fi
  local path="$python_bin:/usr/local/bin:/usr/bin:/bin"
  [ -n "$prepend" ] && path="$prepend:$path"
  local out="$work/$name.jsonl"
  HOME="$home" PATH="$path" OINO_FAUX_RESPONSES="$(cat <<'JSON'
[{"toolCalls":[{"id":"s1","name":"memory_store","arguments":{"title":"mempalace_rs_smoke","content":"oino's unipi.memory drives MemPalace on a palace copy; parity smoke note.","type":"decision","tags":["smoke"]}}]},
 {"text":"stored"},
 {"toolCalls":[{"id":"s2","name":"memory_search","arguments":{"query":"mempalace_rs_smoke","scope":"project"}}]},
 {"text":"searched"},
 {"toolCalls":[{"id":"s3","name":"memory_delete","arguments":{"title":"mempalace_rs_smoke"}}]},
 {"text":"deleted"}]
JSON
)" timeout 900 python3 "$driver" "$bin" "$home" "$agent" >"$out" 2>"$work/$name.err"
  local rc=$?
  local backend
  backend="$(python3 - "$out" <<'PY'
import json, sys
for line in open(sys.argv[1]):
    try: v = json.loads(line)
    except ValueError: continue
    for n in v.get("notes", []) + v.get("texts", []):
        for l in str(n).splitlines():
            if l.startswith("backend: mempalace "):
                print(l)
                raise SystemExit
PY
)"
  if [ -z "$backend" ]; then
    bad "$name: no backend line (rc=$rc, see $out)"
    sed -n '1,5p' "$work/$name.err" >&2
    return
  fi
  case "$backend" in
    *"($expect)"*) pass "$name: $backend" ;;
    *) bad "$name: expected ($expect), got '$backend'" ;;
  esac
  [ -n "$setting" ] && { grep -q "binary: " "$out" && pass "$name: binary line present" || bad "$name: no binary line"; }
  [ "$setting" = "~/mp-bin" ] && { grep -q "binary: $home/mp-bin/mempalace" "$out" && pass "$name: ~ expanded to the configured folder" || bad "$name: configured path not expanded"; }
  [ "$setting" = "~/nope" ] && { grep -q "binary: mempalace " "$out" && pass "$name: stale path fell back to PATH" || bad "$name: stale path was not replaced by PATH"; }
  grep -q 'filed to the palace' "$out" && pass "$name: memory_store filed" || bad "$name: memory_store did not file"
  grep -qE 'Found [1-9][0-9]* memories' "$out" && pass "$name: memory_search found it" || bad "$name: memory_search found nothing"
  grep -q 'Deleted memory' "$out" && pass "$name: memory_delete removed it" || bad "$name: memory_delete failed"
  if [ "$(live_unit_pid)" = "$live_pid_before" ]; then
    pass "$name: live mempalace.service untouched"
  else
    bad "$name: live mempalace.service pid changed"
  fi
  # Stop the daemon this case started on the copy (state lives inside the copy).
  local mp="$python_bin/mempalace"
  [ -n "$prepend" ] && mp="$prepend/mempalace"
  [ "$setting" = "~/mp-bin" ] && mp="$rust_bin/mempalace"
  HOME="$home" "$mp" --palace "$home/.mempalace/palace" daemon stop >/dev/null 2>&1
  # After store + delete the copy's index must hold no drawer with the note's text. The CLI
  # `search` echoes the query, so read the palace's own sqlite (both ports write chroma).
  if command -v sqlite3 >/dev/null; then
    n="$(sqlite3 "$home/.mempalace/palace/chroma.sqlite3" \
      "select count(*) from embedding_metadata where string_value like '%unipi.memory drives MemPalace on a palace copy%';" 2>/dev/null)"
    [ "$n" = "0" ] && pass "$name: drawer gone from the palace copy" \
      || bad "$name: drawer still in the palace copy ($n)"
  fi
  if ls "$agent"/state/unipi.memory/global/memory/*/*/mempalace_rs_smoke*.md >/dev/null 2>&1; then
    bad "$name: markdown memory file still present"
  else
    pass "$name: markdown memory file gone"
  fi
  rm -rf "$home/.mempalace/daemon"
}

echo "== rust port:   $rust_bin"
echo "== python:      $python_bin"
echo "== palace copy: $live (live daemon MainPID ${live_pid_before:-unknown})"

run_case rust-path "" "$rust_bin" rust
run_case python-path "" "" python
run_case rust-config "~/mp-bin" "" rust
run_case stale-config "~/nope" "" python

[ "$(live_unit_pid)" = "$live_pid_before" ] && pass "live mempalace.service untouched (all cases)" || bad "live mempalace.service pid changed"

if [ "$fail" -eq 0 ]; then
  echo "RESULT: PASS (all memory/mempalace-rs checks)"
  exit 0
fi
echo "RESULT: FAIL ($fail)"
exit 1
