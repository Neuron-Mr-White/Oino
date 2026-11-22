/**
 * `interrupted` (perf T13): the host was killed mid-turn. Core's resume repair closed the
 * dangling tool call and appended an `oino.turn-interrupted` marker; the chat opens on the
 * one-line "Interrupted — Continue" row. `?continued` adds the owner's Continue and the
 * resumed answer (the row stays as history, without the button). `?resumed` is a long-horizon chat: the
 * extension auto-continued with its agent-facing instruction (r7 P2-5: it must not show as a "System" card).
 */
import type { MockScenario } from '../host';
import { genCards, genFsTree } from '../gen';

const T = Date.parse('2026-11-14T09:00:00Z');
const continued = typeof location !== 'undefined' && new URLSearchParams(location.search).has('continued');
const resumed = typeof location !== 'undefined' && new URLSearchParams(location.search).has('resumed');
const RESUME = 'Your previous turn was interrupted (the host stopped or restarted while you were working); continue from where you stopped. Re-check the last tool results and the workspace before you repeat any outside action.';

function entry(n: number, extra: Record<string, unknown>) {
  return { ...extra, id: `e${String(n).padStart(4, '0')}`, parentId: n > 1 ? `e${String(n - 1).padStart(4, '0')}` : null, timestamp: new Date(T + n * 3000).toISOString() };
}

const ENTRIES = [
  entry(1, { type: 'message', message: { role: 'user', content: [{ type: 'text', text: 'Run the test suite and fix what fails' }], timestamp: T + 3000 } }),
  entry(2, { type: 'message', message: { role: 'assistant', content: [{ type: 'text', text: 'Running the suite first.' }, { type: 'toolCall', id: 'call_1', name: 'bash', arguments: { command: 'cargo test --workspace' } }], provider: 'anthropic', model: 'claude-sonnet-4-5', stopReason: 'toolUse', timestamp: T + 6000 } }),
  entry(3, { type: 'custom', customType: 'oino.tool-dispatch', data: { toolCallId: 'call_1', toolName: 'bash', input: { command: 'cargo test --workspace' }, readOnly: false } }),
  entry(4, { type: 'message', message: { role: 'toolResult', toolCallId: 'call_1', toolName: 'bash', content: [{ type: 'text', text: 'Tool execution was interrupted before completion; the outcome is unknown.' }], isError: true, timestamp: T + 12000 } }),
  entry(5, { type: 'custom', customType: 'oino.resume-repair', data: { closedToolCalls: ['call_1'], tornTail: false, at: T + 12000 } }),
  entry(6, { type: 'custom', customType: 'oino.turn-interrupted', data: { reason: 'tool_calls_closed', at: T + 12000 } }),
  ...(resumed
    ? [
        entry(7, { type: 'message', message: { role: 'user', content: [{ type: 'text', text: RESUME }], unipiHarness: { source: 'Long-horizon', title: 'Continue' }, timestamp: T + 20000 } }),
        entry(8, { type: 'message', message: { role: 'assistant', content: [{ type: 'text', text: 'Picking up where I stopped: the suite passes now.' }], provider: 'anthropic', model: 'claude-sonnet-4-5', stopReason: 'stop', timestamp: T + 30000 } }),
      ]
    : continued
    ? [
        entry(7, { type: 'custom_message', customType: 'unipi-continue', content: '', display: false }),
        entry(8, { type: 'message', message: { role: 'assistant', content: [{ type: 'text', text: 'Picking up where I stopped: the suite passes now.' }], provider: 'anthropic', model: 'claude-sonnet-4-5', stopReason: 'stop', timestamp: T + 30000 } }),
      ]
    : []),
];

export const interrupted: MockScenario = {
  name: 'interrupted',
  setup() { location.hash = '#/s/pc-coffee/int001'; },
  hosts: [
    {
      id: 'pc-coffee',
      name: 'coffee',
      conn: { state: 'connected' },
      cards: genCards(3, { prefix: 'int' }),
      fs: genFsTree(),
      sessions: {
        int001: {
          snapshot: {
            session_id: 'int001',
            reset: true,
            seq: 0,
            state: { sessionId: 'int001', isStreaming: false, isCompacting: false, model: { provider: 'anthropic', id: 'claude-sonnet-4-5' }, thinkingLevel: 'medium', messageCount: ENTRIES.length, pendingMessageCount: 0 },
            entries: ENTRIES,
            leaf_id: ENTRIES.at(-1)!.id,
            commands: [],
            ui: { scope: 'session', sessionId: 'int001', rev: 0, entries: [], dialogs: [] },
            topics: [],
          },
          events: [],
        },
      },
    },
  ],
};
