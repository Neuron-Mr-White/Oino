import { describe, expect, it } from 'vitest';
import { buildRows, collapseCommandRows, injectedBody, isHiddenCommandText, diffStat, formatDuration, liveActivity, processSummary, thinkingPreview, toolSubject, type Entry, type LiveTools, type Row } from './model';
import { applyMessage, PENDING_LOST_MS, sweepPending, type ChatState } from './store';

const user = (id: string, text: string, ts = 1000): Entry => ({ type: 'message', id, message: { role: 'user', content: text, timestamp: ts } });
const asst = (id: string, content: unknown[], stopReason = 'stop', ts = 5000, cost = 0.01): Entry => ({
  type: 'message',
  id,
  message: { role: 'assistant', content, stopReason, timestamp: ts, usage: { cost: { total: cost } } } as never,
});
const result = (id: string, callId: string, text: string, isError = false): Entry => ({
  type: 'message',
  id,
  message: { role: 'toolResult', toolCallId: callId, toolName: 'bash', content: [{ type: 'text', text }], isError } as never,
});
const resultWithDetails = (id: string, callId: string, text: string, details: unknown): Entry => ({
  type: 'message',
  id,
  message: { role: 'toolResult', toolCallId: callId, toolName: 'edit', content: [{ type: 'text', text }], details } as never,
});

