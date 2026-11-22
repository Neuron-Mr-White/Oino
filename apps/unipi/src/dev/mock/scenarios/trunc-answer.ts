/**
 * `trunc-answer` (hotfix 1.0.9): a chat opened from a snapshot whose final answer is 40 KB and
 * the host cut it at its 32 KiB prose budget (`message.truncated` + the host note, mid inline
 * code). `get_entries{ids}` answers the full entries (`?slowfull` delays it).
 */
import type { MockScenario } from '../host';
import { genCards, genFsTree } from '../gen';

const T = Date.parse('2026-11-14T09:00:00Z');
const NOTE = '\n…[truncated by the host — expand for the full output]';
const BUDGET = 32 * 1024;

function longAnswer(): string {
  const out: string[] = ['## Migration report\n'];
  for (let i = 1; out.join('\n').length < 40 * 1024; i++) {
    out.push(`### Step ${i}\n\nThe \`handler_${i}\` module now calls \`session.open(id)\` before it reads the snapshot, so a chat that was cut by the frame bound still gets its whole history. Checked against \`crates/unipi-host/src/sessions/bound.rs\` and the timeline row tests; nothing else changed in step ${i}.\n`);
  }
  out.push('Everything above is done. FINAL-SENTENCE-MARKER: the full answer reached the screen.');
  return out.join('\n');
}

const FULL = longAnswer();
// Cut where the host would: at the budget, in the middle of an inline-code span.
let at = BUDGET - NOTE.length;
while (FULL[at - 1] !== '`' && at > 0) at -= 1;
const HEAD = FULL.slice(0, at + 4) + NOTE;

const entry = (n: number, extra: Record<string, unknown>) => ({
  ...extra,
  id: `e${n}`,
  parentId: n > 1 ? `e${n - 1}` : null,
  timestamp: new Date(T + n * 3000).toISOString(),
});
const user = entry(1, { type: 'message', message: { role: 'user', content: [{ type: 'text', text: 'Write up the migration report' }], timestamp: T + 3000 } });
const assistant = (text: string, cut: boolean) =>
  entry(2, { type: 'message', message: { role: 'assistant', content: [{ type: 'text', text }], provider: 'anthropic', model: 'claude-sonnet-4-5', stopReason: 'stop', timestamp: T + 6000, ...(cut ? { truncated: true } : {}) } });

export const truncAnswer: MockScenario = {
  name: 'trunc-answer',
  setup() { location.hash = '#/s/pc-coffee/tr001'; },
  hosts: [
    {
      id: 'pc-coffee',
      name: 'coffee',
      conn: { state: 'connected' },
      cards: genCards(3, { prefix: 'tr' }),
      fs: genFsTree(),
      sessions: {
        tr001: {
          snapshot: {
            session_id: 'tr001',
            reset: true,
            seq: 0,
            state: { sessionId: 'tr001', isStreaming: false, isCompacting: false, model: { provider: 'anthropic', id: 'claude-sonnet-4-5' }, thinkingLevel: 'medium', messageCount: 2, pendingMessageCount: 0 },
            entries: [user, assistant(HEAD, true)],
            leaf_id: 'e2',
            commands: [],
            ui: { scope: 'session', sessionId: 'tr001', rev: 0, entries: [], dialogs: [] },
            topics: [],
          },
          // What `get_entries{ids}` answers: the whole entries.
          entries: { entries: [user, assistant(FULL, false)], leafId: 'e2' },
          events: [],
        },
      },
    },
  ],
};
