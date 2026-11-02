// @vitest-environment jsdom
/**
 * ui-stream: the live tail turn is rebuilt (fresh objects) on every stream
 * flush; its rows must stay mounted — the streaming prose keeps its DOM node
 * and only its text grows.
 */
import { cleanup, render } from '@solidjs/testing-library';
import { createSignal } from 'solid-js';
import { afterEach, expect, it } from 'vitest';
import { setStreamSmoothing } from '../../lib/timeline/smoothStream';
import { buildTurns, type TurnsInput } from '../../lib/session/turns';
import type { SessionEntry } from '../../lib/session/types';
import type { TimelineCtx } from './context';
import { TurnRow } from './TurnRow';

afterEach(() => {
  cleanup();
  setStreamSmoothing(undefined);
});

const user = {
  type: 'message',
  id: 'u0',
  parentId: null,
  timestamp: new Date(0).toISOString(),
  message: { role: 'user', content: [{ type: 'text', text: 'Q' }], timestamp: 1 },
} as unknown as SessionEntry;

const input = (parts: unknown[]): TurnsInput => ({
  entries: [user],
  leafId: 'u0',
  entriesRev: 0,
  streaming: { message: { role: 'assistant', content: parts } as never, startedAt: 1 },
  liveTools: {},
  run: { running: true, startedAt: 1 },
});

const ctx = {
  session: { state: { byId: {}, entries: [] }, key: 'k' },
  hostId: 'h',
  sessionId: () => 's',
  cwd: () => '/',
  refs: { hostId: 'h', cwd: () => '/' },
  commands: () => [],
  columnWidth: () => 720,
} as unknown as TimelineCtx;

it('keeps the streaming prose and the work rows mounted across stream flushes', () => {
  setStreamSmoothing(false);
  const tool = { type: 'toolCall', id: 't1', name: 'read', arguments: { path: 'a.ts' } };
  const [turn, setTurn] = createSignal(buildTurns(input([tool, { type: 'text', text: 'Hello' }])).at(-1)!);
  const r = render(() => <TurnRow turn={turn()} ctx={ctx} />);
  const prose = r.container.querySelector('.tl-prose');
  const segment = r.container.querySelector('.tl-segment');
  expect(prose?.textContent).toContain('Hello');
  setTurn(buildTurns(input([tool, { type: 'text', text: 'Hello world' }])).at(-1)!);
  expect(r.container.querySelector('.tl-prose')).toBe(prose);
  expect(r.container.querySelector('.tl-segment')).toBe(segment);
  expect(prose?.textContent).toContain('Hello world');
});