describe('buildRows', () => {
  it('folds thinking + tool calls of a turn into one process group before the prose', () => {
    const entries = [
      user('u1', 'list files'),
      asst('a1', [{ type: 'thinking', thinking: 'Let me look.' }, { type: 'toolCall', id: 'c1', name: 'bash', arguments: { command: 'ls' } }], 'toolUse', 2000),
      result('r1', 'c1', 'a\nb'),
      asst('a2', [{ type: 'toolCall', id: 'c2', name: 'read', arguments: { path: '/p/x.ts' } }], 'toolUse', 3000),
      result('r2', 'c2', 'code', true),
      asst('a3', [{ type: 'text', text: 'Two files.' }], 'stop', 7000),
    ];
    const rows = buildRows(entries, undefined, {}, false);
    expect(rows.map((r) => r.kind)).toEqual(['user', 'process', 'prose', 'turn_end']);
    const p = rows[1] as Extract<(typeof rows)[number], { kind: 'process' }>;
    expect(p.items.map((i) => i.kind)).toEqual(['thinking', 'tool', 'tool']);
    const tools = p.items.filter((i) => i.kind === 'tool') as Array<{ tool: { state: string; output?: string } }>;
    expect(tools.map((t) => t.tool.state)).toEqual(['ok', 'error']);
    expect(tools[0]!.tool.output).toBe('a\nb');
    const end = rows[3] as { ms?: number; cost?: number };
    expect(end.ms).toBe(6000);
    expect(end.cost).toBeCloseTo(0.03);
  });

  it('turn_end lists the files edited/written this turn with +/- counts', () => {
    const entries = [
      user('u1', 'fix it'),
      asst(
        'a1',
        [
          { type: 'toolCall', id: 'c1', name: 'edit', arguments: { path: '/p/a.ts', edits: [{ oldText: 'x\ny', newText: 'x\nz\nw' }] } },
          { type: 'toolCall', id: 'c2', name: 'write', arguments: { path: '/p/b.ts', content: 'one\ntwo' } },
          { type: 'text', text: 'Done.' },
        ],
        'stop',
        4000,
      ),
      result('r1', 'c1', 'ok'),
      result('r2', 'c2', 'ok'),
    ];
    const rows = buildRows(entries, undefined, {}, false);
    const end = rows.find((r) => r.kind === 'turn_end') as Extract<(typeof rows)[number], { kind: 'turn_end' }>;
    expect(end.files?.map((f) => [f.path, f.add, f.del])).toEqual([
      ['/p/a.ts', 3, 2],
      ['/p/b.ts', 2, 0],
    ]);
    // Each file keeps its own edits, for the per-edit sub-rows (UNI-192).
    expect(end.files?.[0]!.edits).toEqual([{ oldText: 'x\ny', newText: 'x\nz\nw', add: 3, del: 2, tool: expect.objectContaining({ id: 'c1' }) }]);
  });

  it('several edit/write calls against the same path merge into one TurnFile with several edits (UNI-192)', () => {
    const entries = [
      user('u1', 'fix it twice'),
      asst(
        'a1',
        [
          { type: 'toolCall', id: 'c1', name: 'edit', arguments: { path: '/p/a.ts', edits: [{ oldText: 'x', newText: 'y' }] } },
          { type: 'toolCall', id: 'c2', name: 'edit', arguments: { path: '/p/a.ts', edits: [{ oldText: 'p', newText: 'q\nr' }] } },
        ],
        'stop',
        4000,
      ),
      result('r1', 'c1', 'ok'),
      result('r2', 'c2', 'ok'),
    ];
    const rows = buildRows(entries, undefined, {}, false);
    const end = rows.find((r) => r.kind === 'turn_end') as Extract<(typeof rows)[number], { kind: 'turn_end' }>;
    expect(end.files?.map((f) => f.path)).toEqual(['/p/a.ts']);
    const file = end.files![0]!;
    expect(file.add).toBe(3);
    expect(file.del).toBe(2);
    expect(file.edits.map((e) => [e.oldText, e.newText])).toEqual([
      ['x', 'y'],
      ['p', 'q\nr'],
    ]);
  });

  it('a single-edit call picks up the real file line from the edit tool result details.firstChangedLine (UNI-297)', () => {
    const entries = [
      user('u1', 'fix it'),
      asst('a1', [{ type: 'toolCall', id: 'c1', name: 'edit', arguments: { path: '/p/a.ts', edits: [{ oldText: 'x', newText: 'y' }] } }], 'stop', 4000),
      resultWithDetails('r1', 'c1', 'ok', { diff: 'fake', firstChangedLine: 42 }),
    ];
    const rows = buildRows(entries, undefined, {}, false);
    const end = rows.find((r) => r.kind === 'turn_end') as Extract<(typeof rows)[number], { kind: 'turn_end' }>;
    expect(end.files?.[0]!.edits[0]!.realLine).toBe(42);
  });

  it('a multi-edit call (edits[].length > 1) leaves realLine unset — one firstChangedLine can\'t be attributed to any one of them (UNI-297)', () => {
    const entries = [
      user('u1', 'fix it'),
      asst(
        'a1',
        [{ type: 'toolCall', id: 'c1', name: 'edit', arguments: { path: '/p/a.ts', edits: [{ oldText: 'x', newText: 'y' }, { oldText: 'p', newText: 'q' }] } }],
        'stop',
        4000,
      ),
      resultWithDetails('r1', 'c1', 'ok', { diff: 'fake', firstChangedLine: 42 }),
    ];
    const rows = buildRows(entries, undefined, {}, false);
    const end = rows.find((r) => r.kind === 'turn_end') as Extract<(typeof rows)[number], { kind: 'turn_end' }>;
    expect(end.files?.[0]!.edits.map((e) => e.realLine)).toEqual([undefined, undefined]);
  });

  it('a write call\'s one all-additions edit starts at line 1 (UNI-297)', () => {
    const entries = [
      user('u1', 'write it'),
      asst('a1', [{ type: 'toolCall', id: 'c1', name: 'write', arguments: { path: '/p/new.ts', content: 'one\ntwo' } }], 'stop', 4000),
      result('r1', 'c1', 'ok'),
    ];
    const rows = buildRows(entries, undefined, {}, false);
    const end = rows.find((r) => r.kind === 'turn_end') as Extract<(typeof rows)[number], { kind: 'turn_end' }>;
    expect(end.files?.[0]!.edits[0]!.realLine).toBe(1);
  });

  it('streams: live thinking/tool rows, preparing calls, then live prose', () => {
    const entries = [user('u1', 'go')];
    const rows = buildRows(
      entries,
      { id: 's1', parts: [{ type: 'thinking', thinking: 'hmm' }, { type: 'toolCall', id: 'c9', name: 'bash', arguments: '{"comm' }] },
      {},
      true,
    );
    const p = rows[1] as Extract<(typeof rows)[number], { kind: 'process' }>;
    expect(p.live).toBe(true);
    expect((p.items[1] as { tool: { state: string } }).tool.state).toBe('preparing');
    const rows2 = buildRows(entries, { id: 's1', parts: [{ type: 'text', text: 'Hel' }] }, { c9: { name: 'bash', args: { command: 'ls' }, text: 'out' } }, true);
    expect(rows2.map((r) => r.kind)).toEqual(['user', 'prose', 'process']);
    expect((rows2[1] as { live: boolean }).live).toBe(true);
  });

  it('places unipi cards and steps; an aborted reply is marked stopped', () => {
    const entries: Entry[] = [
      user('u1', 'x'),
      { type: 'custom', id: 'k1', customType: 'subagent-step', data: { kind: 'tool', name: 'read' } },
      { type: 'custom_message', id: 'm1', customType: 'subagent-completion', content: 'done', display: true },
      asst('a1', [{ type: 'text', text: 'partial' }], 'aborted'),
      { type: 'custom', id: 'k2', customType: 'unipi-command-echo', data: { text: '/unipi:goal' } },
      { type: 'custom', id: 'k3', customType: 'something-unknown', data: {} },
    ];
    const rows = buildRows(entries, undefined, {}, false);
    // UNI-251: a bare command echo is the user's own bubble (command chip), not a card.
    expect(rows.map((r) => r.kind)).toEqual(['user', 'process', 'card', 'prose', 'turn_end', 'user']);
    expect(rows[5]).toMatchObject({ kind: 'user', text: '/unipi:goal', command: { name: 'unipi:goal' } });
    expect((rows[3] as { stopped?: boolean }).stopped).toBe(true);
    expect((rows[4] as { stopped?: boolean }).stopped).toBe(true);
  });

  it('error replies show an error row', () => {
    const entries = [user('u1', 'x'), { type: 'message', id: 'a', message: { role: 'assistant', content: [], stopReason: 'error', errorMessage: '429 rate limited' } } as Entry];
    expect(buildRows(entries, undefined, {}, false).find((r) => r.kind === 'error')).toMatchObject({ text: '429 rate limited' });
    const aborted = [user('u1', 'x'), { type: 'message', id: 'a', message: { role: 'assistant', content: [], stopReason: 'error', errorMessage: 'This operation was aborted' } } as Entry];
    const rows = buildRows(aborted, undefined, {}, false);
    expect(rows.some((r) => r.kind === 'error')).toBe(false);
    expect(rows.at(-1)).toMatchObject({ kind: 'turn_end', stopped: true });
  });
});

