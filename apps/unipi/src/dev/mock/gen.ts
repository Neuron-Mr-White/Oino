/**
 * Deterministic mock data generators (app spec §3.15, WP-A1): sessions with
 * N turns, tool mixes, diffs, thinking, images, steers, compaction, errors,
 * subagents — as oino session entries (pi's session entry format, which oino
 * keeps), plus session cards and fs trees. Same seed ⇒ same output, so
 * captures and component tests are stable. Later WPs extend the options.
 */
import type { SessionCard, SessionEventDto } from '../../lib/host/client';
import type { MockFsNode, MockSession } from './host';
import recMath from './fixtures/rec-math.json';

/** mulberry32: tiny seeded PRNG. */
export function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const pick = <T,>(r: () => number, items: readonly T[]): T => items[Math.floor(r() * items.length)]!;

/** A fixed clock (2026-10-30 09:00 UTC) so timestamps never depend on "now". */
export const BASE_TIME = Date.UTC(2026, 9, 30, 9, 0, 0);

export type Entry = Record<string, unknown> & { type: string; id: string; parentId: string | null; timestamp: string };

export type GenOptions = {
  seed?: number;
  /** Number of user→assistant turns. */
  turns?: number;
  cwd?: string;
  /** Tool calls per turn (0 = none). */
  toolsPerTurn?: number;
  thinking?: boolean;
  /** Include an `edit` tool with a unified diff on every other turn. */
  diffs?: boolean;
  /** Attach an image to the first user message. */
  images?: boolean;
  /** A steer (user message mid-run) in turn 2. */
  steers?: boolean;
  /** A compaction entry after the first half. */
  compaction?: boolean;
  /** The last assistant message ends in an error. */
  error?: boolean;
  /** A subagent run (custom entry) in turn 1. */
  subagents?: boolean;
  model?: string;
};

const PROMPTS = [
  'Fix the failing test in src/math.test.ts',
  'Add a --json flag to the CLI and document it',
  'Why is the build slow? Profile it and suggest fixes',
  'Rename `parseArgs` to `parseFlags` across the repo',
  'Write a short README section about configuration',
  'Refactor the reducer so the tail turn recomputes alone',
];
const TOOLS = ['read', 'bash', 'grep', 'find', 'edit', 'write'] as const;
const FILES = ['src/math.ts', 'src/cli.ts', 'src/reducer.ts', 'README.md', 'package.json', 'src/lib/turns.ts'];

