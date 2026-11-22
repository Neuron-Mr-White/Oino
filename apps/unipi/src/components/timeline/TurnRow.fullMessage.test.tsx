// @vitest-environment jsdom
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import { createMemo, For } from 'solid-js';
import { createStore, produce, unwrap } from 'solid-js/store';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createFullMessages, TRUNCATION_NOTE } from '../../lib/session/fullMessages';
import { initialState } from '../../lib/session/reducer';
import { buildTurns } from '../../lib/session/turns';
import type { SessionEntry, SessionState } from '../../lib/session/types';
import { TurnRow } from './TurnRow';
import { setDesktopOverride } from '../../lib/layout';
import { clearExpansion } from '../../lib/timeline/expansion';
import type { TimelineCtx } from './context';

afterEach(() => { cleanup(); setDesktopOverride(undefined); clearExpansion(); });

const HEAD = 'The answer starts here and runs up to `inline';
const TAIL = ' code` and then finishes with the tail sentence.';
const user = (n: number, text = `Question ${n}`): SessionEntry => ({ type: 'message', id: `u${n}`, parentId: n ? `a${n - 1}` : null, message: { role: 'user', content: [{ type: 'text', text }] } });
const cutAnswer = (n: number): SessionEntry => ({
  type: 'message', id: `a${n}`, parentId: `u${n}`,
  message: { role: 'assistant', stopReason: 'stop', truncated: true, content: [{ type: 'text', text: `${HEAD}${TRUNCATION_NOTE}` }] },
});
const fullAnswer = (n: number): SessionEntry => ({
  type: 'message', id: `a${n}`, parentId: `u${n}`,
  message: { role: 'assistant', stopReason: 'stop', content: [{ type: 'text', text: `${HEAD}${TAIL}` }] },
});

function mount(entries: SessionEntry[], getEntries: (ids: readonly string[]) => Promise<SessionEntry[]>) {
  const [state, setState] = createStore<SessionState>({ ...initialState('h', 's'), entries, byId: Object.fromEntries(entries.map((e, i) => [e.id, i])), leafId: entries.at(-1)!.id });
  const fullMessages = createFullMessages({
    entry: (id) => { const at = state.byId[id]; return at === undefined ? undefined : state.entries[at]; },
    getEntries,
    apply: (full) => setState(produce((s) => {
      for (const e of full) { const at = s.byId[e.id]; if (at !== undefined) s.entries[at] = e; }
      s.entriesRev += 1;
    })),
  });
  const ctx = {
    session: { state, key: 'k', fullMessages },
    hostId: 'h', sessionId: () => 's', cwd: () => '/', refs: { hostId: 'h', cwd: () => '/' }, commands: () => [], columnWidth: () => 720, readOnly: true,
  } as unknown as TimelineCtx;
  const turns = createMemo(() => buildTurns({ entries: unwrap(state).entries, leafId: state.leafId, entriesRev: state.entriesRev, liveTools: {}, run: { running: false } }));
  // Keyed by turn id like the Timeline's rows: a rebuilt turn (full entry swapped in) updates the mounted row.
  return render(() => <For each={turns().map((t) => t.turnId)}>{(id) => <TurnRow turn={turns().find((t) => t.turnId === id)!} ctx={ctx} />}</For>);
}

