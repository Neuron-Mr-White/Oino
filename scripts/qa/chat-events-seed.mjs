#!/usr/bin/env node
// [chat-events] Writes a QA chat holding every chat event kind of the corpus
// (apps/unipi/src/dev/mock/fixtures/chatEvents.ts) in the session v3 layout,
// grouped into themed turns: events that open a turn sit before the work,
// the rest after the answer. Isolated agent dir only (never ~/.oino).
//
//   node scripts/qa/chat-events-seed.mjs <agent_dir> <cwd>   → prints the session file path
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';

const [agentDir, cwd] = process.argv.slice(2);
if (!agentDir || !cwd) {
  console.error('usage: chat-events-seed.mjs <agent_dir> <cwd>');
  process.exit(2);
}
const { CHAT_EVENT_CORPUS } = await import(new URL('../../apps/unipi/src/dev/mock/fixtures/chatEvents.ts', import.meta.url));
const byId = Object.fromEntries(CHAT_EVENT_CORPUS.map(([, e]) => [e.id, e]));

const TURNS = [
  { ask: 'Memory: recall, wake-up, save', lead: ['ev01', 'ev04'], answer: 'Recalled what this project knows and saved what this task taught.', trail: ['ev05', 'ev03', 'ev02'] },
  { ask: 'Agents: subagent, fleet, sidekick', lead: [], answer: 'Three helpers finished; their reports are below.', trail: ['ev06', 'ev07', 'ev08'] },
  { ask: 'Background tasks and shell', lead: [], answer: 'The test run failed once, then passed.', trail: ['ev09', 'ev10', 'ev28', 'ev29'] },
  { ask: 'Compaction', lead: [], answer: 'Context was compacted and recalled.', trail: ['ev11', 'ev12', 'ev13'] },
  { ask: 'Modes: graph, goal, ralph, plan', lead: ['ev14', 'ev31'], answer: 'Switched to Graph mode and checked progress.', trail: ['ev15', 'ev16', 'ev17', 'ev18'] },
  { ask: 'Warnings, skills, replies, board', lead: [], answer: 'Here is what the extensions reported.', trail: ['ev19', 'ev20', 'ev21', 'ev22', 'ev23'] },
  { ask: 'Dream, side question, attachments, browser, branch, other', lead: [], answer: 'The rest of the event kinds.', trail: ['ev24', 'ev25', 'ev26', 'ev27', 'ev30', 'ev32', 'ev33', 'ev34', 'ev35', 'ev36'] },
];

const sid = randomUUID();
const t0 = Date.parse('2026-11-03T09:00:00Z');
let n = 0;
let parent = null;
const lines = [{ type: 'session', version: 3, id: sid, timestamp: new Date(t0).toISOString(), cwd }];
const push = (e) => {
  n++;
  const at = t0 + n * 7000;
  const id = `ce${String(n).padStart(4, '0')}`;
  const entry = { ...structuredClone(e), id, parentId: parent, timestamp: new Date(at).toISOString() };
  if (entry.message) entry.message.timestamp = at;
  lines.push(entry);
  parent = id;
};
const msg = (role, content, extra = {}) => ({ type: 'message', message: { role, content, ...extra } });
const usage = { input: 1200, output: 300, cacheRead: 0, cacheWrite: 0, totalTokens: 1500, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0.002 } };

push({ type: 'model_change', provider: 'ollama', modelId: 'fake-1' });
for (const [i, t] of TURNS.entries()) {
  push(msg('user', [{ type: 'text', text: t.ask }]));
  for (const id of t.lead) push(byId[id]);
  push(msg('assistant', [{ type: 'toolCall', id: `call${i}`, name: 'read', arguments: { path: 'README.md' } }], { api: 'openai-completions', provider: 'ollama', model: 'fake-1', usage, stopReason: 'toolUse' }));
  push(msg('toolResult', [{ type: 'text', text: '# Readme\n' }], { toolCallId: `call${i}`, toolName: 'read', isError: false }));
  push(msg('assistant', [{ type: 'text', text: t.answer }], { api: 'openai-completions', provider: 'ollama', model: 'fake-1', usage, stopReason: 'stop' }));
  for (const id of t.trail) push(byId[id]);
}
push({ type: 'session_info', name: 'Chat events QA' });

const dir = join(agentDir, 'sessions', `--${cwd.replace(/^\/+|\/+$/g, '').replace(/\//g, '-')}--`);
mkdirSync(dir, { recursive: true });
const file = join(dir, `2026-11-03T09-00-00-000Z_${sid}.jsonl`);
writeFileSync(file, lines.map((l) => JSON.stringify(l)).join('\n') + '\n');
console.log(file);