/** A generated session as entries (current branch, oldest first). */
export function genEntries(opts: GenOptions = {}): Entry[] {
  const r = rng(opts.seed ?? 1);
  const turns = opts.turns ?? 3;
  const toolsPerTurn = opts.toolsPerTurn ?? 2;
  const model = opts.model ?? 'claude-sonnet-4-5';
  const out: Entry[] = [];
  let n = 0;
  let parent: string | null = null;
  let t = BASE_TIME;
  const add = (e: Record<string, unknown> & { type: string }): Entry => {
    n += 1;
    t += 1000 + Math.floor(r() * 4000);
    const entry = { ...e, id: `e${String(n).padStart(4, '0')}`, parentId: parent, timestamp: new Date(t).toISOString() } as Entry;
    parent = entry.id;
    out.push(entry);
    return entry;
  };
  const msg = (message: Record<string, unknown>) => add({ type: 'message', message: { timestamp: t, ...message } });
  add({ type: 'model_change', provider: 'anthropic', modelId: model });
  for (let turn = 0; turn < turns; turn++) {
    const content: unknown[] = [{ type: 'text', text: PROMPTS[(turn + (opts.seed ?? 0)) % PROMPTS.length] }];
    if (opts.images && turn === 0) content.push({ type: 'image', mimeType: 'image/png', data: TINY_PNG });
    msg({ role: 'user', content });
    if (opts.subagents && turn === 1) {
      add({ type: 'custom', customType: 'subagent-run', data: { id: `sa-${turn}`, agent: 'explore', task: 'Map the reducer call sites', status: 'done', turns: 4 } });
    }
    for (let k = 0; k < toolsPerTurn; k++) {
      const tool = opts.diffs && k === 0 && turn % 2 === 1 ? 'edit' : pick(r, TOOLS);
      const file = pick(r, FILES);
      const id = `call_${turn}_${k}`;
      const args = toolArgs(tool, file);
      const blocks: unknown[] = [];
      if (opts.thinking && k === 0) blocks.push({ type: 'thinking', thinking: `Let me look at ${file} first, then decide.` });
      blocks.push({ type: 'toolCall', id, name: tool, arguments: args });
      msg({ role: 'assistant', content: blocks, provider: 'anthropic', model, stopReason: 'toolUse', usage: usage(r) });
      msg({ role: 'toolResult', toolCallId: id, toolName: tool, content: [{ type: 'text', text: toolOutput(tool, file, r) }], isError: false });
      if (opts.steers && turn === 1 && k === 0) msg({ role: 'user', content: [{ type: 'text', text: 'Also keep the public API unchanged.' }] });
    }
    const last = turn === turns - 1;
    if (opts.error && last) {
      msg({ role: 'assistant', content: [], provider: 'anthropic', model, stopReason: 'error', errorMessage: 'overloaded_error: the model is overloaded, try again', usage: usage(r) });
    } else {
      const blocks: unknown[] = [];
      if (opts.thinking && toolsPerTurn === 0) blocks.push({ type: 'thinking', thinking: 'Short answer is enough here.' });
      blocks.push({ type: 'text', text: summary(turn) });
      msg({ role: 'assistant', content: blocks, provider: 'anthropic', model, stopReason: 'stop', usage: usage(r) });
    }
    if (opts.compaction && turn === Math.floor(turns / 2) - 1) {
      add({ type: 'compaction', summary: 'Earlier: fixed the failing math test and added the --json flag.', firstKeptEntryId: parent, tokensBefore: 48210 });
    }
  }
  return out;
}

function toolArgs(tool: string, file: string): Record<string, unknown> {
  switch (tool) {
    case 'bash':
      return { command: pickCommand(file) };
    case 'grep':
      return { pattern: 'parseArgs', path: 'src' };
    case 'find':
      return { pattern: '*.test.ts' };
    case 'edit':
      return { path: file, edits: [{ oldText: 'export function add(a, b) {\n  return a - b;\n}', newText: 'export function add(a: number, b: number): number {\n  return a + b;\n}' }] };
    case 'write':
      return { path: file, content: '# Configuration\n\nSet `UNIPI_HOME` to move the data folder.\n' };
    default:
      return { path: file };
  }
}

const pickCommand = (file: string) => (file.endsWith('.json') ? 'npm test -- --run' : `wc -l ${file}`);

function toolOutput(tool: string, file: string, r: () => number): string {
  switch (tool) {
    case 'bash':
      return `${10 + Math.floor(r() * 300)} ${file}`;
    case 'grep':
      return 'src/cli.ts:12:  const flags = parseArgs(argv);\nsrc/cli.test.ts:4:import { parseArgs } from "./cli";';
    case 'find':
      return 'src/math.test.ts\nsrc/cli.test.ts';
    case 'edit':
      return `Successfully replaced 1 block in ${file}.`;
    case 'write':
      return `Wrote ${file}.`;
    default:
      return `export function add(a, b) {\n  return a - b;\n}\n// ${file}`;
  }
}

function summary(turn: number): string {
  return [
    'Done. The test failed because `add` subtracted; it now returns `a + b` and all 12 tests pass.',
    'Added `--json`:\n\n- prints the result as one JSON object\n- documented in `README.md`\n\n```ts\nif (flags.json) console.log(JSON.stringify(result));\n```',
    'The build spends 70% in type-checking. Turning on `incremental` and splitting the test project cuts it from 41 s to 12 s.',
  ][turn % 3]!;
}

