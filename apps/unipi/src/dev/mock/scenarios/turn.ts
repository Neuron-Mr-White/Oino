/**
 * WP-A7 capture scenarios:
 *
 * - `turn-actions`: several settled turns with tools, costs and timestamps
 *   (today / yesterday / older), an image, a stopped turn and a long final
 *   query — hover a turn tail for Copy/Quote/Fork/More and the time label.
 * - `turn-navigator`: twelve short turns so the query rail shows at ≥ 1100 px.
 * - `bubble-fold`: one very long prompt with an image and file chip, folded to
 *   120 px with the centred expand button.
 */
import type { MockScenario } from '../host';
import { genCards, genEntries, genFsTree, genSession, type Entry, type GenOptions } from '../gen';

const BASE = Date.parse('2026-10-30T09:00:00Z');

/** Run `fn` over the entries of one turn (the `index`th user message and what follows). */
function overTurn(entries: Entry[], index: number, fn: (entry: Entry) => void): void {
  let turn = -1;
  const isUser = (e: Entry) => e.type === 'message' && (e.message as { role?: string } | undefined)?.role === 'user';
  for (const entry of entries) {
    if (isUser(entry)) turn += 1;
    if (turn === index) fn(entry);
  }
}

function shiftTurn(entries: Entry[], index: number, deltaMs: number): void {
  overTurn(entries, index, (entry) => {
    entry.timestamp = new Date(Date.parse(entry.timestamp) + deltaMs).toISOString();
    const message = entry.message as { timestamp?: number } | undefined;
    if (message && typeof message.timestamp === 'number') message.timestamp += deltaMs;
  });
}

/** Appends one user → assistant turn after the last entry. */
function appendTurn(entries: Entry[], text: string, answer: string, opts: { stopReason?: string; at?: number } = {}): void {
  const last = entries.at(-1)!;
  const at = opts.at ?? Date.parse(last.timestamp) + 45_000;
  const id = (n: number) => `e${String(entries.length + n).padStart(4, '0')}`;
  entries.push({
    type: 'message',
    id: id(1),
    parentId: last.id,
    timestamp: new Date(at).toISOString(),
    message: { role: 'user', content: [{ type: 'text', text }], timestamp: at },
  } as Entry);
  entries.push({
    type: 'message',
    id: id(2),
    parentId: id(1),
    timestamp: new Date(at + 25_000).toISOString(),
    message: {
      role: 'assistant',
      content: [{ type: 'text', text: answer }],
      stopReason: opts.stopReason ?? 'stop',
      provider: 'anthropic',
      model: 'claude-sonnet-4-5',
      usage: { input: 2400, output: 320, totalTokens: 2720, cost: { total: 0.0121 } },
      timestamp: at + 25_000,
    },
  } as Entry);
}

const LONG_PROMPT = [
  'Review the timeline reducer and make the tail fast, then tell me what you changed.',
  '',
  'Context and constraints:',
  '- 2 000 settled turns must open in under 120 ms on the desktop; the running turn streams outside the virtual list.',
  '- The fold is forced open while running and on stopped/error, and the user\'s toggle must survive virtual-scroll remounts.',
  '- The bottom lock is intent-based: detach at > 48 px, re-lock within 48 px or on the jump pill, and show the unread dot only for content that arrived while detached.',
  '- Do not touch the composer, the dock, or the sidebar while you are in there.',
  '',
  'Acceptance: height cache LRU, bottom-lock tests, scroll memory, and a 60 fps scroll pass with the perf marks recorded in `window.__UNIPI_PERF__`.',
].join('\n');

function session(opts: GenOptions, mutate?: (entries: Entry[]) => void) {
  const entries = genEntries(opts);
  mutate?.(entries);
  const mock = genSession('s001', opts);
  mock.snapshot.entries = entries as unknown as typeof mock.snapshot.entries;
  mock.snapshot.leaf_id = entries.at(-1)?.id ?? null;
  return mock;
}

export const turnActions: MockScenario = {
  name: 'turn-actions',
  hosts: [
    {
      id: 'pc-coffee',
      name: 'coffee',
      conn: { state: 'connected' },
      cards: genCards(4, { prefix: 's' }),
      fs: genFsTree(),
      sessions: {
        s001: session(
          { seed: 41, turns: 4, toolsPerTurn: 1, thinking: true, diffs: true, images: true, cwd: '/home/you/projects/unipi' },
          (entries) => {
            shiftTurn(entries, 0, -86_400_000);
            shiftTurn(entries, 1, BASE - Date.parse('2025-12-31T11:00:00Z'));
            appendTurn(entries, 'Stop here — I want to check the diff myself.', '', { stopReason: 'aborted' });
            const last = entries.at(-1)!;
            appendTurn(entries, LONG_PROMPT, 'The fold now closes at 120 px and the tail stays O(1); scroll memory and the height cache are unchanged.', {
              at: Date.parse(last.timestamp) + 60_000,
            });
          },
        ),
      },
    },
  ],
};

export const turnNavigator: MockScenario = {
  name: 'turn-navigator',
  hosts: [
    {
      id: 'pc-coffee',
      name: 'coffee',
      conn: { state: 'connected' },
      cards: genCards(4, { prefix: 's' }),
      fs: genFsTree(),
      sessions: {
        s001: session({ seed: 42, turns: 12, toolsPerTurn: 1, cwd: '/home/you/projects/unipi' }),
      },
    },
  ],
};

export const bubbleFold: MockScenario = {
  name: 'bubble-fold',
  hosts: [
    {
      id: 'pc-coffee',
      name: 'coffee',
      conn: { state: 'connected' },
      cards: genCards(4, { prefix: 's' }),
      fs: genFsTree(),
      sessions: {
        s001: session(
          { seed: 43, turns: 2, toolsPerTurn: 0, images: true, cwd: '/home/you/projects/unipi' },
          (entries) => {
            // make the first user message the long one (the fold shot)
            const first = entries.find((e) => e.type === 'message');
            if (first) {
              (first.message as { content: unknown[] }).content = [
                { type: 'text', text: LONG_PROMPT },
                { type: 'image', mimeType: 'image/png', data: 'iVBORw0KGgoAAAANSUhEUgAAADAAAAAwCAYAAABXAvmHAAABJ0lEQVR42u3ZwUoCYRQF4HmLThCIG58xIiJCEImQEEIiIgJJEZEeQ0kKlUKpSAwVRZk3mJilB53Vz/ifuIuz/7/FHe49ExwdHkTKCQxgAAP8U0Aum/EqBjDAvgHhMaL1CaLVKfQByzN3iDCsbSQ1wOIczh+fhHAyAwyYX8D543chnA0xA2Z5aAHiMGBagBYgDgN+i9ACxGHA5BJ+D/G2MGB8BT8/o0lhwE8JeqsEA76vobcLMeCrDL1ljgGfN9DbRhkwqkBvnWbA8BZ69wADPu6gd9Aw4P0eehcZAwYP0DspGdB/hN5NzIBeFXpHPQPenqDXSjDgtQ69WoUB3Qb0eiEGvDShV2wxoNOCXjPHgPYz9KpFeUAceYC10wbwCWB/KQ1gAAOkkj92JS6B78SdMwAAAABJRU5ErkJggg==' },
              ];
            }
          },
        ),
      },
    },
  ],
};
