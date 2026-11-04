#!/usr/bin/env bash
# [chat-load] Chat stability rig: the real frontend of a tree (vite dev server + the dev
# mock host) serving COPIES of real chats, driven by headless Chromium (drive.mjs).
#
#   scripts/qa/chat-stability/run.sh <app-dir> <chat-dir> <label> <out-dir> [chat.jsonl ...]
#
#   app-dir   apps/unipi of the tree under test (a `git worktree` of an old commit works;
#             it needs node_modules — symlink the main tree's)
#   chat-dir  a directory of chat COPIES (never point it at ~/.oino/sessions)
#
# Prints one JSON line per chat and writes <out-dir>/<label>-<chat>.{json,mp4,*.png}.
# Light: one vite (no build) + one headless shell at a time; everything is killed on exit.
set -euo pipefail
app=$(cd "$1" && pwd); chats=$(cd "$2" && pwd); label=$3; out=$(mkdir -p "$4" && cd "$4" && pwd); shift 4
here=$(cd "$(dirname "$0")" && pwd)
case "$chats" in "$HOME/.oino"*) echo "refusing to read the live chat store; pass a directory of copies" >&2; exit 2 ;; esac
port=${RL_PORT:-$((1500 + RANDOM % 300))}
cfg="$app/.chat-stability.vite.ts"
cat > "$cfg" <<EOF
import { defineConfig } from 'vite';
import solid from 'vite-plugin-solid';
import tailwind from '@tailwindcss/vite';
const mockHost = { name: 'chat-stability', enforce: 'pre' as const, transform(code: string, id: string) {
  if (id.endsWith('/src/lib/host/factory.ts')) return \`import '/src/dev/mock/boot.ts';\n\${code}\`;
  if (id.endsWith('/src/dev/mock/scenarios/index.ts')) return \`\${code}\nimport { ownerChat as __oc } from '/@fs$here/ownerChat.scenario.ts';\nSCENARIOS['owner-chat'] = __oc as never;\n\`;
  return null; } };
export default defineConfig({ root: '$app', plugins: [solid(), tailwind(), mockHost], clearScreen: false, define: { 'import.meta.env.VITE_CHAT_DIR': JSON.stringify('$chats') },
  server: { port: $port, strictPort: true, host: '127.0.0.1', fs: { strict: false, allow: ['$app', '$here', '$chats'] } } });
EOF
vite_pid=
cleanup() { [ -n "$vite_pid" ] && kill -- -"$vite_pid" 2>/dev/null || true; rm -f "$cfg"; }
trap cleanup EXIT
( cd "$app" && exec setsid npx vite --config "$cfg" --force >"$out/.vite-$label.log" 2>&1 ) &
vite_pid=$!
for _ in $(seq 1 120); do curl -sf "http://127.0.0.1:$port/" >/dev/null && break; sleep 0.5; done
for chat in "$@"; do
  name=${chat%.jsonl}
  node "${RL_DRIVER:-$here/drive.mjs}" "$port" "$chat" "$label-$name" "$out" || echo "{\"label\":\"$label-$name\",\"error\":\"driver failed\"}"
done
