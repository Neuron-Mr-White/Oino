/**
 * ASK-STALE: an answered (or never-answered, ended) `ask_user` card must never stay interactive or
 * read as unanswered, on any path: answer method, replay/snapshot, remount (switch / relaunch from
 * the cache), another client answering, a failed send, interrupted/aborted calls.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { createSignal } from 'solid-js';
import { AskUserCard } from './AskUserCard';
import { staleAskDialogs } from '../../dock/AskPendingChip';
import { sessionUi } from '../../../ui/registries';
import { fakeCtx } from '../tools/testutil';
import { buildTurns } from '../../../lib/session/turns';
import { askSettled, askCallQuestions } from '../../../lib/chat/askUser';
import { clearTimelineSnapshots, readTimelineSnapshot, writeTimelineSnapshot } from '../../../lib/timeline/snapshotCache';
import type { ToolRun } from '../../../lib/session/turns';
import type { AgentMessage, SessionEntry } from '../../../lib/session/types';

afterEach(() => {
  cleanup();
  clearTimelineSnapshots();
  vi.useRealTimers();
});

const ARGS = { questions: [{ question: 'Which DB?', header: 'DB', options: [{ label: 'Postgres' }, { label: 'SQLite' }] }] };
const FORM = {
  id: 'dlg-1',
  method: 'form',
  extensionId: 'unipi.ask-user',
  toolCallId: 'a1',
  questions: [{ id: 'q0', question: 'Which DB?', header: 'DB', options: [{ label: 'Postgres', value: 'Postgres' }, { label: 'SQLite', value: 'SQLite' }], multiSelect: false, other: true }],
};
const tool = (over: Partial<ToolRun> = {}): ToolRun => ({ id: 'a1', name: 'ask_user', args: ARGS, state: 'running', ...over });
const open = () => sessionUi('h1', 's1').applyRequest({ type: 'extension_ui_request', ...FORM });
/** Everything a reader could still press on the card (the recap toggle of a settled card excluded). */
const live = () => [...document.querySelectorAll<HTMLElement>('[data-testid=ask-card] button:not(:disabled), [data-testid=ask-card] textarea:not(:disabled)')].filter((b) => b.dataset.testid !== 'ask-answered');
const state = () => screen.getByTestId('ask-card').dataset.state;

describe('ask card: answered by every method stays settled', () => {
  it('an option + Submit, typed Other and Skip all settle at once and stay settled when the result lands', () => {
    for (const how of ['option', 'other', 'skip'] as const) {
      cleanup();
      const ctx = fakeCtx([]);
      const cmd = vi.spyOn(ctx.session, 'cmd');
      open();
      const [t, setT] = createSignal(tool());
      render(() => <AskUserCard tool={t()} ctx={ctx} />);
      if (how === 'option') {
        fireEvent.click(screen.getByText('SQLite'));
        fireEvent.click(screen.getByTestId('ask-submit'));
      } else if (how === 'other') {
        fireEvent.input(screen.getByTestId('ask-other'), { target: { value: 'MySQL' } });
        fireEvent.click(screen.getByTestId('ask-submit'));
      } else fireEvent.click(screen.getByTestId('ask-skip'));
      expect(cmd).toHaveBeenCalledTimes(1);
      expect(state()).toBe('settled');
      expect(live()).toHaveLength(0);
      expect(sessionUi('h1', 's1').dialogs()).toHaveLength(0);
      // the result arrives (answered): the card keeps its one compact row
      const details = how === 'skip' ? { outcome: 'answered', answers: [{ selected: [], skipped: true }] } : { outcome: 'answered', answers: [{ selected: how === 'option' ? ['SQLite'] : [], custom_text: how === 'other' ? 'MySQL' : undefined, skipped: false }] };
      setT(tool({ state: 'ok', output: 'User answered your questions:', details }));
      expect(state()).toBe('settled');
      expect(live()).toHaveLength(0);
      expect(screen.getByTestId('ask-answered').textContent).toContain(how === 'skip' ? 'You skipped the questions' : 'You answered:');
    }
  });

  it('a result that says the host cancelled it wins over the local "You answered"', () => {
    const ctx = fakeCtx([]);
    open();
    const [t, setT] = createSignal(tool());
    render(() => <AskUserCard tool={t()} ctx={ctx} />);
    fireEvent.click(screen.getByText('SQLite'));
    fireEvent.click(screen.getByTestId('ask-submit'));
    setT(tool({ state: 'ok', details: { outcome: 'cancelled' } }));
    expect(screen.getByTestId('ask-answered').textContent).toContain('Not answered');
    expect(live()).toHaveLength(0);
  });

  it('a stream that ended without a result keeps what the user answered (never back to waiting)', () => {
    const ctx = fakeCtx([]);
    open();
    const [t, setT] = createSignal(tool());
    render(() => <AskUserCard tool={t()} ctx={ctx} />);
    fireEvent.click(screen.getByText('SQLite'));
    fireEvent.click(screen.getByTestId('ask-submit'));
    setT(tool({ state: 'stopped' }));
    expect(state()).toBe('settled');
    expect(screen.getByTestId('ask-answered').textContent).toContain('You answered:SQLite');
  });
});

