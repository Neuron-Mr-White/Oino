// @vitest-environment jsdom
/** Perf T13: the `oino.turn-interrupted` marker is a one-line "Interrupted — Continue" row; Continue sends `/continue`. */
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import { createStore } from 'solid-js/store';
import { afterEach, expect, it, vi } from 'vitest';
import { CONTINUE_ACKED_FALLBACK_MS } from './cards/InterruptedRow';
import { buildTurns } from '../../lib/session/turns';
import type { SessionEntry } from '../../lib/session/types';
import type { TimelineCtx } from './context';
import { TurnRow } from './TurnRow';

afterEach(() => cleanup());

const at = new Date(0).toISOString();
const msg = (id: string, parentId: string | null, role: 'user' | 'assistant', text: string) =>
  ({ type: 'message', id, parentId, timestamp: at, message: { role, content: [{ type: 'text', text }], timestamp: 1 } }) as unknown as SessionEntry;
const tool = (id: string, parentId: string) =>
  ({ type: 'message', id, parentId, timestamp: at, message: { role: 'assistant', stopReason: 'toolUse', content: [{ type: 'toolCall', id: `c-${id}`, name: 'bash', arguments: { command: 'make' } }], timestamp: 1 } }) as unknown as SessionEntry;
const result = (id: string, parentId: string, call: string) =>
  ({ type: 'message', id, parentId, timestamp: at, message: { role: 'toolResult', toolCallId: `c-${call}`, toolName: 'bash', content: [{ type: 'text', text: 'interrupted' }], isError: true, timestamp: 1 } }) as unknown as SessionEntry;
const marker = (id: string, parentId: string) =>
  ({ type: 'custom', id, parentId, timestamp: at, customType: 'oino.turn-interrupted', data: { reason: 'tool_calls_closed', at: 1 } }) as unknown as SessionEntry;

let sessions = 0;
/** `turn`: which turn to draw (default the last). */
function setup(entries: SessionEntry[], running = false, turn = -1) {
  const turns = buildTurns({ entries, leafId: entries.at(-1)!.id, entriesRev: 0, liveTools: {}, run: { running } } as never);
  const cmd = vi.fn(() => Promise.resolve());
  const [state, setState] = createStore({ byId: {}, entries, run: { running } });
  const ctx = {
    session: { state, key: `k${++sessions}`, turns: () => turns, cmd },
    hostId: 'h',
    sessionId: () => 's',
    cwd: () => '/',
    refs: { hostId: 'h', cwd: () => '/' },
    commands: () => new Set<string>(),
    columnWidth: () => 720,
  } as unknown as TimelineCtx;
  const r = render(() => <TurnRow turn={turns.at(turn)!} ctx={ctx} last />);
  return { r, cmd, turns, setState };
}

const cutOff = () => [msg('u0', null, 'user', 'Run the tests'), tool('t1', 'u0'), result('r1', 't1', 't1'), marker('m1', 'r1')];

it('shows one "Interrupted — Continue" row outside the Worked fold; Continue sends /continue', () => {
  const { r, cmd, turns } = setup(cutOff());
  expect(turns).toHaveLength(1);
  const row = r.getByTestId('tl-interrupted');
  expect(row.textContent).toContain('Interrupted');
  expect(row.textContent).toContain('Continue');
  expect(row.closest('details')).toBeNull();
  fireEvent.click(r.getByTestId('tl-interrupted-continue'));
  expect(cmd).toHaveBeenCalledWith({ type: 'prompt', message: '/continue' });
});

it('keeps the row as history without a button once the chat went on, and while a run is going', () => {
  const later = [...cutOff(), msg('u1', 'm1', 'user', 'go on'), msg('a1', 'u1', 'assistant', 'Done')];
  // The marker ends that run: "go on" is a new turn (D1), so the row stays in the turn it closed.
  const { r, turns } = setup(later, false, 0);
  expect(turns).toHaveLength(2);
  expect(r.getByTestId('tl-interrupted').textContent).toContain('Interrupted');
  expect(r.queryByTestId('tl-interrupted-continue')).toBeNull();
  cleanup();
  const running = setup(cutOff(), true);
  expect(running.r.queryByTestId('tl-interrupted-continue')).toBeNull();
});

it('D22/P2-2: Continue is pending from the click, past the ACK and any wait, until the run starts', async () => {
  vi.useFakeTimers();
  try {
    const { r, cmd, setState } = setup(cutOff());
    const button = () => r.getByTestId('tl-interrupted-continue') as HTMLButtonElement;
    expect(button().textContent).toBe('Continue');
    fireEvent.click(button());
    expect(button().textContent).toBe('Continuing…');
    expect(button().disabled).toBe(true);
    expect(button().getAttribute('aria-busy')).toBe('true');
    await vi.advanceTimersByTimeAsync(500); // the host ACKed (cmd resolved) but no run has started
    expect(button().textContent).toBe('Continuing…');
    fireEvent.click(button());
    // Well past the old fixed 15 s release: still pending, a second click sends nothing.
    await vi.advanceTimersByTimeAsync(30_000);
    expect(button().textContent).toBe('Continuing…');
    fireEvent.click(button());
    expect(cmd).toHaveBeenCalledTimes(1);
    setState('run', 'running', true); // agent_start: the row is history, the button goes
    expect(r.queryByTestId('tl-interrupted-continue')).toBeNull();
  } finally {
    vi.useRealTimers();
  }
});

it('P2-2: pending while the command itself is still unanswered, however long', async () => {
  vi.useFakeTimers();
  try {
    const { r, cmd } = setup(cutOff());
    cmd.mockImplementationOnce(() => new Promise<void>(() => undefined)); // no ACK yet
    fireEvent.click(r.getByTestId('tl-interrupted-continue'));
    await vi.advanceTimersByTimeAsync(CONTINUE_ACKED_FALLBACK_MS * 2);
    const button = r.getByTestId('tl-interrupted-continue') as HTMLButtonElement;
    expect(button.textContent).toBe('Continuing…');
  } finally {
    vi.useRealTimers();
  }
});

it('P2-2: a message following the marker supersedes the pending Continue', async () => {
  const { r, setState } = setup(cutOff());
  fireEvent.click(r.getByTestId('tl-interrupted-continue'));
  expect((r.getByTestId('tl-interrupted-continue') as HTMLButtonElement).textContent).toBe('Continuing…');
  setState('entries', (list) => [...list, msg('u1', 'm1', 'user', 'go on')]);
  expect(r.queryByTestId('tl-interrupted-continue')).toBeNull();
});

it('P2-2: an ACKed Continue that never starts a run is given back after the fallback', async () => {
  vi.useFakeTimers();
  try {
    const { r } = setup(cutOff());
    fireEvent.click(r.getByTestId('tl-interrupted-continue'));
    await vi.advanceTimersByTimeAsync(CONTINUE_ACKED_FALLBACK_MS + 10);
    expect((r.getByTestId('tl-interrupted-continue') as HTMLButtonElement).textContent).toBe('Continue');
  } finally {
    vi.useRealTimers();
  }
});

it('D22: a failed Continue goes back to a usable button', async () => {
  const { r, cmd } = setup(cutOff());
  cmd.mockRejectedValueOnce(new Error('nope'));
  fireEvent.click(r.getByTestId('tl-interrupted-continue'));
  await new Promise((res) => setTimeout(res, 0));
  const button = r.getByTestId('tl-interrupted-continue') as HTMLButtonElement;
  expect(button.textContent).toBe('Continue');
  expect(button.disabled).toBe(false);
});
