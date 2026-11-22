/**
 * ASK-STALE: every path by which an `ask_user` card could stay on screen as an unanswered,
 * interactive form after it was answered (or after its call ended without an answer).
 * Contract under test, whatever the path:
 *   - answered  -> one compact "You answered: …" row, nothing clickable (no options / Skip / Submit);
 *   - never answered and the call is over (stopped, interrupted, cancelled) -> "Not answered", nothing clickable.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, within } from '@solidjs/testing-library';
import { AskUserCard } from './AskUserCard';
import { buildTurns, type Turn, type TurnsInput } from '../../../lib/session/turns';
import type { SessionEntry } from '../../../lib/session/types';
import type { SessionSnapshot } from '../../../lib/host/client';
import { applySnapshot, initialState } from '../../../lib/session/reducer';
import { routeSessionUi, sessionUi } from '../../../ui/registries';
import { fakeCtx } from '../tools/testutil';
import { setDesktopOverride } from '../../../lib/layout';
import { readTimelineSnapshot, writeTimelineSnapshot, clearTimelineSnapshots } from '../../../lib/timeline/snapshotCache';
import type { ToolRun } from '../../../lib/session/turns';

beforeEach(() => {
  setDesktopOverride(true);
  sessionStorage.clear();
});
afterEach(() => {
  cleanup();
  setDesktopOverride(undefined);
  clearTimelineSnapshots();
  vi.restoreAllMocks();
});

const ARGS = {
  questions: [
    { question: 'Which DB?', header: 'DB', options: [{ label: 'Postgres' }, { label: 'SQLite', value: 'sqlite' }] },
    { question: 'Extras?', header: 'Extras', multi_select: true, options: [{ label: 'Auth' }, { label: 'Cache' }] },
  ],
};
const FORM = {
  id: 'dlg-1',
  method: 'form',
  extensionId: 'unipi.ask-user',
  toolCallId: 'a1',
  questions: [
    { id: 'q0', question: 'Which DB?', header: 'DB', options: [{ label: 'Postgres', value: 'Postgres' }, { label: 'SQLite', value: 'sqlite' }], multiSelect: false, other: true },
    { id: 'q1', question: 'Extras?', header: 'Extras', options: [{ label: 'Auth', value: 'Auth' }, { label: 'Cache', value: 'Cache' }], multiSelect: true, other: true },
  ],
};
const tool = (over: Partial<ToolRun> = {}): ToolRun => ({ id: 'a1', name: 'ask_user', args: ARGS, state: 'running', ...over });
const openDialog = () => sessionUi('h1', 's1').applyRequest({ type: 'extension_ui_request', ...FORM });

/** Nothing on the card can be clicked to answer. */
function expectInert() {
  expect(screen.queryByTestId('ask-submit')).toBeNull();
  expect(screen.queryByTestId('ask-skip')).toBeNull();
  expect(screen.queryAllByTestId('ask-option')).toHaveLength(0);
  expect(screen.queryAllByTestId('ask-other')).toHaveLength(0);
  expect(screen.getByTestId('ask-card').getAttribute('data-state')).toBe('settled');
}
const rowText = () => screen.getByTestId('ask-answered').textContent ?? '';

describe('answered from the card: every method collapses to the summary', () => {
  it('by clicking options', () => {
    const ctx = fakeCtx([]);
    openDialog();
    render(() => <AskUserCard tool={tool()} ctx={ctx} />);
    fireEvent.click(screen.getByText('SQLite'));
    fireEvent.click(screen.getByText('Auth'));
    fireEvent.click(screen.getByTestId('ask-submit'));
    expect(rowText()).toContain('You answered:');
    expect(rowText()).toContain('DB: SQLite · Extras: Auth');
    expectInert();
  });

  it('by typing Other', () => {
    const ctx = fakeCtx([]);
    openDialog();
    render(() => <AskUserCard tool={tool()} ctx={ctx} />);
    fireEvent.input(screen.getAllByTestId('ask-other')[0]!, { target: { value: 'MySQL' } });
    fireEvent.click(screen.getByTestId('ask-submit'));
    expect(rowText()).toContain('MySQL');
    expectInert();
  });

  it('by Skip', () => {
    const ctx = fakeCtx([]);
    openDialog();
    render(() => <AskUserCard tool={tool()} ctx={ctx} />);
    fireEvent.click(screen.getByTestId('ask-skip'));
    expect(rowText()).toContain('You skipped the questions');
    expectInert();
  });

  it('the answered row survives the call result landing (details replace the local recap)', () => {
    const ctx = fakeCtx([]);
    openDialog();
    const first = render(() => <AskUserCard tool={tool()} ctx={ctx} />);
    fireEvent.click(screen.getByText('Postgres'));
    fireEvent.click(screen.getByTestId('ask-submit'));
    first.unmount();
    render(() => (
      <AskUserCard
        tool={tool({ state: 'ok', details: { outcome: 'answered', answers: [{ selected: ['Postgres'], skipped: false }, { selected: [], skipped: true }] } })}
        ctx={ctx}
      />
    ));
    expect(rowText()).toContain('DB: Postgres');
    expectInert();
  });
});

