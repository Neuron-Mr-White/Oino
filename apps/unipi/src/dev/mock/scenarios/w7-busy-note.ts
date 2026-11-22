/**
 * `w7-busy-note` (1.0.10 rc3 G3-3): a long chat; `/swarm …` or `/graph …` is answered the way a busy host does
 * it — the command's own turn (user bubble + a short reply) and, as a ui `notify` effect, a long refusal note
 * that becomes the folded command-report row at the bottom. After sending, the sent command must be on screen.
 */
import type { MockScenario } from '../host';
import { noteCommandRun, routeNotify } from '../../../lib/reports';

const T = Date.parse('2026-11-14T09:00:00Z');
const at = (n: number) => new Date(T + n * 3000).toISOString();
const SID = 'nb001';
const KEY = `pc-coffee/${SID}`;

const entry = (n: number, role: 'user' | 'assistant', text: string) => ({
  type: 'message',
  id: `n${n}`,
  parentId: n > 1 ? `n${n - 1}` : null,
  timestamp: at(n),
  message: { role, content: [{ type: 'text', text }], timestamp: T + n * 3000, ...(role === 'assistant' ? { provider: 'anthropic', model: 'claude-sonnet-4-5', stopReason: 'stop' } : {}) },
});
const ENTRIES = Array.from({ length: 12 }, (_, i) => [
  entry(i * 2 + 1, 'user', `Question ${i + 1}: how does module ${i + 1} wire its handlers?`),
  entry(i * 2 + 2, 'assistant', `Module ${i + 1} registers its handlers in \`init()\`.\n\nIt then hands the router to the session, so a chat that reopens keeps every route. Nothing else changed in step ${i + 1}; the tests for it pass.`),
]).flat();

let seq = 100;
let n = ENTRIES.length;

export const w7BusyNote: MockScenario = {
  name: 'w7-busy-note',
  setup() {
    location.hash = `#/s/pc-coffee/${SID}`;
  },
  hosts: [
    {
      id: 'pc-coffee',
      name: 'coffee',
      conn: { state: 'connected' },
      sessions: {
        [SID]: {
          snapshot: {
            session_id: SID,
            reset: true,
            seq,
            state: { sessionId: SID, isStreaming: false, isCompacting: false, model: { provider: 'anthropic', id: 'claude-sonnet-4-5' }, thinkingLevel: 'medium', messageCount: ENTRIES.length, pendingMessageCount: 0 },
            entries: ENTRIES,
            leaf_id: `n${ENTRIES.length}`,
            commands: [{ name: 'swarm', description: 'Run independent items in parallel', source: 'extension' }, { name: 'graph', description: 'Run a graph', source: 'extension' }] as never,
            ui: { scope: 'session', sessionId: SID, rev: 0, entries: [], dialogs: [] },
            topics: [],
          },
          entries: { entries: ENTRIES, leafId: `n${ENTRIES.length}` },
          events: [],
          cmd: {
            prompt: (cmd, host) => {
              const text = String((cmd as { message?: string }).message ?? '');
              const push = (event: Record<string, unknown>) => host.world.push('pc-coffee', SID, [{ session: SID, seq: (seq += 1), event } as never]);
              if (/^\/(swarm|graph)/.test(text)) {
                // The host records the command as `oino.command-echo` (carrying the originating request id): the outbox bubble pairs with it.
                n += 1;
                const id = `n${n}`;
                const [name, ...rest] = text.slice(1).split(' ');
                const delay = Number(new URLSearchParams(location.search).get('echodelay') ?? 300);
                setTimeout(() => push({ type: 'entry_appended', entry: { type: 'custom', id, parentId: `n${n - 1}`, timestamp: new Date().toISOString(), customType: 'oino.command-echo', data: { name, args: rest.join(' '), messageId: (cmd as { retryMessageId?: string }).retryMessageId } } }), delay);
                noteCommandRun(KEY, text.slice(1).split(' ')[0]!);
                setTimeout(
                  () => routeNotify('A graph is already running in another chat (graph 4f2c).\nStop it first with /graph stop, or wait for it to finish.\nQueued commands: none.\nThe command you sent was not started.', 'warning', KEY),
                  Number(new URLSearchParams(location.search).get('notedelay') ?? 450),
                );
              } else {
                n += 1;
                push({ type: 'entry_appended', entry: entry(n, 'user', text) });
                n += 1;
                push({ type: 'entry_appended', entry: entry(n, 'assistant', 'Done.') });
              }
              return {};
            },
          },
        },
      },
    },
  ],
};