describe('hotfix 1.0.9: cut messages fetch their full entry', () => {
  it('a cut answer asks get_entries once, never shows the host note, then renders the full text', async () => {
    let resolve!: (e: SessionEntry[]) => void;
    const getEntries = vi.fn(() => new Promise<SessionEntry[]>((r) => (resolve = r)));
    const r = mount([user(0), cutAnswer(0)], getEntries);
    await vi.waitFor(() => expect(getEntries).toHaveBeenCalledTimes(1));
    expect(getEntries).toHaveBeenCalledWith(['a0']);
    const prose = () => r.container.querySelector('.tl-prose')!.textContent ?? '';
    expect(prose()).toContain('runs up to');
    expect(r.container.textContent).not.toContain('truncated by the host');
    expect(r.getByTestId('full-message-loading').textContent).toContain('Loading the rest…');
    resolve([fullAnswer(0)]);
    await vi.waitFor(() => expect(prose()).toContain('and then finishes with the tail sentence.'));
    expect(r.container.textContent).not.toContain('truncated by the host');
    expect(r.queryByTestId('full-message-loading')).toBeNull();
    expect(getEntries).toHaveBeenCalledTimes(1);
  });

  it('a failed fetch shows "Show full message", which retries', async () => {
    const getEntries = vi.fn<(ids: readonly string[]) => Promise<SessionEntry[]>>().mockRejectedValueOnce(new Error('offline'));
    const r = mount([user(0), cutAnswer(0)], getEntries);
    const retry = await r.findByTestId('full-message-retry');
    expect(retry.textContent).toBe('Show full message');
    expect(r.container.textContent).not.toContain('truncated by the host');
    getEntries.mockResolvedValueOnce([fullAnswer(0)]);
    fireEvent.click(retry);
    await vi.waitFor(() => expect(r.container.querySelector('.tl-prose')!.textContent).toContain('tail sentence'));
    expect(getEntries).toHaveBeenCalledTimes(2);
    expect(r.queryByTestId('full-message-retry')).toBeNull();
  });

  it('three cut rows rendered together make one request', async () => {
    const getEntries = vi.fn(async (ids: readonly string[]) => ids.map((id) => fullAnswer(Number(id.slice(1)))));
    const r = mount([user(0), cutAnswer(0), user(1), cutAnswer(1), user(2), cutAnswer(2)], getEntries);
    await vi.waitFor(() => expect(r.container.querySelectorAll('.tl-prose').length).toBe(3));
    await vi.waitFor(() => expect(r.container.textContent).toContain('tail sentence'));
    expect(getEntries).toHaveBeenCalledTimes(1);
    expect(getEntries).toHaveBeenCalledWith(['a0', 'a1', 'a2']);
    expect(Array.from(r.container.querySelectorAll('.tl-prose')).every((el) => el.textContent?.includes('tail sentence'))).toBe(true);
  });

  it('a cut user message is fetched too and its bubble never shows the note', async () => {
    const cutUser: SessionEntry = { type: 'message', id: 'u0', parentId: null, message: { role: 'user', truncated: true, content: [{ type: 'text', text: `A long pasted question${TRUNCATION_NOTE}` }] } };
    const full: SessionEntry = { type: 'message', id: 'u0', parentId: null, message: { role: 'user', content: [{ type: 'text', text: 'A long pasted question, whole.' }] } };
    const getEntries = vi.fn(async () => [full]);
    const r = mount([cutUser, fullAnswer(0)], getEntries);
    expect(r.container.textContent).not.toContain('truncated by the host');
    await vi.waitFor(() => expect(r.container.textContent).toContain('A long pasted question, whole.'));
    expect(getEntries).toHaveBeenCalledWith(['u0']);
  });
});

it('renders the end of a >1 MB assistant toolCall after full-entry hydration', async () => {
  setDesktopOverride(true);
  const cut: SessionEntry = { type: 'message', id: 'args', parentId: 'u0', message: { role: 'assistant', truncated: true, content: [
    { type: 'toolCall', id: 'call', name: 'bash', arguments: { command: 'head' + TRUNCATION_NOTE } },
  ] } };
  const full = structuredClone(cut);
  delete full.message!.truncated;
  (full.message!.content as unknown as Array<{ arguments: { command: string } }>)[0]!.arguments.command = 'x'.repeat(1100000) + ' ARGS-END';
  const getEntries = vi.fn(async () => [full]);
  const r = mount([user(0), cut], getEntries);
  await vi.waitFor(() => expect(getEntries).toHaveBeenCalledWith(['args']));
  await new Promise((resolve) => setTimeout(resolve, 0)); // let the full-entry replacement settle before opening its new fold
  const fold = r.container.querySelector('[data-testid="tl-fold"] button') as HTMLElement | null;
  if (fold) fireEvent.click(fold);
  const row = await r.findByTestId('tl-tool-row');
  fireEvent.click(row);
  await vi.waitFor(() => expect(r.container.querySelector('.tool-term-cmd')?.textContent).toContain('ARGS-END'));
  expect(r.container.textContent).not.toContain('truncated by the host');
  expect(r.queryByTestId('tool-full-arguments')).toBeNull();
});

