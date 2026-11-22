#!/usr/bin/env node
// One-off converter (WP-A1): a recorded unipi bridge session (archive
// `unipi-app/docs/m5/fixtures/rec-*.json`, `[{at, line}]` of bridge JSON lines)
// → a mock-host fixture `{source, snapshot, events}` in oino shapes:
// `snapshot` = host.md §7.3 `SessionSnapshot` (entries verbatim — pi and oino
// share the session entry format), `events` = `{at, dto: SessionEventDto}` with
// `event` in oino `ClientEvent` names (agent_start/end, message_start/update/end,
// tool_execution_start/end).
//
//   node scripts/convert-rec-fixture.mjs <rec.json> > src/dev/mock/fixtures/<name>.json
import { readFileSync } from 'node:fs';
import { basename } from 'node:path';

const file = process.argv[2];
if (!file) {
  console.error('usage: convert-rec-fixture.mjs <rec.json>');
  process.exit(2);
}
const rows = JSON.parse(readFileSync(file, 'utf8')).map((r) => ({ at: r.at, msg: JSON.parse(r.line) }));
const hello = rows.find((r) => r.msg.t === 'hello')?.msg;
if (!hello) throw new Error('no hello line');
const sessionId = hello.session.id;
let seq = 0;
const events = [];
const push = (at, event) => events.push({ at, dto: { session: sessionId, seq: ++seq, event } });
const DELTA = { text: 'text_delta', thinking: 'thinking_delta', toolcall: 'toolcall_delta' };
for (const { at, msg } of rows) {
  switch (msg.t) {
    case 'state':
      if (msg.running === true) push(at, { type: 'agent_start' });
      else if (msg.running === false) push(at, { type: 'agent_end' });
      break;
    case 'entry':
      if (msg.entry.type === 'message') push(at, { type: 'message_end', message: msg.entry.message, entry: msg.entry });
      else push(at, { type: 'entry_added', entry: msg.entry });
      break;
    case 'msg_start':
      push(at, { type: 'message_start', message: { id: msg.id, role: msg.role, content: [] } });
      break;
    case 'delta':
      push(at, { type: 'message_update', message: { id: msg.id }, assistantMessageEvent: { type: DELTA[msg.kind] ?? 'text_delta', contentIndex: msg.index ?? 0, delta: msg.text } });
      break;
    case 'msg_end':
      // The settled message arrives as the following `entry` (message_end above).
      break;
    case 'tool_start':
      push(at, { type: 'tool_execution_start', toolCallId: msg.callId, toolName: msg.name, args: msg.args });
      break;
    case 'tool_end':
      push(at, { type: 'tool_execution_end', toolCallId: msg.callId, isError: !!msg.isError, result: msg.result ?? null });
      break;
    default:
      break; // hello / ack / input: transport noise
  }
}
const out = {
  source: `archived unipi-app docs/m5/fixtures/${basename(file)} (converted by scripts/convert-rec-fixture.mjs)`,
  snapshot: {
    session_id: sessionId,
    cwd: hello.session.cwd,
    reset: true,
    seq: 0,
    state: hello.state,
    entries: hello.entries,
    leaf_id: hello.entries.at(-1)?.id ?? null,
    commands: hello.commands ?? [],
    ui: [],
    topics: [],
  },
  events,
};
process.stdout.write(`${JSON.stringify(out, null, 1)}\n`);
