/** [ask-user] inline question card + the dock chip (owner 2026-11-04). */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import { AskUserCard } from './AskUserCard';
import { askCallQuestions, askDialogFor, askQuestions, askResponse, askSettled, isAskDialog, skipResponse } from '../../../lib/chat/askUser';
import { inlineAskCalls } from '../../dock/AskPendingChip';
import { sessionUi } from '../../../ui/registries';
import { fakeCtx } from '../tools/testutil';
import { answerPendingAsk, answerPendingAskWithNote, hasPendingAsk } from '../../../lib/chat/askPending';
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

  it('owner-fix-9: a waiting card is pending for its chat; a composer send submits its picks, then it is no longer pending', () => {
    const ctx = fakeCtx([]);
    const cmd = vi.spyOn(ctx.session, 'cmd');
    sessionUi('h1', 's1').applyRequest({ type: 'extension_ui_request', ...FORM });
    render(() => <AskUserCard tool={tool()} ctx={ctx} />);
    expect(hasPendingAsk(ctx.session.key)).toBe(true);
    fireEvent.click(screen.getByText('SQLite'));
    expect(answerPendingAsk(ctx.session.key)).toBe(true);
    expect(cmd).toHaveBeenCalledWith({ type: 'extension_ui_response', id: 'dlg-1', value: { answers: [{ id: 'q0', selected: ['sqlite'] }, { id: 'q1', selected: [] }] } });
    expect(screen.getByTestId('ask-answered').textContent).toContain('You answered');
    expect(hasPendingAsk(ctx.session.key)).toBe(false);
    expect(answerPendingAsk(ctx.session.key)).toBe(false);
  });

  it('owner-fix-9: nothing picked, a composer send skips the questions', () => {
    const ctx = fakeCtx([]);
    const cmd = vi.spyOn(ctx.session, 'cmd');
    sessionUi('h1', 's1').applyRequest({ type: 'extension_ui_request', ...FORM });
    render(() => <AskUserCard tool={tool()} ctx={ctx} />);
    answerPendingAsk(ctx.session.key);
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

/** OIN-29: the real result shapes of typed ("Other") answers are answered, never Skipped. */
describe('ask_user: typed answers read as answered', () => {
  const SA_ARGS = { question: 'Which project should I use?\n\n- `oino`\n- `website`' };
  const SA_DIALOG = { id: 'sa-1', method: 'input', extensionId: 'superagent', title: SA_ARGS.question };
  const saTool = (over: Partial<ToolRun> = {}): ToolRun => ({ id: 'sa1', name: 'ask_user', args: SA_ARGS, state: 'running', ...over });

  it("Superagent's result `{\"answer\": text}` (no details) is answered with the typed text", () => {
    const qs = askCallQuestions(SA_ARGS);
    expect(qs).toHaveLength(1);
    expect(askSettled({ state: 'ok', output: '{"answer":"the oino project"}' }, qs)).toEqual({ tone: 'ok', label: 'You answered', text: 'the oino project' });
    expect(askSettled({ state: 'ok', output: '{"answer":""}' }, qs).label).toBe('You skipped the questions');
    expect(askSettled({ state: 'ok', output: '{"cancelled":true}' }, qs).label).toBe('You dismissed the questions');
    // Even without the call's question (an old row): never "answered: skipped".
    expect(askSettled({ state: 'ok', output: '{"answer":"x"}' }, [])).toMatchObject({ label: 'You answered', text: '{"answer":"x"}' });
  });

  it('unipi.ask-user: a typed Other answer in details.answers is answered with the typed text', () => {
    const qs = askQuestions(ARGS.questions).slice(0, 1);
    const details = { outcome: 'answered', questions: ARGS.questions.slice(0, 1), answers: [{ selected: [], custom_text: 'MySQL', skipped: false }] };
    expect(askSettled({ state: 'ok', output: 'User answered your questions:', details }, qs)).toEqual({ tone: 'ok', label: 'You answered', text: 'MySQL' });
    const fromText = askSettled({ state: 'ok', output: 'User answered your questions:\n{\n  "Which DB?": {\n    "selected": [],\n    "custom_text": "MySQL",\n    "skipped": false\n  }\n}' }, qs);
    expect(fromText).toEqual({ tone: 'ok', label: 'You answered', text: 'MySQL' });
  });

  it("a settled Superagent card shows the answer, not Skipped", () => {
    render(() => <AskUserCard tool={saTool({ state: 'ok', output: '{"answer":"the oino project"}' })} ctx={fakeCtx([])} />);
    const row = screen.getByTestId('ask-answered');
    expect(row.textContent).toContain('You answered:the oino project');
    expect(row.textContent).not.toMatch(/skipped/i);
    fireEvent.click(row);
    expect(screen.getByTestId('ask-recap').textContent).toContain('the oino project');
  });

  it("Superagent's input dialog is answered in the card with the typed text", () => {
    const ctx = fakeCtx([]);
    const cmd = vi.spyOn(ctx.session, 'cmd');
    sessionUi('h1', 's1').applyRequest({ type: 'extension_ui_request', ...SA_DIALOG });
    render(() => <AskUserCard tool={saTool()} ctx={ctx} />);
    expect(screen.queryByTestId('ask-wait')).toBeNull();
    fireEvent.input(screen.getByTestId('ask-other'), { target: { value: 'the oino project' } });
    fireEvent.click(screen.getByTestId('ask-submit'));
    expect(cmd).toHaveBeenCalledWith({ type: 'extension_ui_response', id: 'sa-1', value: 'the oino project' });
    expect(screen.getByTestId('ask-answered').textContent).toContain('You answered:the oino project');
  });

  it('a composer send with nothing picked IS the answer (Other of the first question), and is not sent twice', () => {
    const ctx = fakeCtx([]);
    const cmd = vi.spyOn(ctx.session, 'cmd');
    sessionUi('h1', 's1').applyRequest({ type: 'extension_ui_request', ...FORM });
    render(() => <AskUserCard tool={tool()} ctx={ctx} />);
    expect(answerPendingAskWithNote(ctx.session.key, 'Something else entirely')).toBe(true);
    expect(cmd).toHaveBeenCalledWith({ type: 'extension_ui_response', id: 'dlg-1', value: { answers: [{ id: 'q0', selected: [], other: 'Something else entirely' }, { id: 'q1', selected: [] }] } });
    const row = screen.getByTestId('ask-answered');
    expect(row.textContent).toContain('You answered:');
    expect(row.textContent).toContain('Something else entirely');
    expect(row.textContent).not.toContain('You skipped');
  });

  it('a composer send with a pick keeps the pick and the note still goes out as a message', () => {
    const ctx = fakeCtx([]);
    sessionUi('h1', 's1').applyRequest({ type: 'extension_ui_request', ...FORM });
    render(() => <AskUserCard tool={tool()} ctx={ctx} />);
    fireEvent.click(screen.getByText('SQLite'));
    expect(answerPendingAskWithNote(ctx.session.key, 'and fast please')).toBe(false);
    expect(screen.getByTestId('ask-answered').textContent).toContain('SQLite');
  });
});

describe('ask_user: question and option text render as chat markdown', () => {
  it('lists, inline code and line breaks in the question; inline code in an option description', () => {
    const ctx = fakeCtx([]);
    sessionUi('h1', 's1').applyRequest({
      type: 'extension_ui_request',
      id: 'dlg-md',
      method: 'form',
      extensionId: 'unipi.ask-user',
      toolCallId: 'md1',
      questions: [{ id: 'q0', question: 'Pick **one** for `build.rs`:\nsecond line\n\n- first\n- second', header: 'DB', options: [{ label: 'A', value: 'A', description: 'uses `cargo` here' }], other: true }],
    });
    render(() => <AskUserCard tool={{ id: 'md1', name: 'ask_user', args: {}, state: 'running' }} ctx={ctx} />);
    const q = screen.getByTestId('ask-q');
    expect(q.classList.contains('chat-prose')).toBe(true);
    expect(q.querySelector('strong')?.textContent).toBe('one');
    expect(q.querySelector('code')?.textContent).toBe('build.rs');
    expect(q.querySelector('br')).toBeTruthy();
    expect([...q.querySelectorAll('ul > li')].map((li) => li.textContent)).toEqual(['first', 'second']);
    expect(document.querySelector('.ask-opt-desc code')?.textContent).toBe('cargo');
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