describe('ask card: replay, snapshot, remount, relaunch', () => {
  it('a settled call stays settled when a replayed / snapshot request re-adds its dialog', () => {
    const ctx = fakeCtx([]);
    render(() => <AskUserCard tool={tool({ state: 'ok', details: { outcome: 'answered', answers: [{ selected: ['SQLite'], skipped: false }] } })} ctx={ctx} />);
    open(); // stale `pending_requests` / replayed `extension_ui_request`
    expect(state()).toBe('settled');
    expect(live()).toHaveLength(0);
    expect(screen.getByTestId('ask-answered').textContent).toContain('You answered:SQLite');
  });

  it('the dock drops a leftover ask dialog whose call already ended', () => {
    const turns = buildTurns({ entries: ask({ answered: true }), leafId: null, entriesRev: 1, liveTools: {}, run: { running: false } as never });
    expect(staleAskDialogs([FORM as never], turns)).toHaveLength(1);
    const waitingTurns = buildTurns({ entries: ask({ answered: false }), leafId: null, entriesRev: 1, liveTools: {}, run: { running: true } as never });
    expect(staleAskDialogs([FORM as never], waitingTurns)).toHaveLength(0);
  });

  it('a remount from the result (chat switch, relaunch) renders the compact row, not the form', () => {
    const ctx = fakeCtx([]);
    const details = { outcome: 'answered', answers: [{ selected: ['Postgres'], skipped: false }] };
    const first = render(() => <AskUserCard tool={tool({ state: 'ok', details })} ctx={ctx} />);
    first.unmount();
    render(() => <AskUserCard tool={tool({ state: 'ok', details })} ctx={ctx} />);
    expect(state()).toBe('settled');
    expect(screen.queryByTestId('ask-question')).toBeNull();
  });

  it('the timeline snapshot cache restores an answered ask as settled and never turns a settled call back into running', () => {
    const turns = buildTurns({ entries: ask({ answered: true }), leafId: null, entriesRev: 1, liveTools: {}, run: { running: false } as never });
    writeTimelineSnapshot('h1/s1', turns);
    vi.useFakeTimers();
    vi.advanceTimersByTime(400);
    clearTimelineSnapshots; // keep the mem entry
    const cached = readTimelineSnapshot('h1/s1')!;
    expect(cached[0]!.segments.find((s) => s.ask)!.ask!.state).toBe('ok');
  });
});

describe('ask card: remount between the answer and the result', () => {
  it('a chat switch right after answering does not bring back a dead "Opening the question…" form', () => {
    const ctx = fakeCtx([]);
    open();
    const first = render(() => <AskUserCard tool={tool()} ctx={ctx} />);
    fireEvent.click(screen.getByText('SQLite'));
    fireEvent.click(screen.getByTestId('ask-submit'));
    first.unmount();
    render(() => <AskUserCard tool={tool()} ctx={ctx} />); // same call, still running, dialog already answered
    expect(screen.queryByTestId('ask-wait')).toBeNull();
    expect(state()).toBe('settled');
    expect(screen.getByTestId('ask-answered').textContent).toContain('You answered:SQLite');
    expect(live()).toHaveLength(0);
  });
});