describe('summaries', () => {
  const tool = (name: string, args: Record<string, unknown>, state = 'ok') => ({ kind: 'tool' as const, key: name, tool: { id: name, name, args, state: state as 'ok' } });
  it('live header names the current activity; done header ranks categories', () => {
    expect(processSummary([tool('read', { path: '/w/p/src/a.ts' })], true, '/w/p')).toEqual({ title: 'Reading', detail: 'src/a.ts' });
    expect(processSummary([{ kind: 'thinking', key: 't', text: '', live: true }], true)).toEqual({ title: 'Thinking' });
    const done = processSummary([tool('read', {}), tool('read', {}), tool('bash', {}), tool('edit', {}, 'error'), tool('grep', {}), tool('web_search', {})], false);
    expect(done.title).toBe('Read files, ran commands, edited files, …');
    expect(done.detail).toBe('6 steps · 1 failed');
  });
  it('subjects, previews, diff stats and durations', () => {
    expect(toolSubject({ name: 'bash', args: { command: 'npm test\necho' } })).toBe('npm test');
    expect(toolSubject({ name: 'read', args: { path: '/home/oi/x' } })).toBe('~/x');
    expect(thinkingPreview('First **bold**.\n\nSecond para\nmore', true)).toBe('First bold.');
    expect(thinkingPreview('Only one', false)).toBe('Only one');
    expect(diffStat({ name: 'edit', args: { edits: [{ oldText: 'a\nb', newText: 'c' }] } })).toEqual({ add: 1, del: 2 });
    expect(diffStat({ name: 'write', args: { content: 'x\ny\nz' } })).toEqual({ add: 3, del: 0 });
    expect(formatDuration(63_000)).toBe('1m 3s');
  });
});

