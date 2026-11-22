// @vitest-environment jsdom
import { cleanup, render } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { buildTurns, type WorkNode } from '../../lib/session/turns';
import type { SessionEntry } from '../../lib/session/types';
import { clearExpansion } from '../../lib/timeline/expansion';
import { afterRuns, TurnRow } from './TurnRow';
import type { TimelineCtx } from './context';

const tool = (key: string): WorkNode => ({ kind: 'tool', key, tool: { id: key, name: 'bash', args: { command: 'echo ok' }, state: 'ok' } });
const card: WorkNode = { kind: 'card', key: 'card', entry: {} as SessionEntry };
const ctx = {
  session: { state: { byId: {}, entries: [] }, key: 'k' }, hostId: 'h', sessionId: () => 'after-runs',
  cwd: () => '/', refs: { hostId: 'h', cwd: () => '/' }, commands: () => [], columnWidth: () => 720, readOnly: true,
} as unknown as TimelineCtx;

beforeEach(() => { localStorage.clear(); clearExpansion(); });
afterEach(() => { cleanup(); clearExpansion(); });

describe('afterRuns', () => {
  it('merges consecutive tools and derives a detail summary', () => {
    const result = afterRuns([tool('1'), tool('2'), tool('3')]);
    expect(result).toHaveLength(1);
    expect(result[0]).toMatchObject({ kind: 'work', key: '1', detail: 'ran 3 commands' });
  });

  it('keeps cards between two pieces of work inside the one fold', () => {
    const result = afterRuns([tool('1'), card, tool('2')]);
    expect(result.map((r) => r.kind)).toEqual(['work']);
    expect(result[0]).toMatchObject({ nodes: [{ key: '1' }, { key: 'card' }, { key: '2' }], detail: 'ran 2 commands' });
  });

  it('keeps leading and trailing cards plain around the fold; compaction splits it', () => {
    expect(afterRuns([card, tool('1'), card]).map((r) => r.kind)).toEqual(['plain', 'work', 'plain']);
    const compaction: WorkNode = { kind: 'compaction', key: 'cmp', entry: {} as SessionEntry };
    expect(afterRuns([tool('1'), card, compaction, tool('2')]).map((r) => r.kind)).toEqual(['work', 'plain', 'plain', 'work']);
  });

  it('does not split work around a blank state card', () => {
    const blank: WorkNode = { kind: 'card', key: 'blank', entry: { type: 'custom', customType: 'oino.image-policy', data: {} } as SessionEntry };
    const result = afterRuns([tool('1'), blank, tool('2')]);
    expect(result).toHaveLength(1);
    expect(result[0]).toMatchObject({ kind: 'work', key: '1', detail: 'ran 2 commands' });
  });

  it('returns no runs for empty input', () => {
    expect(afterRuns([])).toEqual([]);
  });
});

it('renders one fold for three tools trailing an answer', () => {
  const text = (text: string) => ({ type: 'text', text });
  const parts = [text('Answer.'), ...[1, 2, 3].map((i) => ({ type: 'toolCall', id: `tool${i}`, name: 'bash', arguments: { command: `echo ${i}` } }))];
  const entries = [
    { type: 'message', id: 'u', parentId: null, message: { role: 'user', content: [text('Question')] } },
    { type: 'message', id: 'a', parentId: 'u', message: { role: 'assistant', content: parts, stopReason: 'stop' } },
  ] as unknown as SessionEntry[];
  const turn = buildTurns({ entries, leafId: 'a', entriesRev: 1, liveTools: {}, run: { running: false } })[0]!;
  const rendered = render(() => <TurnRow turn={turn} ctx={ctx} />);
  expect(rendered.container.querySelector('.tl-prose')?.textContent).toContain('Answer.');
  expect(rendered.container.querySelectorAll('.tl-segment > .tl-fold')).toHaveLength(1);
  expect(rendered.container.querySelector('.tl-fold .proc-detail')?.textContent).toBe('ran 3 commands');
});

it('keeps one trailing fold across ten tool rounds interleaved with core state entries', () => {
  const text = (text: string) => ({ type: 'text', text });
  const entries: SessionEntry[] = [
    { type: 'message', id: 'u', parentId: null, message: { role: 'user', content: [text('Question')] } },
    { type: 'message', id: 'a0', parentId: 'u', message: { role: 'assistant', content: [text('Intermediate answer.')], stopReason: 'stop' } },
  ] as unknown as SessionEntry[];
  let parent = 'a0';
  for (let i = 0; i < 10; i++) {
    const a = `a${i + 1}`;
    const r = `r${i + 1}`;
    entries.push({ type: 'message', id: a, parentId: parent, message: { role: 'assistant', content: [{ type: 'thinking', thinking: `round ${i}` }, { type: 'toolCall', id: `tool${i}`, name: 'bash', arguments: { command: `echo ${i}` } }], stopReason: 'toolUse' } } as unknown as SessionEntry);
    entries.push({ type: 'message', id: r, parentId: a, message: { role: 'toolResult', toolCallId: `tool${i}`, toolName: 'bash', content: [text('ok')], isError: false } } as unknown as SessionEntry);
    entries.push({ type: 'custom', id: `p${i}`, parentId: r, customType: 'oino.image-policy', data: {} } as unknown as SessionEntry);
    entries.push({ type: 'custom', id: `s${i}`, parentId: `p${i}`, customType: 'oino.sent-frontier', data: {} } as unknown as SessionEntry);
    parent = `s${i}`;
  }
  const turn = buildTurns({ entries, leafId: parent, entriesRev: 1, liveTools: {}, run: { running: false } })[0]!;
  const rendered = render(() => <TurnRow turn={turn} ctx={ctx} />);
  expect(rendered.container.querySelector('.tl-prose')?.textContent).toContain('Intermediate answer.');
  expect(rendered.container.querySelectorAll('.tl-fold').length).toBeLessThanOrEqual(2);
});