describe('answered, then the card remounts before the result lands (chat switch, scroll away and back, restore)', () => {
  it('stays "You answered", never a dead "Opening the question…" form', () => {
    const ctx = fakeCtx([]);
    openDialog();
    const first = render(() => <AskUserCard tool={tool()} ctx={ctx} />);
    fireEvent.click(screen.getByText('SQLite'));
    fireEvent.click(screen.getByTestId('ask-submit'));
    first.unmount();
    // Same call, still `running` (the result has not landed), its dialog already answered.
    render(() => <AskUserCard tool={tool()} ctx={ctx} />);
    expect(screen.queryByTestId('ask-wait')).toBeNull();
    expect(rowText()).toContain('You answered');
    expect(rowText()).toContain('SQLite');
    expectInert();
  });
});

describe('answered somewhere else (phone, second window, terminal)', () => {
  it('the dialog goes away while the call still runs: the card is not an answerable form', () => {
    const ctx = fakeCtx([]);
    openDialog();
    render(() => <AskUserCard tool={tool()} ctx={ctx} />);
    expect(screen.getByTestId('ask-submit')).toBeTruthy();
    routeSessionUi('h1', 's1', { type: 'extension_ui_dismiss', id: 'dlg-1', reason: 'answered' });
    expect(screen.queryByTestId('ask-submit')).toBeNull();
    expect(screen.queryAllByTestId('ask-option')).toHaveLength(0);
    expect(screen.getByTestId('ask-card').getAttribute('data-state')).toBe('settled');
  });

  it('a replayed request after the call settled does not bring the form back', () => {
    const ctx = fakeCtx([]);
    openDialog();
    const settled = tool({ state: 'ok', details: { outcome: 'answered', answers: [{ selected: ['Postgres'], skipped: false }, { selected: [], skipped: true }] } });
    render(() => <AskUserCard tool={settled} ctx={ctx} />);
    // A resume / resync replays the old request (and its pending_requests entry) after the result.
    routeSessionUi('h1', 's1', { type: 'extension_ui_request', ...FORM });
    expectInert();
  });
});

describe('hide / restore, relaunch: snapshots never resurrect an answered question', () => {
  const answeredEntries = (): SessionEntry[] =>
    [
      { type: 'message', id: 'u1', parentId: null, timestamp: '2026-11-20T10:00:00Z', message: { role: 'user', content: [{ type: 'text', text: 'set it up' }], timestamp: 1 } },
      { type: 'message', id: 'e2', parentId: 'u1', timestamp: '2026-11-20T10:00:01Z', message: { role: 'assistant', content: [{ type: 'toolCall', id: 'a1', name: 'ask_user', arguments: ARGS }], stopReason: 'toolUse', timestamp: 2 } },
      {
        type: 'message',
        id: 'e3',
        parentId: 'e2',
        timestamp: '2026-11-20T10:00:09Z',
        message: { role: 'toolResult', toolCallId: 'a1', toolName: 'ask_user', content: [{ type: 'text', text: 'ok' }], isError: false, details: { outcome: 'answered', answers: [{ selected: ['Postgres'], skipped: false }, { selected: [], skipped: true }] }, timestamp: 3 },
      },
    ] as unknown as SessionEntry[];

  const input = (entries: SessionEntry[], running: boolean): TurnsInput => ({ entries, leafId: entries.at(-1)?.id ?? null, entriesRev: 1, liveTools: {}, run: running ? { running: true, startedAt: 1 } : { running: false } });
  const askOf = (turns: Turn[]) => turns[0]!.segments.find((s) => s.ask)!.ask!;

  it('a resume snapshot whose pending_requests dropped the dialog removes it from the registry; the card follows the result', () => {
    const s = initialState('h1', 's1');
    const dialog = { type: 'extension_ui_request', ...FORM };
    applySnapshot(s, { session_id: 's1', seq: 5, reset: true, entries: [], pending_requests: [dialog] } as unknown as SessionSnapshot);
    expect(s.dialogs).toContain('dlg-1');
    const r = applySnapshot(s, { session_id: 's1', seq: 9, reset: false, replay: [], pending_requests: [] } as unknown as SessionSnapshot);
    for (const msg of r.ui ?? []) routeSessionUi('h1', 's1', msg as Record<string, unknown>);
    routeSessionUi('h1', 's1', dialog);
    routeSessionUi('h1', 's1', { type: 'extension_ui_dismiss', id: 'dlg-1' });
    expect(s.dialogs).not.toContain('dlg-1');
    const ctx = fakeCtx([]);
    render(() => <AskUserCard tool={askOf(buildTurns(input(answeredEntries(), false)))} ctx={ctx} />);
    expect(rowText()).toContain('DB: Postgres');
    expectInert();
  });

  it('a full snapshot after answering shows the result, not the question', () => {
    const turns = buildTurns(input(answeredEntries(), true));
    expect(askOf(turns).state).toBe('ok');
    const ctx = fakeCtx([]);
    render(() => <AskUserCard tool={askOf(turns)} ctx={ctx} />);
    expect(rowText()).toContain('You answered:');
    expectInert();
  });

  it('relaunch: the persisted timeline cache restores an answered card as the summary, a waiting one as a disabled form', () => {
    const settledTurns = buildTurns(input(answeredEntries(), false));
    writeTimelineSnapshot('h1/cache-a', settledTurns);
    const restored = readTimelineSnapshot('h1/cache-a')!;
    render(() => <AskUserCard tool={askOf(restored)} ctx={fakeCtx([])} />);
    expect(rowText()).toContain('DB: Postgres');
    expectInert();
    cleanup();

    const waiting = buildTurns(input(answeredEntries().slice(0, 2), true));
    expect(askOf(waiting).state).toBe('running');
    writeTimelineSnapshot('h1/cache-b', waiting);
    const back = readTimelineSnapshot('h1/cache-b')!;
    const ctx = fakeCtx([]);
    sessionUi('h1', 's1').applySnapshot?.({ scope: 'session', sessionId: 's1', rev: 77, entries: [], dialogs: [] });
    render(() => <AskUserCard tool={askOf(back)} ctx={ctx} />);
    // No dialog yet (the live stream has not opened): the options cannot be clicked, Submit / Skip are disabled.
    expect((screen.getByTestId('ask-skip') as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByTestId('ask-submit') as HTMLButtonElement).disabled).toBe(true);
    screen.getAllByTestId('ask-option').forEach((o) => expect((o as HTMLButtonElement).disabled).toBe(true));
  });
});