describe('w7-ui G3-2: an open tool row follows its call entry once it is hydrated', () => {
  // `stopReason: 'stop'` on a call that has a result: its text is the segment's answer and the work trails it (the "after" run).
  // Open/closed state is remembered per tool key (`expansion.ts`): every test uses its own call id.
  const callEntry = (command: string, clipped: boolean, id = 'call1'): SessionEntry => ({
    type: 'message', id, parentId: 'u0',
    message: { role: 'assistant', stopReason: 'stop', ...(clipped ? { truncated: true } : {}), content: [
      { type: 'text', text: 'Running the big script.' },
      { type: 'toolCall', id: 'c1', name: 'bash', arguments: { command } },
    ] },
  });
  const result: SessionEntry = { type: 'message', id: 'res1', parentId: 'call1', message: { role: 'toolResult', toolCallId: 'c1', toolName: 'bash', isError: false, content: [{ type: 'text', text: 'done' }] } };
  const after: SessionEntry = { type: 'message', id: 'a2', parentId: 'res1', message: { role: 'assistant', stopReason: 'stop', content: [{ type: 'text', text: 'Finished.' }] } };

  it('shows "Show full command" for a clipped command, and the whole command in the same open row once hydrated', async () => {
    setDesktopOverride(true);
    const head = `echo start\n${'echo "arg line"\n'.repeat(500)}`;
    const FULL = `${head}${'echo "more"\n'.repeat(60_000)}ARGS-END`;
    let resolve!: (e: SessionEntry[]) => void;
    const getEntries = vi.fn(() => new Promise<SessionEntry[]>((r) => (resolve = r)));
    const r = mount([user(0), callEntry(head + TRUNCATION_NOTE, true), result, after], getEntries);
    await vi.waitFor(() => expect(getEntries).toHaveBeenCalledWith(['call1']));
    const fold = r.container.querySelector('[data-testid="tl-fold"] button') as HTMLElement | null;
    if (fold) fireEvent.click(fold);
    fireEvent.click(await r.findByTestId('tl-tool-row'));
    await vi.waitFor(() => expect(r.container.querySelector('.tool-term-cmd')).toBeTruthy());
    // opened while clipped: the cut is announced, never silent
    const button = r.getByTestId('tool-full-arguments');
    expect(button.textContent).toContain('Loading full command');
    expect(r.container.querySelector('.tool-term-cmd')!.textContent).not.toContain('ARGS-END');
    expect(r.container.textContent).not.toContain('truncated by the host');
    resolve([callEntry(FULL, false)]);
    // the SAME open row now carries the whole command
    await vi.waitFor(() => expect(r.container.querySelector('.tool-term-cmd')?.textContent).toContain('ARGS-END'));
    expect(r.queryByTestId('tool-full-arguments')).toBeNull();
  });

  it('a failed fetch leaves a retryable "Show full command"', async () => {
    setDesktopOverride(true);
    const head = 'echo start\n'.repeat(50);
    const getEntries = vi.fn<(ids: readonly string[]) => Promise<SessionEntry[]>>().mockRejectedValueOnce(new Error('offline'));
    const r = mount([user(0), callEntry(head + TRUNCATION_NOTE, true, 'call2'), { ...result, parentId: 'call2' }, { ...after, parentId: 'res1' }], getEntries);
    const fold = r.container.querySelector('[data-testid="tl-fold"] button') as HTMLElement | null;
    if (fold) fireEvent.click(fold);
    fireEvent.click(await r.findByTestId('tl-tool-row'));
    await vi.waitFor(() => expect(r.getByTestId('tool-full-arguments').textContent).toContain('Show full command'));
    getEntries.mockResolvedValueOnce([callEntry(`${head}ARGS-END`, false, 'call2')]);
    fireEvent.click(r.getByTestId('tool-full-arguments'));
    await vi.waitFor(() => expect(r.container.querySelector('.tool-term-cmd')?.textContent).toContain('ARGS-END'));
  });
});
