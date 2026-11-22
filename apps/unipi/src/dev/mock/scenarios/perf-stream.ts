/**
 * `perf-stream` (v3-perf P5, spec §3 T9 / §4.7): the webview render budget scenario.
 *
 * - `big` (shown): a 40 000-entry chat. The snapshot carries the newest page (80 entries,
 *   `page.has_more`), older pages come from `get_entries{before, limit}` like the real host.
 * - `w01`…`w09`: nine more chats, all subscribed (acquired, as status panes / the superagent
 *   fleet hold them) but not on screen.
 * - All ten stream at ~50 tokens/s (one token per 20 ms each) with the real host's wire shape:
 *   `message_update` carrying the full partial `message` plus the `text_delta`, each frame
 *   JSON round-tripped (the app parses every frame it gets). Every ~400 tokens the message
 *   ends, a `bash` tool runs with streamed output, and a new assistant message starts — the
 *   live turn keeps growing like a long agent run.
 *
 * `?scenario=perf-stream&shell=desktop` → opens on `big`. `window.__PERF_STREAM__.stop()`
 * stops the drivers; `.chats` sets how many stream (default 10).
 */
import type { MockScenario, MockSession, MockWorld } from '../host';
import { BASE_TIME, genCards, genEntries, genFsTree, genSession, type Entry } from '../gen';

const HOST = 'pc-coffee';
const BIG = 'big';
const HIDDEN = Array.from({ length: 9 }, (_, i) => `w${String(i + 1).padStart(2, '0')}`);
const ENTRIES = 40_000;
const PAGE = 80;
const TOKEN_MS = 20;
const TOKENS_PER_MESSAGE = 400;
const MODEL = 'claude-sonnet-4-5';

/** Markdown the stream types out: prose, a list, inline code and a highlighted fence. */
const CORPUS = [
  'Looking at the reducer, the tail turn is rebuilt on every flush, which is the hot path here.',
  '',
  '- `applyDelta` appends the text in place',
  '- `takeMessage` patches unchanged parts by identity',
  '- the turns index recomputes only the tail',
  '',
  '```ts',
  'export function tail(entries: readonly Entry[], from: number): Turn[] {',
  '  const out: Turn[] = [];',
  '  for (let i = from; i < entries.length; i++) {',
  '    const e = entries[i]!;',
  "    if (e.type === 'message' && e.message?.role === 'user') out.push(open(e));",
  '    else out.at(-1)?.entries.push(e);',
  '  }',
  '  return out;',
  '}',
  '```',
  '',
  'So the fix is to keep settled blocks cached by **content** and only re-render the growing one; the rest of the turn stays mounted.',
  '',
].join('\n');
const TOKENS = CORPUS.match(/\s*\S+|\s+/g) ?? [CORPUS];

/** The 40k chat: generated once, on first use (the scenario registry imports every scenario). */
let bigEntries: Entry[] | undefined;
function big(): Entry[] {
  if (!bigEntries) {
    // 6 entries per turn (user, 2 × tool call + result, answer) + the model_change.
    bigEntries = genEntries({ seed: 40, turns: Math.ceil((ENTRIES - 1) / 6), toolsPerTurn: 2, thinking: true, diffs: true, cwd: '/home/you/projects/unipi' }).slice(0, ENTRIES);
    // End on a user prompt: the run streaming below answers it.
    const last = bigEntries.at(-1)!;
    if ((last.message as { role?: string } | undefined)?.role !== 'user') {
      bigEntries.push({ type: 'message', id: 'e-perf-prompt', parentId: last.id, timestamp: new Date(BASE_TIME + 9e8).toISOString(), message: { role: 'user', content: [{ type: 'text', text: 'Profile the timeline while ten chats stream.' }], timestamp: BASE_TIME + 9e8 } });
    }
  }
  return bigEntries;
}

function running(session: MockSession): MockSession {
  const state = session.snapshot.state as Record<string, unknown>;
  session.snapshot.state = { ...state, isStreaming: true };
  return session;
}

function bigSession(): MockSession {
  const all = big();
  const page = all.slice(-PAGE);
  const s = genSession(BIG, { turns: 0 });
  s.snapshot.entries = page;
  s.snapshot.leaf_id = page.at(-1)!.id;
  (s.snapshot as Record<string, unknown>).page = { total: all.length, has_more: true, cursor: page[0]!.id };
  s.cmd = {
    get_entries: (cmd) => {
      const before = typeof cmd.before === 'string' ? cmd.before : undefined;
      const limit = typeof cmd.limit === 'number' ? cmd.limit : PAGE;
      if (before) {
        const at = all.findIndex((e) => e.id === before);
        const from = Math.max(0, at - limit);
        const entries = at > 0 ? all.slice(from, at) : [];
        return { entries, total: all.length, hasMore: from > 0, cursor: entries[0]?.id ?? null };
      }
      return { entries: all.slice(-limit), leafId: all.at(-1)!.id, total: all.length, hasMore: true, cursor: all.at(-limit)!.id };
    },
  };
  return running(s);
}

function hiddenSession(id: string, i: number): MockSession {
  const s = genSession(id, { seed: 100 + i, turns: 20, toolsPerTurn: 2, thinking: true, cwd: '/home/you/projects/oino' });
  const entries = s.snapshot.entries as Entry[];
  const last = entries.at(-1)!;
  entries.push({ type: 'message', id: `${id}-prompt`, parentId: last.id, timestamp: new Date(BASE_TIME + 9e8).toISOString(), message: { role: 'user', content: [{ type: 'text', text: 'Keep going.' }], timestamp: BASE_TIME + 9e8 } });
  s.snapshot.leaf_id = `${id}-prompt`;
  return running(s);
}

