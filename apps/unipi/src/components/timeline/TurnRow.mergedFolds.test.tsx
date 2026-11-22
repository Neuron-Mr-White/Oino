// @vitest-environment jsdom
/** OIN-96: consecutive "Worked" folds with no visible prose between them render as ONE fold row. */
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import { afterEach, beforeEach, expect, it } from 'vitest';
import { buildTurns } from '../../lib/session/turns';
import type { SessionEntry } from '../../lib/session/types';
import { clearExpansion } from '../../lib/timeline/expansion';
import { TurnRow } from './TurnRow';
import type { TimelineCtx } from './context';

const ctx = {
  session: { state: { byId: {}, entries: [] }, key: 'k' }, hostId: 'h', sessionId: () => 'merged-folds',
  cwd: () => '/', refs: { hostId: 'h', cwd: () => '/' }, commands: () => [], columnWidth: () => 720, readOnly: true,
} as unknown as TimelineCtx;

beforeEach(() => { localStorage.clear(); clearExpansion(); });
afterEach(() => { cleanup(); clearExpansion(); });

const text = (t: string) => ({ type: 'text', text: t });

/** user → `wakes` × (one bash call + result + empty `stop` message) → optional final text. */
function turnOf(wakes: number, final?: string) {
  const entries: SessionEntry[] = [{ type: 'message', id: 'u', parentId: null, message: { role: 'user', content: [text('Run the swarm')] } } as unknown as SessionEntry];
  let parent = 'u';
  const push = (e: unknown) => { entries.push(e as SessionEntry); parent = (e as { id: string }).id; };
  for (let i = 0; i < wakes; i++) {
    push({ type: 'message', id: `a${i}`, parentId: parent, message: { role: 'assistant', content: [{ type: 'toolCall', id: `t${i}`, name: 'bash', arguments: { command: `echo ${i}` } }], stopReason: 'toolUse' } });
    push({ type: 'message', id: `r${i}`, parentId: parent, message: { role: 'toolResult', toolCallId: `t${i}`, toolName: 'bash', content: [text('ok')], isError: false } });
    push({ type: 'message', id: `s${i}`, parentId: parent, message: { role: 'assistant', content: [], stopReason: 'stop' } });
  }
  if (final) push({ type: 'message', id: 'fin', parentId: parent, message: { role: 'assistant', content: [text(final)], stopReason: 'stop' } });
  return buildTurns({ entries, leafId: parent, entriesRev: 1, liveTools: {}, run: { running: false } })[0]!;
}

it('seven wakes render one fold row, its summary aggregated, the final text visible below', () => {
  const r = render(() => <TurnRow turn={turnOf(7, 'Final answer.')} ctx={ctx} />);
  expect(r.container.querySelectorAll('.tl-fold')).toHaveLength(1);
  expect(r.container.querySelector('.tl-fold .proc-detail')?.textContent).toBe('ran 7 commands');
  expect(r.container.querySelector('.tl-fold .proc-title')?.textContent).toMatch(/^Worked/);
  expect(r.container.querySelector('.tl-prose')?.textContent).toContain('Final answer.');
  expect(r.container.querySelector('.tl-fold .tl-prose')).toBeNull();
});

it('expanding the merged fold shows every wake\'s work', () => {
  const r = render(() => <TurnRow turn={turnOf(3)} ctx={ctx} />);
  expect(r.container.querySelectorAll('.tl-fold')).toHaveLength(1);
  fireEvent.click(r.container.querySelector('.tl-fold .proc-head')!);
  const body = r.container.querySelector('.tl-fold-body')!;
  expect(body).toBeTruthy();
  // the three wakes' bash calls sit in one command group inside the single body
  expect(body.textContent).toContain('Ran 3 commands');
});

/** Real swarm shape: the lead's answer is followed by the work it started in the same message, then work-only wakes. */
function swarmTurn(wakes: number) {
  const entries: SessionEntry[] = [{ type: 'message', id: 'u', parentId: null, message: { role: 'user', content: [text('Run the swarm')] } } as unknown as SessionEntry];
  let parent = 'u';
  const push = (e: unknown) => { entries.push(e as SessionEntry); parent = (e as { id: string }).id; };
  push({ type: 'message', id: 'a0', parentId: parent, message: { role: 'assistant', content: [text('Dispatched the workers.'), { type: 'toolCall', id: 't0', name: 'bash', arguments: { command: 'echo w0' } }], stopReason: 'toolUse' } });
  push({ type: 'message', id: 'r0', parentId: parent, message: { role: 'toolResult', toolCallId: 't0', toolName: 'bash', content: [text('ok')], isError: false } });
  push({ type: 'message', id: 's0', parentId: parent, message: { role: 'assistant', content: [], stopReason: 'stop' } });
  for (let i = 1; i <= wakes; i++) {
    push({ type: 'message', id: `a${i}`, parentId: parent, message: { role: 'assistant', content: [{ type: 'toolCall', id: `t${i}`, name: 'bash', arguments: { command: `echo w${i}` } }], stopReason: 'toolUse' } });
    push({ type: 'message', id: `r${i}`, parentId: parent, message: { role: 'toolResult', toolCallId: `t${i}`, toolName: 'bash', content: [text('ok')], isError: false } });
    push({ type: 'message', id: `s${i}`, parentId: parent, message: { role: 'assistant', content: [], stopReason: 'stop' } });
  }
  return buildTurns({ entries, leafId: parent, entriesRev: 1, liveTools: {}, run: { running: false } })[0]!;
}

it('an answer followed by seven supervisor wakes renders ONE "Worked" row under it, expanding to every step', () => {
  const r = render(() => <TurnRow turn={swarmTurn(7)} ctx={ctx} />);
  expect(r.container.querySelectorAll('.tl-fold')).toHaveLength(1);
  const prose = r.container.querySelector('.tl-prose')!;
  const fold = r.container.querySelector('.tl-fold')!;
  expect(prose.textContent).toContain('Dispatched the workers.');
  // the fold row sits BELOW the answer
  expect(prose.compareDocumentPosition(fold) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(fold.querySelector('.proc-detail')?.textContent).toBe('ran 8 commands');
  fireEvent.click(fold.querySelector('.proc-head')!);
  expect(r.container.querySelector('.tl-fold-body')?.textContent).toContain('Ran 8 commands');
});
