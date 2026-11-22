/**
 * WP-A8 scenarios: `tools-mix` (every native body + row state), `tools-groups`
 * (Z-38 Execute/Explored/Changed groups, one live), `renderer-cards`
 * (extension renderers: every U7b card type through `_oino/render`, one
 * native fallback, one that never answers — the 2 s timeout path).
 */
import type { MockScenario } from '../host';
import type { Entry } from '../gen';
import { BASE_TIME, genCards, genFsTree } from '../gen';
import cards from '../fixtures/cards.json';

const TINY_PNG = 'iVBORw0KGgoAAAANSEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==';

/** Deterministic entry builder (ids `eNNN`, timestamps step 3 s so "Thought for 3s" is stable). */
function chain() {
  const out: Entry[] = [];
  let n = 0;
  let parent: string | null = null;
  let t = BASE_TIME;
  const add = (e: Record<string, unknown> & { type: string }): Entry => {
    n += 1;
    t += 3000;
    const entry = { ...e, id: `e${String(n).padStart(3, '0')}`, parentId: parent, timestamp: new Date(t).toISOString() } as Entry;
    parent = entry.id;
    out.push(entry);
    return entry;
  };
  return {
    out,
    entry: add,
    model: (modelId: string) => add({ type: 'model_change', provider: 'anthropic', modelId }),
    user: (text: string) => add({ type: 'message', message: { role: 'user', content: [{ type: 'text', text }], timestamp: t } }),
    /** thinking + toolCall parts; result as its own toolResult entry. */
    assistant: (parts: unknown[], extra: Record<string, unknown> = {}) =>
      add({ type: 'message', message: { role: 'assistant', content: parts, provider: 'anthropic', model: 'claude-sonnet-4-5', stopReason: 'toolUse', usage: { input: 2000, output: 100, totalTokens: 2100, cost: { total: 0.009 } }, timestamp: t, ...extra } }),
    answer: (text: string) =>
      add({ type: 'message', message: { role: 'assistant', content: [{ type: 'text', text }], provider: 'anthropic', model: 'claude-sonnet-4-5', stopReason: 'stop', usage: { input: 2400, output: 80, totalTokens: 2480, cost: { total: 0.008 } }, timestamp: t } }),
    think: (thinking: string) => ({ type: 'thinking', thinking }),
    call: (id: string, name: string, args: Record<string, unknown>) => ({ type: 'toolCall', id, name, arguments: args }),
    result: (id: string, name: string, content: unknown[], isError = false, details?: unknown) =>
      add({ type: 'message', message: { role: 'toolResult', toolCallId: id, toolName: name, content, isError, ...(details !== undefined ? { details } : {}), timestamp: t } }),
    text: (text: string) => ({ type: 'text', text }),
    custom: (customType: string, data: Record<string, unknown>, kind: 'custom' | 'custom_message' = 'custom') =>
      add(kind === 'custom' ? { type: 'custom', customType, data } : { type: 'custom_message', customType, data }),
    bashExec: (command: string, output: string, exitCode = 0) =>
      add({ type: 'message', message: { role: 'bashExecution', content: `$ ${command}\n${output}`, timestamp: t, details: { command, exitCode, output } } }),
    step: (name: string, arg: string) => add({ type: 'custom', customType: 'subagent-step', data: { kind: 'subagent-step', name, arg } }),
  };
}

function snapshotOf(id: string, entries: Entry[], streaming = false) {
  return {
    session_id: id,
    reset: true,
    seq: 0,
    state: { sessionId: id, isStreaming: streaming, isCompacting: false, model: { provider: 'anthropic', id: 'claude-sonnet-4-5' }, thinkingLevel: 'medium', messageCount: entries.length, pendingMessageCount: 0 },
    entries,
    leaf_id: entries.at(-1)?.id ?? null,
    commands: [],
    ui: { scope: 'session' as const, sessionId: id, rev: 0, entries: [], dialogs: [] },
    topics: [],
  };
}