function usage(r: () => number) {
  const input = 1200 + Math.floor(r() * 8000);
  const output = 80 + Math.floor(r() * 900);
  return { input, output, cacheRead: Math.floor(input * 0.6), cacheWrite: 0, totalTokens: input + output, cost: { total: Number(((input * 3 + output * 15) / 1e6).toFixed(4)) } };
}

/** A generated session for the mock host: snapshot (+ optional live script). */
export function genSession(id: string, opts: GenOptions = {}): MockSession {
  const entries = genEntries(opts);
  return {
    snapshot: {
      session_id: id,
      reset: true,
      seq: 0,
      state: { sessionId: id, isStreaming: false, isCompacting: false, model: { provider: 'anthropic', id: opts.model ?? 'claude-sonnet-4-5' }, thinkingLevel: 'medium', messageCount: entries.length, pendingMessageCount: 0 },
      entries,
      leaf_id: entries.at(-1)?.id ?? null,
      commands: [],
      ui: { scope: 'session', sessionId: id, rev: 0, entries: [], dialogs: [] },
      topics: [],
    },
    events: [],
  };
}

/** The recorded real session (archive rec-math) as a mock session. */
export function recMathSession(): MockSession {
  return recMath as unknown as MockSession;
}

/** `n` session cards spread over a few project folders (host.md §4.2 `SessionCard`). */
export function genCards(n: number, opts: { seed?: number; root?: string } = {}): SessionCard[] {
  const r = rng(opts.seed ?? 7);
  const root = opts.root ?? '/home/you/projects';
  const folders = ['unipi', 'oino', 'website'];
  const states: SessionCard['run_state'][] = ['idle', 'idle', 'working', 'idle'];
  return Array.from({ length: n }, (_, i) => ({
    session_id: `s${String(i + 1).padStart(3, '0')}`,
    name: PROMPTS[i % PROMPTS.length],
    cwd: `${root}/${folders[i % folders.length]}`,
    run_state: pick(r, states),
    updated_at: BASE_TIME - i * 3_600_000,
    message_count: 4 + i * 2,
  }));
}

/** A small project tree for fs ops. */
export function genFsTree(root = '/home/you/projects/unipi'): Record<string, MockFsNode> {
  const file = (text: string): MockFsNode => ({ kind: 'file', text, mtime: BASE_TIME });
  return {
    [root]: {
      kind: 'dir',
      children: {
        src: { kind: 'dir', children: { 'math.ts': file('export function add(a: number, b: number): number {\n  return a + b;\n}\n'), 'cli.ts': file('export {};\n') } },
        'README.md': file('# unipi\n\nThe UniPi app.\n'),
        'package.json': file('{ "name": "unipi" }\n'),
      },
    },
  };
}

/** Wraps ClientEvent payloads into a timed script (`speed` in the scenario replays it). */
export function script(session: string, events: Array<{ at: number; event: SessionEventDto['event'] }>): Array<{ at: number; dto: SessionEventDto }> {
  return events.map((e, i) => ({ at: e.at, dto: { session, seq: i + 1, event: e.event } }));
}

/** 1×1 transparent PNG. */
const TINY_PNG = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==';

export type LiveRunOptions = {
  /** First event seq (the snapshot's seq + 1). */
  seq?: number;
  /** Virtual start time (ms) of the run in the script. */
  at?: number;
  prompt?: string;
  /** A steer that arrives after the first tool result (Z-54). */
  steer?: string;
  /** Leave the run going (no final answer / agent_end). */
  running?: boolean;
  model?: string;
};

/**
 * A live run as oino `ClientEvent`s (crates/oino-core/src/events.rs wire
 * shapes): user prompt → streamed thinking + a tool call → tool execution →
 * result → (steer) → streamed answer → `agent_end`. For the session stream
 * scenarios (WP-A2) and component tests.
 */