describe('applyMessage', () => {
  const fresh = (): ChatState => ({ conn: 'live', run: { running: false }, entries: [], truncated: false, tools: {}, commands: [], models: [], dialogs: [], queue: [], pending: [], hasOlder: false, loadingOlder: false, seams: [], btwPages: [], outbox: [], work: [], infoGroups: [] });
  it('hello → deltas → msg_end → entry', () => {
    const s = fresh();
    applyMessage(s, { t: 'hello', session: { id: 's', cwd: '/p' }, state: { running: true }, entries: [user('u1', 'hi')], tools: [], commands: [{ name: 'model', source: 'builtin' }], models: [], dialogs: [], queue: [] });
    expect(s.entries).toHaveLength(1);
    expect(s.runStartedAt).toBeTypeOf('number');
    applyMessage(s, { t: 'msg_start', id: 'x', role: 'assistant' });
    applyMessage(s, { t: 'delta', id: 'x', kind: 'thinking', index: 0, text: 'hm' });
    applyMessage(s, { t: 'delta', id: 'x', kind: 'text', index: 1, text: 'Hel' });
    applyMessage(s, { t: 'delta', id: 'x', kind: 'text', index: 1, text: 'lo' });
    expect(s.streaming!.parts).toEqual([{ type: 'thinking', thinking: 'hm' }, { type: 'text', text: 'Hello' }]);
    applyMessage(s, { t: 'delta', id: 'other', kind: 'text', index: 1, text: '!' });
    expect((s.streaming!.parts[1] as { text: string }).text).toBe('Hello');
    applyMessage(s, { t: 'msg_end', id: 'x' });
    expect(s.streaming).toBeUndefined();
    applyMessage(s, { t: 'entry', entry: asst('a1', [{ type: 'text', text: 'Hello' }]) });
    applyMessage(s, { t: 'entry', entry: asst('a1', [{ type: 'text', text: 'Hello!' }]) });
    expect(s.entries).toHaveLength(2);
    applyMessage(s, { t: 'state', running: false });
    expect(s.runStartedAt).toBeUndefined();
  });
  it('pending bubbles clear on delivery or error; dialogs open/close', () => {
    const s = fresh();
    s.pending.push({ ref: 'r1', text: 'do it', mode: 'prompt', at: 0 }, { ref: 'r2', text: 'bad', mode: 'prompt', at: 0 });
    applyMessage(s, { t: 'entry', entry: user('u2', 'do it') });
    applyMessage(s, { t: 'error', message: 'nope', ref: 'r2' });
    expect(s.pending).toEqual([]);
    expect(s.error?.message).toBe('nope');
    applyMessage(s, { t: 'dialog', id: 4, kind: 'select', title: 'Pick', options: ['a'] });
    applyMessage(s, { t: 'dialog', id: 4, kind: 'select', title: 'Pick', options: ['a'] });
    expect(s.dialogs).toHaveLength(1);
    applyMessage(s, { t: 'dialog_end', id: 4, by: 'tui' });
    expect(s.dialogs).toHaveLength(0);
  });
  describe('pending bubbles never get stuck (UNI-143)', () => {
    const cmd = (s: ChatState) => s.commands.push({ name: 'unipi:kanboard-do', source: 'extension' }, { name: 'tpl', source: 'prompt' });
    it('an extension command (no args, nothing saved) clears on its ack', () => {
      const s = fresh();
      cmd(s);
      s.pending.push({ ref: 'r1', text: '/unipi:kanboard-do', mode: 'prompt', at: 0 });
      applyMessage(s, { t: 'ack', ref: 'r1' });
      expect(s.pending).toEqual([]);
      s.pending.push({ ref: 'r2', text: '/whatever', mode: 'prompt', at: 0 });
      applyMessage(s, { t: 'ack', ref: 'r2', as: 'command' });
      expect(s.pending).toEqual([]);
    });
    it('a rewritten prompt (template / skill) clears when its entry lands, by ref order not text', () => {
      const s = fresh();
      cmd(s);
      s.pending.push({ ref: 'r1', text: '/tpl fix it', mode: 'prompt', at: 0 });
      applyMessage(s, { t: 'ack', ref: 'r1' });
      expect(s.pending[0]!.acked).toBe(true);
      applyMessage(s, { t: 'input', text: '/tpl fix it', source: 'extension', mode: 'prompt', ref: 'r1' });
      expect(s.pending[0]!.accepted).toBe(true);
      applyMessage(s, { t: 'entry', entry: user('u9', 'Expanded template text: fix it') });
      expect(s.pending).toEqual([]);
    });
    it('a steer / follow-up leaves the bubbles when pi queues it', () => {
      const s = fresh();
      s.pending.push({ ref: 'r1', text: 'also this', mode: 'steer', at: 0 });
      applyMessage(s, { t: 'input', text: 'also this', source: 'extension', mode: 'steer', ref: 'r1' });
      expect(s.pending).toEqual([]);
    });
    it('a reconnect (hello) drops what pi already has; unacked sends stay for the sweep', () => {
      const s = fresh();
      s.pending.push({ ref: 'r1', text: 'hi', mode: 'prompt', at: 0, acked: true }, { ref: 'r2', text: 'later', mode: 'prompt', at: 0 }, { ref: 'r3', text: 'saved', mode: 'prompt', at: 0 });
      applyMessage(s, { t: 'hello', session: { id: 's', cwd: '/p' }, state: { running: false }, entries: [user('u1', 'saved')], tools: [], commands: [], models: [], dialogs: [], queue: [] });
      expect(s.pending.map((p) => p.ref)).toEqual(['r2']);
    });
    it('the sweep drops acked bubbles after a while and reports never-acked ones as lost', () => {
      const s = fresh();
      s.pending.push({ ref: 'a', text: 'x', mode: 'prompt', at: 0, acked: true }, { ref: 'b', text: 'y', mode: 'prompt', at: 0 }, { ref: 'c', text: 'z', mode: 'prompt', at: 25_000 });
      expect(sweepPending(s, 10_000)).toEqual([]);
      expect(s.pending).toHaveLength(3);
      const lost = sweepPending(s, PENDING_LOST_MS + 1);
      expect(lost.map((p) => p.ref)).toEqual(['b']);
      expect(s.pending.map((p) => p.ref)).toEqual(['c']);
    });
  });
  it('a paged-in older chunk never changes the keys of rows already shown (scroll anchor stays)', () => {
    const shown = [
      asst('a2', [{ type: 'toolCall', id: 'c2', name: 'read', arguments: { path: '/p/x.ts' } }], 'toolUse', 3000),
      result('r2', 'c2', 'code'),
      asst('a3', [{ type: 'text', text: 'Done.' }], 'stop', 7000),
    ];
    const before = buildRows(shown, undefined, {}, false).map((r) => r.key);
    const older = [user('u1', 'go'), asst('a1', [{ type: 'toolCall', id: 'c1', name: 'bash', arguments: { command: 'ls' } }], 'toolUse', 2000), result('r1', 'c1', 'x')];
    // Without the seam the older tool call would absorb the group (key p-a1-0).
    expect(buildRows([...older, ...shown], undefined, {}, false).map((r) => r.key)).not.toEqual(expect.arrayContaining(before));
    const after = buildRows([...older, ...shown], undefined, {}, false, new Set(['a2'])).map((r) => r.key);
    expect(after.slice(-before.length)).toEqual(before);
    // The store records the seam.
    const s = fresh();
    s.entries.push(...shown);
    s.hasOlder = true;
    applyMessage(s, { t: 'history', before: 'a2', entries: older, more: false });
    expect(s.seams).toEqual(['a2']);
    applyMessage(s, { t: 'hello', session: { id: 's', cwd: '/' }, state: { running: false }, entries: shown });
    expect(s.seams).toEqual([]);
  });
  it('history pages prepend only onto the list they were asked for', () => {
    const s = fresh();
    s.entries.push(user('u5', 'five'));
    s.hasOlder = true;
    s.loadingOlder = true;
    applyMessage(s, { t: 'history', before: 'u5', entries: [user('u3', 'three'), user('u4', 'four')], more: true });
    expect(s.entries.map((e) => e.id)).toEqual(['u3', 'u4', 'u5']);
    expect(s.hasOlder).toBe(true);
    expect(s.loadingOlder).toBe(false);
    applyMessage(s, { t: 'history', before: 'u5', entries: [user('u1', 'stale')], more: false });
    expect(s.entries[0]!.id).toBe('u3');
    applyMessage(s, { t: 'history', before: 'u3', entries: [user('u2', 'two')], more: false });
    expect(s.entries.map((e) => e.id)).toEqual(['u2', 'u3', 'u4', 'u5']);
    expect(s.hasOlder).toBe(false);
  });
  it('tool lifecycle and queue', () => {
    const s = fresh();
    applyMessage(s, { t: 'tool_start', callId: 'c', name: 'bash', args: { command: 'ls' } });
    applyMessage(s, { t: 'tool_update', callId: 'c', text: 'x' });
    applyMessage(s, { t: 'tool_end', callId: 'c', isError: true });
    expect(s.tools.c).toEqual({ name: 'bash', args: { command: 'ls' }, text: 'x', done: true, isError: true });
    applyMessage(s, { t: 'queue', items: [{ text: 'later', mode: 'followUp' }] });
    expect(s.queue).toHaveLength(1);
  });
});