/* ── tools-mix: every native body and row state ─────────────────────── */
function mixSession(id: string, live = false) {
  const c = chain();
  c.model('claude-sonnet-4-5');
  c.user('Ship the fix and tidy the docs');
  // thinking with a real entry pair after it → "Thought for 3s"
  c.assistant([c.think('Check the failing test first, then edit, then rerun the suite.'), c.call('t1', 'bash', { command: 'npm test -- --run' })]);
  c.result('t1', 'bash', [c.text(' RUN  v5.0.3\n ✓ src/cli.test.ts (8)\n × src/math.test.ts > add (1)\n   expected -1 to be 3\n')]);
  // edit with a diff
  c.assistant([c.call('t2', 'edit', { path: 'src/math.ts', edits: [{ oldText: 'export function add(a, b) {\n  return a - b;\n}', newText: 'export function add(a: number, b: number): number {\n  return a + b;\n}' }] })]);
  c.result('t2', 'edit', [c.text('Successfully replaced 1 block in src/math.ts.')], false, { firstChangedLine: 12 });
  // write a markdown file
  c.assistant([c.call('t3', 'write', { path: 'docs/NOTES.md', content: '# Notes\n\n- `add` adds now\n- suite green\n' })]);
  c.result('t3', 'write', [c.text('Wrote docs/NOTES.md.')]);
  // reads / search / web / memory heads
  c.assistant([c.call('t4', 'read', { path: 'src/math.test.ts' })]);
  c.result('t4', 'read', [c.text('import { add } from "./math";\n\ntest("add", () => {\n  expect(add(1, 2)).toBe(3);\n});\n')]);
  c.assistant([c.call('t5', 'grep', { pattern: 'parseArgs', path: 'src' })]);
  c.result('t5', 'grep', [c.text('src/cli.ts:12:  const flags = parseArgs(argv);\nsrc/cli.test.ts:4:import { parseArgs } from "./cli";')]);
  c.assistant([c.call('t6', 'web_search', { query: 'solidjs store produce best practice' })]);
  c.result('t6', 'web_search', [c.text('solid.dev/guides — prefer produce() for fine-grained updates…')]);
  c.assistant([c.call('t7', 'memory_search', { query: 'reducer conventions' })]);
  c.result('t7', 'memory_search', [c.text('This repo keeps tests next to sources; benches are separate.')]);
  // todo + mcp + ask
  c.assistant([c.call('t8', 'todo_write', { todos: [
    { content: 'Fix the add test', status: 'completed' },
    { content: 'Run the suite again', status: 'in_progress', activeForm: 'Running the suite' },
    { content: 'Update the README', status: 'pending' },
  ] })]);
  c.result('t8', 'todo_write', [c.text('Todos updated.')]);
  c.assistant([c.call('t9', 'mcp__github__search_issues', { q: 'repo:owner/repo is:open label:bug' })]);
  c.result('t9', 'mcp__github__search_issues', [c.text('{ "count": 2, "items": ["#41", "#37"] }')]);
  c.assistant([c.call('t10', 'ask_user', { questions: [{ question: 'Should I also bump the version?' }] })]);
  c.result('t10', 'ask_user', [c.text('Yes, patch bump.')]);
  // an errored run
  c.assistant([c.call('t11', 'bash', { command: 'npm run release' })]);
  c.result('t11', 'bash', [c.text('npm ERR! Missing required flag --otp\nAsk the owner for the 2FA code.')], true);
  // image in a tool result (Z-44)
  c.assistant([c.call('t12', 'screenshot', { url: 'http://127.0.0.1:1450' })]);
  c.result('t12', 'screenshot', [c.text('Captured 412×915.'), { type: 'image', mimeType: 'image/png', data: TINY_PNG }]);
  // a delegated step + user bash
  c.step('explore', 'Map the reducer call sites');
  c.bashExec('git log --oneline -3', 'a1b2c3 fix add\n d4e5f6 docs\n 789abc bench');
  if (!live) c.answer('Done — `add` adds, the suite is green (12 tests) and `docs/NOTES.md` records it.');
  return snapshotOf(id, c.out, live);
}

/* ── tools-groups: Execute / Explored / Changed groups ──────────────── */
function groupsSession(id: string, live: boolean) {
  const c = chain();
  c.user('Explore, then change, then run the checks');
  const read = (i: number, file: string) => {
    c.assistant([c.call(`r${i}`, 'read', { path: file })]);
    c.result(`r${i}`, 'read', [c.text(`// ${file}\nexport const ${i} = ${i};\n`)]);
  };
  read(1, 'src/a.ts');
  read(2, 'src/b.ts');
  c.assistant([c.call('g1', 'grep', { pattern: 'TODO', path: 'src' })]);
  c.result('g1', 'grep', [c.text('src/b.ts:4:// TODO tidy')]);
  c.assistant([c.call('e1', 'edit', { path: 'src/b.ts', edits: [{ oldText: '// TODO tidy', newText: '// tidied' }] })]);
  c.result('e1', 'edit', [c.text('Replaced 1 block.')]);
  c.assistant([c.call('w1', 'write', { path: 'src/c.ts', content: 'export const c = 3;\n' })]);
  c.result('w1', 'write', [c.text('Wrote src/c.ts.')]);
  c.assistant([c.call('x1', 'bash', { command: 'npm test -- --run' })]);
  c.result('x1', 'bash', [c.text('✓ 12 tests')]);
  c.assistant([c.call('x2', 'bash', { command: 'npm run typecheck' })]);
  c.result('x2', 'bash', [c.text('clean')]);
  c.assistant([c.call('x3', 'bash', { command: 'npm run lint' })]);
  c.result('x3', 'bash', [c.text('clean')]);
  if (live) {
    // leave one tool preparing: the run is live (state.isStreaming), the fold
    // stays open and its groups default open (Z-38)
    c.assistant([c.call('x4', 'bash', { command: 'npm run build' })]);
  } else {
    c.answer('Explored two files, tidied one, wrote one, and all checks pass.');
  }
  return snapshotOf(id, c.out, live);
}

