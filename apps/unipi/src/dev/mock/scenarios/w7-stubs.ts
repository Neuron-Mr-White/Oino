/**
 * `w7-stubs` (1.0.10 rc3 G3-1): a chat whose huge entries the host sends as reference-only stubs
 * `{id, truncated:true}` and serves through `get_entries{ids, chunkOffset}`: a huge tool CALL (1.4 MB
 * command), a huge tool RESULT (1.3 MB) and a huge ANSWER (1.2 MB). Open it, then reload with `&keep=1`
 * (the localStorage snapshot cache survives = an app relaunch).
 */
import type { MockScenario } from '../host';

const T = Date.parse('2026-11-14T09:00:00Z');
const at = (n: number) => new Date(T + n * 3000).toISOString();
const line = (tag: string, i: number) => `${tag} line ${String(i).padStart(6, '0')} — the quick brown fox jumps over the lazy dog`;
const big = (tag: string, bytes: number, sep = '\n') => {
  const out: string[] = [];
  for (let i = 0, n = 0; n < bytes; i++) {
    const l = line(tag, i);
    out.push(l);
    n += l.length + 1;
  }
  out.push(`${tag}-END`);
  return out.join(sep);
};

const HUGE_CMD = `echo start\n${big('arg', 1_400_000)}`;
const HUGE_OUT = big('out', 1_300_000);
const HUGE_ANSWER = Array.from({ length: 12_000 }, (_, i) => `Paragraph ${i + 1}: the migration moved another module over without touching its callers.`).join('\n\n') + '\n\nHUGE-ANSWER-END';

const msg = (n: number, message: Record<string, unknown>) => ({ type: 'message', id: `c${n}`, parentId: n > 1 ? `c${n - 1}` : null, timestamp: at(n), message: { ...message, timestamp: T + n * 3000 } });
const asst = (content: unknown[], stopReason: string) => ({ role: 'assistant', content, provider: 'anthropic', model: 'claude-sonnet-4-5', stopReason });

export const W7_ENTRIES = [
  msg(1, { role: 'user', content: [{ type: 'text', text: 'run the big script' }] }),
  msg(2, asst([{ type: 'text', text: 'Running the big script.' }, { type: 'toolCall', id: 'big1', name: 'bash', arguments: { command: HUGE_CMD } }], 'toolUse')),
  msg(3, { role: 'toolResult', toolCallId: 'big1', toolName: 'bash', content: [{ type: 'text', text: HUGE_OUT }], isError: false }),
  msg(4, asst([{ type: 'text', text: 'HUGE-TOOL-AFTER: the script finished.' }], 'stop')),
  msg(5, { role: 'user', content: [{ type: 'text', text: 'now write the long report' }] }),
  msg(6, asst([{ type: 'text', text: HUGE_ANSWER }], 'stop')),
];
const STUBS = ['c2', 'c3', 'c6'];

export const w7Stubs: MockScenario = {
  name: 'w7-stubs',
  setup() {
    location.hash = '#/s/pc-coffee/w7s001';
  },
  hosts: [
    {
      id: 'pc-coffee',
      name: 'coffee',
      conn: { state: 'connected' },
      sessions: {
        w7s001: {
          stubIds: STUBS,
          snapshot: {
            session_id: 'w7s001',
            reset: true,
            seq: 0,
            state: { sessionId: 'w7s001', isStreaming: false, isCompacting: false, model: { provider: 'anthropic', id: 'claude-sonnet-4-5' }, thinkingLevel: 'medium', messageCount: 6, pendingMessageCount: 0 },
            entries: W7_ENTRIES.map((e) => (STUBS.includes(e.id) ? { id: e.id, truncated: true } : e)),
            leaf_id: 'c6',
            commands: [],
            ui: { scope: 'session', sessionId: 'w7s001', rev: 0, entries: [], dialogs: [] },
            topics: [],
          },
          entries: { entries: W7_ENTRIES, leafId: 'c6' },
          events: [],
        },
      },
    },
  ],
};