describe('liveActivity (UNI-146): Thinking only while the model thinks', () => {
  const base = { dialogs: [] as unknown[], tools: {} as LiveTools, run: { running: true } };
  it('a running bash names itself, not Thinking', () => {
    expect(liveActivity({ ...base, tools: { c1: { name: 'bash', args: { command: 'npm test' } } } })).toEqual({ title: 'Running', detail: 'npm test' });
  });
  it('a tool call being written is Preparing <tool>', () => {
    expect(liveActivity({ ...base, streaming: { id: 's', parts: [{ type: 'thinking', thinking: 'hm' }, { type: 'toolCall', id: 'x', name: 'bash', arguments: '{"comm' } as never] } })).toEqual({ title: 'Preparing', detail: 'bash' });
  });
  it('thinking / writing / before the first token / between steps', () => {
    expect(liveActivity({ ...base, streaming: { id: 's', parts: [{ type: 'thinking', thinking: 'hm' } as never] } }).title).toBe('Thinking');
    expect(liveActivity({ ...base, streaming: { id: 's', parts: [{ type: 'text', text: 'Hi' } as never] } }).title).toBe('Writing');
    expect(liveActivity({ ...base, streaming: { id: 's', parts: [] } }).title).toBe('Waiting for the model');
    expect(liveActivity(base).title).toBe('Working');
    expect(liveActivity({ ...base, tools: { c1: { name: 'bash', args: {}, done: true } } }).title).toBe('Working');
  });
  it('an open dialog wins; several tools show a count; paths are made relative', () => {
    expect(liveActivity({ ...base, dialogs: [{}], tools: { c1: { name: 'bash', args: {} } } }).title).toBe('Waiting for you');
    expect(liveActivity({ ...base, tools: { a: { name: 'read', args: { path: '/p/src/a.ts' } }, b: { name: 'read', args: { path: '/p/src/b.ts' } } } }, '/p')).toEqual({ title: 'Reading', detail: 'src/b.ts +1' });
  });
});