/* ── renderer-cards: every U7b card type + fallback + timeout ───────── */
const HARNESS_TYPES = [
  'unipi:lh-continue',
  'unipi:kanboard-continue',
  'unipi:kanboard-next',
  'unipi-memory-recall-reminder',
  'unipi-memory-retro-reminder',
  'unipi-skills-revealed',
  'unipi:plan-mode-message',
  'compactor-recall',
  'unipi-response',
];
const CARD_TYPES: Array<[string, keyof typeof cards]> = [
  ...HARNESS_TYPES.map((t) => [t, 'harnessCard'] as [string, keyof typeof cards]),
  ['unipi-watchdog', 'harnessWarning'],
  ['subagent-completion', 'subagentCompletion'],
  ['sidekick-completion', 'sidekickCompletion'],
  ['background-task-notification', 'backgroundTask'],
  ['unipi-memory-session-card', 'memorySession'],
  ['unipi-memory-save-card', 'memorySave'],
  ['unipi-compaction', 'compaction'],
  ['unipi:kanboard-badge', 'kanboardBadge'],
];

function rendererSession(id: string) {
  const c = chain();
  // Cards *before* the first user message form the preamble, which renders
  // its work loose (no fold) — so captures show every card type directly.
  for (const [type] of CARD_TYPES) {
    c.custom(type, { note: `${type} fixture entry`, title: type, text: 'Card body text.' }, 'custom_message');
  }
  // native fallback (no renderer registered) and the timeout path (never answers)
  c.custom('acme:report', { title: 'Acme weekly report', text: 'Native row until an owner registers.' }, 'custom_message');
  c.custom('acme:stuck', { title: 'This one never answers' }, 'custom_message');
  // a custom *entry* (target customEntry)
  c.custom('unipi:attachments', { items: [{ name: 'cat.png', kind: 'image', bytes: 12288 }] });
  c.user('Show me every card the extensions render');
  c.answer('Every card type, one native fallback and one that never answers.');
  return snapshotOf(id, c.out);
}

const renderers: Record<string, unknown> = {
  ...Object.fromEntries(CARD_TYPES.map(([type, fixture]) => [`customMessage/${type}`, (cards as Record<string, unknown>)[fixture]])),
  'customEntry/unipi:attachments': cards.attachments,
  'tool/bash': cards.toolBashAdvanced,
  'customMessage/acme:stuck': { never: true },
};

export const transcriptFinal: MockScenario = {
  name: 'transcript-final',
  hosts: [{ id: 'pc-coffee', name: 'coffee', conn: { state: 'connected' }, cards: genCards(1), fs: genFsTree(), sessions: { s001: { snapshot: (() => {
    const c = chain();
    c.user('Check the transcript fix');
    c.assistant([c.think('Inspect the turn grouping.'), c.call('t1', 'bash', { command: 'npm test' })]);
    c.result('t1', 'bash', [c.text('All tests passed.')]);
    c.assistant([c.text('The transcript fix is complete. The final answer stays visible when work is collapsed.\n\n```ts\nconst answer = "always visible";\n```\n\nThe trailing thought is separate.'), c.think('One last check after the answer.')], { stopReason: 'stop' });
    return snapshotOf('s001', c.out);
  })() } } }],
};

export const toolsMix: MockScenario = {
  name: 'tools-mix',
  hosts: [
    {
      id: 'pc-coffee',
      name: 'coffee',
      conn: { state: 'connected' },
      cards: genCards(4, { prefix: 'tm' }),
      fs: genFsTree(),
      sessions: {
        s001: { snapshot: mixSession('s001') },
        // The live variant: the run is still going, so the fold is forced open
        // (Z-56) with its groups open — the full row mix visible without any
        // capture-only state seeding.
        s002: { snapshot: mixSession('s002', true) },
      },
    },
  ],
};

export const toolsGroups: MockScenario = {
  name: 'tools-groups',
  hosts: [{ id: 'pc-coffee', name: 'coffee', conn: { state: 'connected' }, cards: genCards(3, { prefix: 'tg' }), fs: genFsTree(), sessions: { s001: { snapshot: groupsSession('s001', false) }, s002: { snapshot: groupsSession('s002', true) } } }],
};

export const rendererCards: MockScenario = {
  name: 'renderer-cards',
  hosts: [{ id: 'pc-coffee', name: 'coffee', conn: { state: 'connected' }, cards: genCards(3, { prefix: 'rc' }), fs: genFsTree(), renderers, sessions: { s001: { snapshot: rendererSession('s001') } } }],
};
