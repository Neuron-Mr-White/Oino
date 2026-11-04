/** [ask-user] inline question card + the dock chip (owner 2026-11-04). */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import { AskUserCard } from './AskUserCard';
import { askDialogFor, askQuestions, askResponse, askSettled, isAskDialog, skipResponse } from '../../../lib/chat/askUser';
import { inlineAskCalls } from '../../dock/AskPendingChip';
import { sessionUi } from '../../../ui/registries';
import { fakeCtx } from '../tools/testutil';
import type { ToolRun, Turn } from '../../../lib/session/turns';

afterEach(cleanup);

const ARGS = {
  questions: [
    { question: 'Which DB?', header: 'DB', options: [{ label: 'Postgres', description: 'server' }, { label: 'SQLite', value: 'sqlite' }] },
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

describe('askUser helpers', () => {
  it('reads questions from args, details and the form request alike', () => {
    expect(askQuestions(ARGS.questions).map((q) => [q.id, q.multi, q.options.map((o) => o.value)])).toEqual([
      ['q0', false, ['Postgres', 'sqlite']],
      ['q1', true, ['Auth', 'Cache']],
    ]);
    expect(askQuestions(FORM.questions)[1]!.multi).toBe(true);
    expect(askQuestions([{ question: 'Name?', options: [], other: false }])[0]!.other).toBe(true);
    expect(askQuestions('nope')).toEqual([]);
  });

  it('finds the call\'s dialog by toolCallId, else by the same questions from unipi.ask-user', () => {
    expect(askDialogFor([FORM], tool())?.id).toBe('dlg-1');
    expect(askDialogFor([{ ...FORM, toolCallId: 'other' }], tool())).toBeUndefined();
    const { toolCallId: _drop, ...old } = FORM;
    expect(askDialogFor([old], tool())?.id).toBe('dlg-1');
    expect(isAskDialog({ id: 'x', method: 'select', extensionId: 'unipi.ask-user' })).toBe(false);
  });

  it('answers in the form shape decode reads; Skip skips every question (never a cancel)', () => {
    const qs = askQuestions(FORM.questions);
    expect(askResponse(qs, [{ selected: ['sqlite'], skipped: false }, { selected: [], custom_text: ' Rate limits ', skipped: false }])).toEqual({
      answers: [
        { id: 'q0', selected: ['sqlite'] },
        { id: 'q1', selected: [], other: 'Rate limits' },
      ],
    });
    expect(skipResponse(qs)).toEqual({ answers: [{ id: 'q0', selected: [] }, { id: 'q1', selected: [] }] });
  });

  it('settled lines: answered (labels, not values), skipped, dismissed, stopped, not shown', () => {
    const qs = askQuestions(ARGS.questions);
    const answered = askSettled(
      { state: 'ok', details: { outcome: 'answered', answers: [{ selected: ['sqlite'], skipped: false }, { selected: ['Auth', 'Cache'], skipped: false }] } },
      qs,
    );
    expect(answered).toEqual({ tone: 'ok', label: 'You answered', text: 'DB: SQLite · Extras: Auth, Cache' });
    expect(askSettled({ state: 'ok', details: { outcome: 'answered', answers: [{ selected: [], skipped: true }, { selected: [], skipped: true }] } }, qs).label).toBe('You skipped the questions');
    expect(askSettled({ state: 'ok', details: { outcome: 'cancelled' } }, qs).label).toBe('You dismissed the questions');
    expect(askSettled({ state: 'stopped' }, qs).label).toBe('Not answered');
    expect(askSettled({ state: 'ok', details: { outcome: 'unavailable', reason: 'disabled' } }, qs)).toMatchObject({ label: 'Not shown' });
    // No details (older sessions): the result text's JSON.
    const fromText = askSettled({ state: 'ok', output: 'User answered your questions:\n{"Which DB?": {"selected": ["Postgres"], "skipped": false}}' }, qs.slice(0, 1));
    expect(fromText).toEqual({ tone: 'ok', label: 'You answered', text: 'Postgres' });
  });
});

describe('AskUserCard', () => {
  it('waiting: question(s), option buttons, multi-select, Other, Submit sends the answers and collapses in place', async () => {
    const ctx = fakeCtx([]);
    const cmd = vi.spyOn(ctx.session, 'cmd');
    sessionUi('h1', 's1').applyRequest({ type: 'extension_ui_request', ...FORM });
    render(() => <AskUserCard tool={tool()} ctx={ctx} />);
    expect(screen.getAllByTestId('ask-question')).toHaveLength(2);
    expect(screen.getByText('Which DB?')).toBeTruthy();
    const submit = screen.getByTestId('ask-submit') as HTMLButtonElement;
    expect(submit.disabled).toBe(true);
    fireEvent.click(screen.getByText('SQLite'));
    fireEvent.click(screen.getByText('Auth'));
    fireEvent.click(screen.getByText('Cache'));
    expect(screen.getAllByRole('checkbox').map((b) => b.getAttribute('aria-checked'))).toEqual(['true', 'true']);
    fireEvent.click(submit);
    expect(cmd).toHaveBeenCalledWith({
      type: 'extension_ui_response',
      id: 'dlg-1',
      value: { answers: [{ id: 'q0', selected: ['sqlite'] }, { id: 'q1', selected: ['Auth', 'Cache'] }] },
    });
    expect(sessionUi('h1', 's1').dialogs()).toHaveLength(0);
    expect(screen.getByTestId('ask-answered').textContent).toContain('You answered:DB: SQLite · Extras: Auth, Cache');
    expect(screen.queryByTestId('ask-question')).toBeNull();
  });

  it('Skip answers every question skipped; typing Other replaces a single choice', () => {
    const ctx = fakeCtx([]);
    const cmd = vi.spyOn(ctx.session, 'cmd');
    sessionUi('h1', 's1').applyRequest({ type: 'extension_ui_request', ...FORM });
    render(() => <AskUserCard tool={tool()} ctx={ctx} />);
    fireEvent.click(screen.getByText('Postgres'));
    fireEvent.input(screen.getAllByTestId('ask-other')[0]!, { target: { value: 'MySQL' } });
    expect(screen.getAllByRole('radio').every((b) => b.getAttribute('aria-checked') === 'false')).toBe(true);
    fireEvent.click(screen.getByTestId('ask-skip'));
    expect(cmd).toHaveBeenCalledWith({ type: 'extension_ui_response', id: 'dlg-1', value: { answers: [{ id: 'q0', selected: [] }, { id: 'q1', selected: [] }] } });
    expect(screen.getByTestId('ask-answered').textContent).toContain('You skipped the questions');
  });

  it('before the dialog arrives the card shows the questions but cannot be answered', () => {
    const ctx = fakeCtx([]);
    sessionUi('h1', 's1').applySnapshot?.({ scope: 'session', sessionId: 's1', rev: 99, entries: [], dialogs: [] });
    render(() => <AskUserCard tool={tool({ id: 'zz' })} ctx={ctx} />);
    expect(screen.getByTestId('ask-wait')).toBeTruthy();
    expect((screen.getByTestId('ask-skip') as HTMLButtonElement).disabled).toBe(true);
  });

  it('settled: one compact row that expands to the Q&A', () => {
    const ctx = fakeCtx([]);
    render(() => (
      <AskUserCard tool={tool({ id: 'a9', state: 'ok', details: { outcome: 'answered', answers: [{ selected: ['Postgres'], skipped: false }, { selected: [], skipped: true }] } })} ctx={ctx} />
    ));
    const row = screen.getByTestId('ask-answered');
    expect(row.textContent).toContain('DB: Postgres · Extras: skipped');
    expect(screen.queryByTestId('ask-recap')).toBeNull();
    fireEvent.click(row);
    expect(screen.getByTestId('ask-recap').textContent).toContain('Which DB?');
    expect(screen.getByTestId('ask-recap').textContent).toContain('Postgres');
  });
});

describe('dock: ask dialogs are inline', () => {
  it('a waiting call in the loaded turns takes its dialog out of the dock', () => {
    const turn = { turnId: 't1', segments: [{ id: 't1', ask: tool() }] } as unknown as Turn;
    const other = { id: 'd2', method: 'select', title: 'Pick', options: ['a'] };
    expect(inlineAskCalls([FORM, other], [turn])).toEqual([{ turnId: 't1', toolId: 'a1', dialog: FORM }]);
    expect(inlineAskCalls([FORM], [])).toEqual([]);
    expect(inlineAskCalls([other], [turn])).toEqual([]);
  });
});