describe('collapseCommandRows (UNI-210 / UNI-251)', () => {
  const echo = (id: string, text: string): Entry => ({ type: 'custom', id, customType: 'unipi-command-echo', data: { text } });
  const decision = (id: string, mode: string, source = 'explicit'): Entry => ({ type: 'custom', id, customType: 'long-horizon-decision', data: { mode, source } });
  const harnessUser = (id: string, text: string, source: string, title: string): Entry => ({
    type: 'message',
    id,
    message: { role: 'user', content: text, unipiHarness: { source, title } },
  });
  const KANBOARD = (req: string) =>
    `[kanboard] For this request you may use the kanboard skill on project unipi-faa347 (CLI: \`unipi-kanboard --actor agent --project unipi-faa347 …\`). Budget this session: 20 task slots. If the request is unclear, ask me instead of guessing.\n\nRequest: ${req}`;

  it('/unipi:graph Start: echo + decision + harness user → ONE user bubble; the harness only repeats the args → tag, no body', () => {
    const entries: Entry[] = [echo('e1', '/unipi:graph Start'), decision('d1', 'graph'), harnessUser('u1', 'Start', 'Long-horizon', 'Graph')];
    const rows = buildRows(entries, undefined, {}, false);
    expect(rows.map((r) => r.kind)).toEqual(['user']);
    expect(rows[0]).toMatchObject({ kind: 'user', key: 'u1', text: '/unipi:graph Start', command: { name: 'unipi:graph', tag: 'Long-horizon · graph', entryId: 'u1' } });
    expect((rows[0] as Extract<Row, { kind: 'user' }>).command!.body).toBeUndefined();
    expect((rows[0] as Extract<Row, { kind: 'user' }>).harness).toBeUndefined();
  });

  it('the real /unipi:kanboard-do shape (UNI-251): bubble = the typed text, chip "Kanboard · task budget", body = the preamble WITHOUT the repeated request', () => {
    const req = '- You can decide this.\n- Strip pi omniroute bridge away from app.\n\n- Can I restart UniPi this session rn?';
    const entries: Entry[] = [
      { type: 'custom', id: 'h0', customType: 'unipi-command-echo', data: { text: '/unipi-app-session' } },
      echo('e1', `/unipi:kanboard-do ${req}`),
      { type: 'custom', id: 'o1', customType: 'compactor-origin', data: { key: 'x' } },
      harnessUser('u1', KANBOARD(req), 'Kanboard', 'Task budget'),
    ];
    const rows = buildRows(entries, undefined, {}, false);
    expect(rows.map((r) => r.kind)).toEqual(['user']);
    const u = rows[0] as Extract<Row, { kind: 'user' }>;
    expect(u.text).toBe(`/unipi:kanboard-do ${req}`);
    expect(u.command).toMatchObject({ name: 'unipi:kanboard-do', tag: 'Kanboard · task budget', entryId: 'u1' });
    expect(u.command!.body).toMatch(/^\[kanboard\] For this request/);
    expect(u.command!.body).not.toContain('Request:');
    expect(u.command!.body).not.toContain('restart UniPi');
  });

  it('the hidden /unipi-app-session never shows — echo entry or a user message', () => {
    const entries: Entry[] = [
      echo('h1', '/unipi-app-session'),
      { type: 'message', id: 'h2', message: { role: 'user', content: '/unipi-app-session' } },
      { type: 'message', id: 'u1', message: { role: 'user', content: 'hi' } },
    ];
    const rows = buildRows(entries, undefined, {}, false);
    expect(rows.map((r) => r.kind)).toEqual(['user']);
    expect(rows[0]).toMatchObject({ key: 'u1', text: 'hi' });
    expect(isHiddenCommandText('/unipi-app-session')).toBe(true);
    expect(isHiddenCommandText('  /unipi-app-session x')).toBe(true);
    expect(isHiddenCommandText('/unipi-app-sessions')).toBe(false);
    expect(isHiddenCommandText('/unipi:kanboard-do x')).toBe(false);
  });

  it('a long-horizon-decision with mode "none" does not label; the echo is still the user bubble', () => {
    const rows = buildRows([echo('e1', '/unipi:regular'), decision('d1', 'none')], undefined, {}, false);
    expect(rows.map((r) => r.kind)).toEqual(['user']);
    expect((rows[0] as Extract<Row, { kind: 'user' }>).command).toEqual({ name: 'unipi:regular', tag: undefined, body: undefined, entryId: undefined });
  });

  it('a bare echo (no harness turn) is a user bubble with no chip, the next card stays', () => {
    const rows = buildRows([echo('e1', '/unipi:btw what codename'), { type: 'custom', id: 'k', customType: 'unipi:kanboard-notice', data: {} }], undefined, {}, false);
    expect(rows.map((r) => r.kind)).toEqual(['user', 'card']);
    expect(rows[0]).toMatchObject({ text: '/unipi:btw what codename', command: { name: 'unipi:btw' } });
  });

  it('a tool call between the echo and the harness turn breaks the fold', () => {
    const entries: Entry[] = [
      echo('e1', '/unipi:graph Start'),
      { type: 'message', id: 'a1', message: { role: 'assistant', content: [{ type: 'toolCall', id: 'c1', name: 'bash', arguments: { command: 'ls' } }], stopReason: 'toolUse' } } as Entry,
      harnessUser('u1', 'Start', 'Long-horizon', 'Graph'),
    ];
    const rows = buildRows(entries, undefined, {}, false);
    const users = rows.filter((r) => r.kind === 'user') as Array<Extract<Row, { kind: 'user' }>>;
    expect(users[0]!.command?.tag).toBeUndefined();
    expect(users[1]!.harness).toMatchObject({ source: 'Long-horizon' });
  });

  it('collapseCommandRows is idempotent (buildRows already applies it)', () => {
    const entries: Entry[] = [echo('e1', '/unipi:graph Start'), decision('d1', 'graph'), harnessUser('u1', 'Start', 'Long-horizon', 'Graph')];
    const once = buildRows(entries, undefined, {}, false);
    expect(collapseCommandRows(once as Row[])).toEqual(once);
  });

  it('injectedBody strips only the repeated request', () => {
    expect(injectedBody('Start', 'Start')).toBe('');
    expect(injectedBody('pre\n\nRequest: do it', 'do it')).toBe('pre');
    expect(injectedBody('Implement this plan', 'other')).toBe('Implement this plan');
    expect(injectedBody('anything', '')).toBe('anything');
    expect(injectedBody('Analyze the following text.\n\nText to analyze:\nmy notes', 'my notes')).toBe('Analyze the following text.');
  });
});