/** One chat's live run, pushed like the host does (one frame per event, seq +1 each). */
function driver(world: MockWorld, sessionId: string): () => void {
  let seq = 0;
  let token = 0;
  let n = 0;
  let msgTs = 0;
  let text = '';
  let call = 0;
  const meta = { api: 'anthropic-messages', provider: 'anthropic', model: MODEL };
  const push = (event: Record<string, unknown>) => {
    seq += 1;
    world.push(HOST, sessionId, JSON.parse(JSON.stringify([{ session: sessionId, seq, event }])));
  };
  const start = () => {
    msgTs = Date.now();
    text = '';
    n = 0;
    token = 0;
    push({ type: 'message_start', message: { role: 'assistant', content: [], ...meta, timestamp: msgTs } });
  };
  const usage = { input: 2100, output: 400, cacheRead: 0, cacheWrite: 0, totalTokens: 2500, cost: { total: 0.01 } };
  const tick = () => {
    if (n >= TOKENS_PER_MESSAGE) {
      call += 1;
      const id = `call_${sessionId}_${call}`;
      const tool = { type: 'toolCall', id, name: 'bash', arguments: { command: 'npm test -- --run' } };
      push({ type: 'message_end', message: { role: 'assistant', content: [{ type: 'text', text }, tool], ...meta, stopReason: 'toolUse', timestamp: msgTs, usage } });
      push({ type: 'tool_execution_start', toolCallId: id, toolName: 'bash', args: tool.arguments });
      const out = Array.from({ length: 40 }, (_, k) => ` ✓ src/lib/mod${k}.test.ts (${k + 3})`).join('\n');
      push({ type: 'tool_execution_update', toolCallId: id, toolName: 'bash', args: tool.arguments, partialResult: { content: [{ type: 'text', text: out }] } });
      push({ type: 'tool_execution_end', toolCallId: id, toolName: 'bash', result: { content: [{ type: 'text', text: out }] }, isError: false, durationMs: 900 });
      push({ type: 'message_end', message: { role: 'toolResult', toolCallId: id, toolName: 'bash', content: [{ type: 'text', text: out }], isError: false, timestamp: Date.now() } });
      start();
      return;
    }
    const delta = TOKENS[token++ % TOKENS.length]!;
    text += delta;
    n += 1;
    const message = { role: 'assistant', content: [{ type: 'text', text }], ...meta, timestamp: msgTs };
    push({ type: 'message_update', message, assistantMessageEvent: { type: 'text_delta', contentIndex: 0, delta, partial: message } });
  };
  push({ type: 'agent_start' });
  start();
  const timer = setInterval(tick, TOKEN_MS);
  return () => clearInterval(timer);
}

type PerfWindow = Window & { __UNIPI_MOCK_WORLD__?: MockWorld; __PERF_STREAM__?: { stop(): void; start(chats?: number): void } };

let hostsCache: MockScenario['hosts'] | undefined;

export const perfStream: MockScenario = {
  name: 'perf-stream',
  get hosts() {
    return (hostsCache ??= [
      {
        id: HOST,
        name: 'coffee',
        conn: { state: 'connected' },
        cards: [
          { session_id: BIG, name: 'Profile the timeline (40k entries)', cwd: '/home/you/projects/unipi', run_state: 'working', updated_at: BASE_TIME, message_count: ENTRIES },
          ...genCards(9).map((c, i) => ({ ...c, session_id: HIDDEN[i]!, run_state: 'working' as const })),
        ],
        fs: genFsTree(),
        sessions: { [BIG]: bigSession(), ...Object.fromEntries(HIDDEN.map((id, i) => [id, hiddenSession(id, i)])) },
      },
    ]);
  },
  setup: () => {
    localStorage.setItem('unipi.shell.last', JSON.stringify({ hostId: HOST, sessionId: BIG }));
    const chats = Number(new URLSearchParams(location.hash.split('?')[1] ?? location.search).get('chats') ?? 10);
    if (!location.hash.startsWith(`#/s/${HOST}/${BIG}`)) location.hash = `#/s/${HOST}/${BIG}`;
    const w = window as PerfWindow;
    let stops: Array<() => void> = [];
    const start = (chats = 10) => {
      const world = w.__UNIPI_MOCK_WORLD__;
      if (!world) return;
      stops.forEach((s) => s());
      stops = [BIG, ...HIDDEN].slice(0, chats).map((id) => driver(world, id));
    };
    w.__PERF_STREAM__ = { start, stop: () => (stops.forEach((s) => s()), (stops = [])) };
    // Deferred past the factory's import cycle (see modal-info): hold the hidden chats open.
    queueMicrotask(() => {
      void Promise.all([import('solid-js'), import('../../../lib/session/store')]).then(([{ batch }, { sessions }]) => {
        // In one batch: `ensure` bumps the registry signal, and a reader reacting to it
        // synchronously (the header's chat actions) would evict the chat before it is pinned.
        batch(() => {
          for (const id of HIDDEN) sessions().ensure(HOST, id).acquire();
        });
        (w as PerfWindow & { __PERF_SESSIONS__?: unknown }).__PERF_SESSIONS__ = sessions;
        // Streams open on a microtask; start once every snapshot is in.
        setTimeout(() => start(chats), 1500);
      });
    });
  },
};