export function genLiveRun(session: string, opts: LiveRunOptions = {}): Array<{ at: number; dto: SessionEventDto }> {
  let seq = opts.seq ?? 1;
  let at = opts.at ?? 0;
  const model = opts.model ?? 'claude-sonnet-4-5';
  const out: Array<{ at: number; dto: SessionEventDto }> = [];
  const ev = (event: SessionEventDto['event'], dt = 120) => {
    at += dt;
    out.push({ at, dto: { session, seq: seq++, event } });
  };
  const ts = () => BASE_TIME + 86_400_000 + at;
  const meta = { api: 'anthropic-messages', provider: 'anthropic', model };
  const user = (text: string) => ({ role: 'user', content: [{ type: 'text', text }], timestamp: ts() });
  const prompt = user(opts.prompt ?? 'Run the tests and fix what fails');
  ev({ type: 'agent_start' }, 0);
  ev({ type: 'message_start', message: prompt });
  ev({ type: 'message_end', message: prompt });
  const call = { type: 'toolCall', id: 'call_live_1', name: 'bash', arguments: { command: 'npm test -- --run' } };
  const a1 = { role: 'assistant', content: [] as unknown[], ...meta, stopReason: 'toolUse', timestamp: ts(), usage: { input: 2100, output: 90, cacheRead: 0, cacheWrite: 0, totalTokens: 2190, cost: { total: 0.0076 } } };
  ev({ type: 'message_start', message: a1 }, 400);
  let thinking = '';
  for (const piece of ['Run the suite first, ', 'then read the failing ', 'test before editing.']) {
    thinking += piece;
    ev({ type: 'message_update', message: { role: 'assistant', timestamp: a1.timestamp, ...meta }, assistantMessageEvent: { type: 'thinking_delta', contentIndex: 0, delta: piece } }, 180);
  }
  ev({ type: 'message_update', message: { role: 'assistant', timestamp: a1.timestamp, ...meta }, assistantMessageEvent: { type: 'toolcall_end', contentIndex: 1, toolCall: call } });
  const a1End = { ...a1, content: [{ type: 'thinking', thinking }, call] };
  ev({ type: 'message_end', message: a1End });
  ev({ type: 'tool_execution_start', toolCallId: call.id, toolName: 'bash', args: call.arguments });
  ev({ type: 'tool_execution_update', toolCallId: call.id, toolName: 'bash', args: call.arguments, partialResult: { content: [{ type: 'text', text: ' RUN  v5.0.3\n ✓ src/cli.test.ts (8)\n' }] } }, 600);
  const output = ' RUN  v5.0.3\n ✓ src/cli.test.ts (8)\n × src/math.test.ts > add (1)\n   expected -1 to be 3\n';
  ev({ type: 'tool_execution_end', toolCallId: call.id, toolName: 'bash', result: { content: [{ type: 'text', text: output }] }, isError: false, durationMs: 1400 }, 900);
  ev({ type: 'message_end', message: { role: 'toolResult', toolCallId: call.id, toolName: 'bash', content: [{ type: 'text', text: output }], isError: false, timestamp: ts() } });
  if (opts.steer) {
    const steer = user(opts.steer);
    ev({ type: 'queue_update', steering: [opts.steer], followUp: [] }, 200);
    ev({ type: 'queue_update', steering: [], followUp: [] }, 200);
    ev({ type: 'message_start', message: steer });
    ev({ type: 'message_end', message: steer });
  }
  if (opts.running) return out;
  const a2 = { role: 'assistant', content: [] as unknown[], ...meta, stopReason: 'stop', timestamp: ts(), usage: { input: 2600, output: 60, cacheRead: 1800, cacheWrite: 0, totalTokens: 2660, cost: { total: 0.0069 } } };
  ev({ type: 'message_start', message: a2 }, 500);
  let text = '';
  for (const piece of ['`add` subtracted. ', 'It now returns `a + b`; ', 'all 9 tests pass.']) {
    text += piece;
    ev({ type: 'message_update', message: { role: 'assistant', timestamp: a2.timestamp, ...meta }, assistantMessageEvent: { type: 'text_delta', contentIndex: 0, delta: piece } }, 220);
  }
  ev({ type: 'message_end', message: { ...a2, content: [{ type: 'text', text }] } });
  ev({ type: 'agent_end', messages: [] });
  return out;
}