describe('a question that was never answered and whose call ended', () => {
  const ended = (over: Partial<ToolRun>) => {
    render(() => <AskUserCard tool={tool(over)} ctx={fakeCtx([])} />);
    expect(rowText()).toContain('Not answered');
    expectInert();
  };

  it('aborted (the run was stopped)', () => ended({ state: 'stopped' }));
  it('stopped from the composer: the extension reports a cancel', () => ended({ state: 'ok', details: { outcome: 'cancelled' } }));
  it('session restarted: the host closes the call as interrupted', () =>
    ended({ state: 'error', output: 'Tool execution was interrupted before completion; the outcome is unknown. The effect may or may not have happened.' }));
  it('cancel written as plain text (older results)', () => ended({ state: 'ok', output: 'User cancelled the questions.' }));
});

describe('the answer could not be delivered', () => {
  it('the form comes back with a retry, instead of a dead card', async () => {
    const ctx = fakeCtx([]);
    let fail = true;
    const cmd = vi.spyOn(ctx.session, 'cmd').mockImplementation(((command: { type: string }) => (command.type === 'extension_ui_response' && fail ? Promise.reject(new Error('E_TRANSPORT')) : Promise.resolve({}))) as never);
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    openDialog();
    render(() => <AskUserCard tool={tool()} ctx={ctx} />);
    fireEvent.click(screen.getByText('Postgres'));
    fireEvent.click(screen.getByTestId('ask-submit'));
    await Promise.resolve();
    await Promise.resolve();
    expect(screen.getByTestId('ask-submit')).toBeTruthy();
    expect((screen.getByTestId('ask-submit') as HTMLButtonElement).disabled).toBe(false);
    fail = false;
    fireEvent.click(screen.getByTestId('ask-submit'));
    expect(cmd).toHaveBeenCalledTimes(2);
    expect(rowText()).toContain('You answered');
    expectInert();
  });
});

describe('the host is slow: the answer command times out (E_TIMEOUT after 30 s)', () => {
  // Packaged 1.0.12 rig (host wedged with SIGSTOP, answer clicked): the card showed "You answered" for
  // 30 s, then fell back to the unanswered form ("Opening the question…", disabled, sidebar "Needs you")
  // although the host received the answer as soon as it woke. A timeout is not a failure: the answer
  // may have landed, so the card keeps the answered summary until the result says otherwise.
  it('keeps "You answered" and never shows the question again', async () => {
    const ctx = fakeCtx([]);
    vi.spyOn(ctx.session, 'cmd').mockImplementation(((command: { type: string }) =>
      command.type === 'extension_ui_response' ? Promise.reject({ code: 'E_TIMEOUT', message: 'The session did not answer in time', retryable: true }) : Promise.resolve({})) as never);
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    openDialog();
    render(() => <AskUserCard tool={tool()} ctx={ctx} />);
    fireEvent.click(screen.getByText('SQLite'));
    fireEvent.click(screen.getByTestId('ask-submit'));
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
    expect(rowText()).toContain('You answered');
    expect(rowText()).toContain('SQLite');
    expectInert();
  });
});

