/**
 * `live-events` (owner-fix-8, item 1): the alignment case — event cards in a
 * **live** turn next to the session's header cards.
 *
 * The chat opens with a Mode card, a Memory recall card and a System row
 * (the preamble: the cards at the top of a session), then a user message whose
 * turn is still running (`isStreaming`): its memory recall card arrives *after*
 * the first tool call, so it lands inside the running work fold. Both cards are
 * the same kind of row and must share one left edge and one width.
 */
import type { MockScenario } from '../host';
import { genCards, genFsTree } from '../gen';

const T = Date.parse('2026-11-06T09:00:00Z');

const RECALL =
  'You have 124 memories for project "unipi".\nAvailable memories:\n- the timeline keeps its own scroll anchor';

function entry(n: number, parentId: string | null, extra: Record<string, unknown>) {
  return {
    ...extra,
    id: `e${String(n).padStart(4, '0')}`,
    parentId,
    timestamp: new Date(T + n * 3000).toISOString(),
  };
}

export const liveEvents: MockScenario = {
  name: 'live-events',
  hosts: [
    {
      id: 'pc-coffee',
      name: 'coffee',
      conn: { state: 'connected' },
      cards: genCards(3, { prefix: 'le' }),
      fs: genFsTree(),
      sessions: {
        s001: {
          snapshot: {
            session_id: 's001',
            reset: true,
            seq: 0,
            state: {
              sessionId: 's001',
              isStreaming: true,
              isCompacting: false,
              model: { provider: 'anthropic', id: 'claude-sonnet-4-5' },
              thinkingLevel: 'medium',
              messageCount: 8,
              pendingMessageCount: 0,
            },
            entries: [
              // ── the session header: three event cards, no user message above them ──
              entry(1, null, { type: 'custom', customType: 'long-horizon-decision', data: { mode: 'graph', source: 'explicit' } }),
              entry(2, 'e0001', { type: 'custom_message', customType: 'unipi-memory-recall-reminder', content: RECALL }),
              entry(3, 'e0002', { type: 'message', message: { role: 'system', content: 'You are the project agent for unipi.', timestamp: T + 9000 } }),
              // ── the running turn ──
              entry(4, 'e0003', {
                type: 'message',
                message: { role: 'user', content: [{ type: 'text', text: 'Fix the failing test in src/math.test.ts' }], timestamp: T + 12000 },
              }),
              entry(5, 'e0004', {
                type: 'message',
                message: {
                  role: 'assistant',
                  content: [{ type: 'toolCall', id: 'call_1', name: 'bash', arguments: { command: 'npm test -- --run' } }],
                  provider: 'anthropic',
                  model: 'claude-sonnet-4-5',
                  stopReason: 'toolUse',
                  timestamp: T + 15000,
                },
              }),
              entry(6, 'e0005', {
                type: 'message',
                message: {
                  role: 'toolResult',
                  toolCallId: 'call_1',
                  toolName: 'bash',
                  content: [{ type: 'text', text: ' × src/math.test.ts > add\nexpected -1 to be 3' }],
                  isError: false,
                  timestamp: T + 18000,
                },
              }),
              // the same memory card, but mid-turn: it lands inside the running fold
              entry(7, 'e0006', { type: 'custom_message', customType: 'unipi-memory-recall-reminder', content: RECALL }),
              ...(typeof location !== 'undefined' && new URLSearchParams(location.search).has('oin55') ? [
                entry(8, 'e0007', { type: 'message', message: { role: 'user', content: 'Agent graph “release” checkpoint: work needs attention (2 errored, 5 completed).', details: { oinoProvenance: { source: 'Long-horizon', title: 'Graph checkpoint' } }, timestamp: T + 24000 } }),
                entry(9, 'e0008', { type: 'message', message: { role: 'user', content: 'A background task is ready.', author: 'Host', timestamp: T + 27000 } }),
              ] : []),
            ],
            leaf_id: typeof location !== 'undefined' && new URLSearchParams(location.search).has('oin55') ? 'e0009' : 'e0007',
            commands: [],
            ui: { scope: 'session', sessionId: 's001', rev: 0, entries: [], dialogs: [] },
            topics: [],
          },
          events: [],
        },
      },
    },
  ],
};
