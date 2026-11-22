// @vitest-environment jsdom
/** "Send as note": an `oino.note` custom entry is its own right-aligned row — never a user message, never inside a fold. */
import { cleanup, render } from '@solidjs/testing-library';
import { afterEach, expect, it } from 'vitest';
import { buildTurns } from '../../lib/session/turns';
import type { SessionEntry } from '../../lib/session/types';
import type { TimelineCtx } from './context';
import { TurnRow } from './TurnRow';

afterEach(() => cleanup());

const at = new Date(0).toISOString();
const msg = (id: string, parentId: string | null, role: 'user' | 'assistant', text: string) =>
  ({ type: 'message', id, parentId, timestamp: at, message: { role, content: [{ type: 'text', text }], timestamp: 1 } }) as unknown as SessionEntry;
const tool = (id: string, parentId: string) =>
  ({ type: 'message', id, parentId, timestamp: at, message: { role: 'assistant', stopReason: 'toolUse', content: [{ type: 'toolCall', id: `c-${id}`, name: 'read', arguments: { path: 'a.ts' } }], timestamp: 1 } }) as unknown as SessionEntry;
const result = (id: string, parentId: string, call: string) =>
  ({ type: 'message', id, parentId, timestamp: at, message: { role: 'toolResult', toolCallId: `c-${call}`, toolName: 'read', content: [{ type: 'text', text: 'ok' }], isError: false, timestamp: 1 } }) as unknown as SessionEntry;
const note = (id: string, parentId: string, text: string) =>
  ({ type: 'custom', id, parentId, timestamp: at, customType: 'oino.note', data: { text } }) as unknown as SessionEntry;

const turnsOf = (entries: SessionEntry[], running = false) =>
  buildTurns({ entries, leafId: entries.at(-1)!.id, entriesRev: 0, liveTools: {}, run: { running } } as never);

const ctxOf = (turns: ReturnType<typeof turnsOf>) =>
  ({
    session: { state: { byId: {}, entries: [] }, key: 'k', turns: () => turns },
    hostId: 'h',
    sessionId: () => 's',
    cwd: () => '/',
    refs: { hostId: 'h', cwd: () => '/' },
    commands: () => new Set<string>(),
    columnWidth: () => 720,
  }) as unknown as TimelineCtx;

it('renders the note as a bubble with the caption "Note · only you", markdown included', () => {
  const entries = [msg('u0', null, 'user', 'Do it'), msg('a0', 'u0', 'assistant', 'Done'), note('n0', 'a0', 'remember **this** for later')];
  const turns = turnsOf(entries);
  // a note is not a user message: still one turn
  expect(turns.filter((t) => t.kind === 'turn')).toHaveLength(1);
  const r = render(() => <TurnRow turn={turns[0]!} ctx={ctxOf(turns)} />);
  const row = r.getByTestId('tl-note');
  expect(row.querySelector('[data-testid="tl-note-caption"]')?.textContent).toContain('Note · only you');
  expect(row.querySelector('strong')?.textContent).toBe('this');
  // the user bubble count stays one: the note is not a second `tl-user` message row of the owner
  expect(r.container.querySelectorAll('.tl-user:not(.tl-note)')).toHaveLength(1);
});

it('a note in the middle of a run is its own row, in order, outside the Worked folds', () => {
  const entries = [
    msg('u0', null, 'user', 'Go'),
    tool('t1', 'u0'),
    result('r1', 't1', 't1'),
    note('n1', 'r1', 'mid-run thought'),
    tool('t2', 'n1'),
    result('r2', 't2', 't2'),
    msg('a9', 'r2', 'assistant', 'All done'),
  ];
  const turns = turnsOf(entries);
  expect(turns).toHaveLength(1);
  const segments = turns[0]!.segments;
  const withNote = segments.find((s) => s.before?.some((n) => n.kind === 'card' && n.entry.id === 'n1'));
  expect(withNote).toBeTruthy();
  // no fold holds the note
  for (const s of segments) expect(s.nodes.some((n) => n.kind === 'card' && n.entry.id === 'n1')).toBe(false);
  const r = render(() => <TurnRow turn={turns[0]!} ctx={ctxOf(turns)} />);
  expect(r.getAllByTestId('tl-note')).toHaveLength(1);
  expect(r.getByTestId('tl-note').textContent).toContain('mid-run thought');
});

it('a note neither starts a turn nor counts as a steer', () => {
  const entries = [msg('u0', null, 'user', 'Go'), note('n1', 'u0', 'early note'), msg('a0', 'n1', 'assistant', 'Hi')];
  const turns = turnsOf(entries);
  expect(turns).toHaveLength(1);
  expect(turns[0]!.user?.id).toBe('u0');
  expect(turns[0]!.segments.every((s) => !s.steer)).toBe(true);
});