describe('a timed-out answer that never reached the host', () => {
  it('when the host announces the question again, the live form returns (the summary would be a lie)', async () => {
    const ctx = fakeCtx([]);
    vi.spyOn(ctx.session, 'cmd').mockImplementation((() => Promise.reject({ code: 'E_TIMEOUT', message: 'The session did not answer in time', retryable: true })) as never);
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    openDialog();
    render(() => <AskUserCard tool={tool()} ctx={ctx} />);
    fireEvent.click(screen.getByText('SQLite'));
    fireEvent.click(screen.getByTestId('ask-submit'));
    await Promise.resolve();
    await Promise.resolve();
    expect(rowText()).toContain('You answered');
    // Reconnect: the snapshot's pending_requests still lists the dialog.
    routeSessionUi('h1', 's1', { type: 'extension_ui_request', ...FORM });
    expect(screen.getByTestId('ask-submit')).toBeTruthy();
    expect((screen.getByTestId('ask-submit') as HTMLButtonElement).disabled).toBe(false);
    expect(screen.getByTestId('ask-failed')).toBeTruthy();
  });
});

describe('answered while the turn keeps running', () => {
  it('a call that has no result yet but whose run moved on (a later assistant message) is over, never waiting', () => {
    const entries = [
      { type: 'message', id: 'u1', parentId: null, timestamp: '2026-11-20T10:00:00Z', message: { role: 'user', content: [{ type: 'text', text: 'go' }], timestamp: 1 } },
      { type: 'message', id: 'e2', parentId: 'u1', timestamp: '2026-11-20T10:00:01Z', message: { role: 'assistant', content: [{ type: 'toolCall', id: 'a1', name: 'ask_user', arguments: ARGS }], stopReason: 'toolUse', timestamp: 2 } },
      { type: 'message', id: 'e3', parentId: 'e2', timestamp: '2026-11-20T10:00:20Z', message: { role: 'assistant', content: [{ type: 'text', text: 'carrying on' }], stopReason: 'stop', timestamp: 3 } },
    ] as unknown as SessionEntry[];
    const turns = buildTurns({ entries, leafId: 'e3', entriesRev: 1, liveTools: {}, run: { running: true, startedAt: 1 } });
    const ask = turns[0]!.segments.find((s) => s.ask)!.ask!;
    expect(ask.state).not.toBe('running');
    render(() => <AskUserCard tool={ask} ctx={fakeCtx([])} />);
    expectInert();
  });

  it('a reference-only stub where the result should be counts as answered', () => {
    const entries = [
      { type: 'message', id: 'u1', parentId: null, timestamp: '2026-11-20T10:00:00Z', message: { role: 'user', content: [{ type: 'text', text: 'go' }], timestamp: 1 } },
      { type: 'message', id: 'e2', parentId: 'u1', timestamp: '2026-11-20T10:00:01Z', message: { role: 'assistant', content: [{ type: 'toolCall', id: 'a1', name: 'ask_user', arguments: ARGS }], stopReason: 'toolUse', timestamp: 2 } },
      { id: 'e3', truncated: true },
    ] as unknown as SessionEntry[];
    const turns = buildTurns({ entries, leafId: 'e3', entriesRev: 1, liveTools: {}, run: { running: true, startedAt: 1 } });
    const ask = turns[0]!.segments.find((s) => s.ask)!.ask!;
    expect(ask.state).toBe('ok');
    render(() => <AskUserCard tool={ask} ctx={fakeCtx([])} />);
    expectInert();
  });
});

describe('phone layout', () => {
  it('answering inside the full-screen sheet closes it and leaves the summary', () => {
    setDesktopOverride(false);
    const ctx = fakeCtx([]);
    openDialog();
    render(() => <AskUserCard tool={tool()} ctx={ctx} />);
    fireEvent.click(screen.getByTestId('ask-open-sheet'));
    expect(screen.getByTestId('ask-sheet')).toBeTruthy();
    const inSheet = within(screen.getByTestId('ask-sheet'));
    fireEvent.click(inSheet.getByText('Postgres'));
    fireEvent.click(inSheet.getByTestId('ask-submit'));
    expect(screen.queryByTestId('ask-sheet')).toBeNull();
    expect(rowText()).toContain('DB: Postgres');
    expectInert();
  });
});