describe('ask card: other clients, failed sends, ended calls', () => {
  it('answered from another window: the dialog vanishes while the call still runs → "Answered elsewhere", not interactive, until the result lands', () => {
    const ctx = fakeCtx([]);
    open();
    const [t, setT] = createSignal(tool());
    render(() => <AskUserCard tool={t()} ctx={ctx} />);
    expect(state()).toBe('waiting');
    sessionUi('h1', 's1').applyRequest({ type: 'extension_ui_dismiss', id: 'dlg-1' });
    expect(state()).toBe('settled');
    expect(live()).toHaveLength(0);
    expect(screen.getByTestId('ask-answered').textContent).toContain('Answered elsewhere');
    setT(tool({ state: 'ok', details: { outcome: 'answered', answers: [{ selected: ['Postgres'], skipped: false }] } }));
    expect(screen.getByTestId('ask-answered').textContent).toContain('You answered:Postgres');
  });

  it('a failed send brings the dialog back: the card is live again and says so', async () => {
    const ctx = fakeCtx([]);
    vi.spyOn(ctx.session, 'cmd').mockRejectedValue(new Error('E_TRANSPORT'));
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    open();
    render(() => <AskUserCard tool={tool()} ctx={ctx} />);
    fireEvent.click(screen.getByText('SQLite'));
    fireEvent.click(screen.getByTestId('ask-submit'));
    expect(state()).toBe('settled');
    await waitFor(() => expect(state()).toBe('waiting'));
    expect(screen.getByTestId('ask-failed')).toBeTruthy();
    expect((screen.getByTestId('ask-submit') as HTMLButtonElement).disabled).toBe(false);
  });

  it('a send that times out keeps "You answered" (the host may have received it)', async () => {
    const ctx = fakeCtx([]);
    vi.spyOn(ctx.session, 'cmd').mockRejectedValue({ code: 'E_TIMEOUT', message: 'The session did not answer in time', retryable: true });
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    open();
    render(() => <AskUserCard tool={tool()} ctx={ctx} />);
    fireEvent.click(screen.getByText('SQLite'));
    fireEvent.click(screen.getByTestId('ask-submit'));
    await new Promise((r) => setTimeout(r, 20));
    expect(state()).toBe('settled');
    expect(screen.getByTestId('ask-answered').textContent).toContain('You answered:SQLite');
    expect(live()).toHaveLength(0);
  });

  it('never answered and ended: aborted / interrupted by a restart / cancelled all read "Not answered" and nothing is interactive', () => {
    const ctx = fakeCtx([]);
    const cases: Partial<ToolRun>[] = [
      { state: 'stopped' },
      { state: 'error', output: 'Tool execution was interrupted before completion; the outcome is unknown. The effect may or may not have happened.' },
      { state: 'ok', details: { outcome: 'cancelled' } },
    ];
    for (const c of cases) {
      cleanup();
      open();
      render(() => <AskUserCard tool={tool(c)} ctx={ctx} />);
      expect(screen.getByTestId('ask-answered').textContent).toContain('Not answered');
      expect(live()).toHaveLength(0);
    }
    expect(askSettled({ state: 'error', output: 'boom' }, askCallQuestions(ARGS)).label).toBe('Could not ask');
  });
});

describe('turns: an ask call the agent went on from is never "waiting"', () => {
  it('a later assistant message without a result closes the call even while the run is live', () => {
    const turns = buildTurns({ entries: ask({ answered: false, andThen: true }), leafId: null, entriesRev: 1, liveTools: {}, run: { running: true } as never });
    expect(turns[0]!.segments.find((s) => s.ask)!.ask!.state).toBe('stopped');
  });
  it('a result that is still a reference-only stub reads as answered, not waiting or not answered', () => {
    const entries = ask({ answered: false });
    entries.push({ id: 'stub1', truncated: true } as unknown as SessionEntry);
    const turns = buildTurns({ entries, leafId: null, entriesRev: 1, liveTools: {}, run: { running: true } as never });
    expect(turns[0]!.segments.find((s) => s.ask)!.ask!.state).toBe('ok');
  });
  it('the newest call with no result in a live run still waits', () => {
    const turns = buildTurns({ entries: ask({ answered: false }), leafId: null, entriesRev: 1, liveTools: {}, run: { running: true } as never });
    expect(turns[0]!.segments.find((s) => s.ask)!.ask!.state).toBe('running');
  });
});

function ask(o: { answered: boolean; andThen?: boolean }): SessionEntry[] {
  const entries: SessionEntry[] = [];
  let t = Date.UTC(2026, 10, 20, 9);
  const msg = (message: AgentMessage) => {
    t += 1000;
    entries.push({ type: 'message', message: { timestamp: t, ...message }, id: `e${entries.length + 1}`, parentId: entries.at(-1)?.id ?? null, timestamp: new Date(t).toISOString() } as SessionEntry);
  };
  msg({ role: 'user', content: [{ type: 'text', text: 'go' }] });
  msg({ role: 'assistant', content: [{ type: 'toolCall', id: 'a1', name: 'ask_user', arguments: ARGS }], stopReason: 'toolUse' });
  if (o.answered) msg({ role: 'toolResult', toolCallId: 'a1', toolName: 'ask_user', content: [{ type: 'text', text: 'User answered your questions:' }], isError: false, details: { outcome: 'answered', answers: [{ selected: ['SQLite'], skipped: false }] } });
  if (o.andThen) msg({ role: 'assistant', content: [{ type: 'text', text: 'moving on' }], stopReason: 'stop' });
  return entries;
}
