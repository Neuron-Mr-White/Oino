// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from '@solidjs/testing-library';
import { Show } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setHostClientFactory } from '../../../lib/host/factory';
import { createMockHost } from '../../../dev/mock/host';
import { superagentLeads } from '../../../dev/mock/scenarios/superagent-leads';
import { resetSessions } from '../../../lib/session/store';
import { resetHubs } from '../../../lib/session/hub';
import { resetLayout, setDesktopOverride } from '../../../lib/layout';
import { setPlatformOverride } from '../../../lib/platform';
import { StatusSchema, superagentApi } from '../api';
import { createSuperagentState } from '../state';
import { LeadsSection } from '../Leads';
import { Question } from '../NeedsYou';
import { LeadNotices } from './LeadNotices';
import { answerLeadQuestion } from './question';
import { LeadSettingsSection } from './LeadSettings';
import { noticeKey } from './notices';
import { confirmSheet } from '../../phone/Sheet';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri')), transformCallback: () => 0 }));
vi.mock('../../phone/Sheet', () => ({
  confirmSheet: vi.fn(async () => true),
  SheetFrame: (props: { open: boolean; children: unknown; footer: unknown }) => <Show when={props.open}><div>{props.children as never}{props.footer as never}</div></Show>,
}));
// The real dialogs load a model catalogue from the host; these stand-ins only prove the wiring.
vi.mock('../../model/HostModelPicker', () => ({
  HostModelDialog: (props: { title: string; onPick: (key: string | undefined) => void; onClose: () => void }) => <div role="dialog" aria-label={props.title}><button type="button" onClick={() => { props.onPick('openai/gpt-5'); props.onClose(); }}>Pick GPT-5</button></div>,
}));
vi.mock('../../../ui/settings/ModelPickerField', () => ({
  ModelPickerField: (props: { label: string; value: string; fallback: string; onPick: (key: string | undefined) => void }) => <div><span data-testid="fallback-name">{props.value || props.fallback}</span><button type="button" aria-label={`${props.label}: change`} onClick={() => props.onPick('openai/gpt-5')}>Change</button><button type="button" onClick={() => props.onPick(undefined)}>Use default</button></div>,
}));

beforeEach(() => localStorage.clear());
afterEach(() => { cleanup(); vi.restoreAllMocks(); setHostClientFactory(undefined); resetSessions(); resetHubs(); resetLayout(); setDesktopOverride(undefined); setPlatformOverride(undefined); });

const HOST = 'pc-coffee';
const lead = (over: object = {}) => ({ projectId: 'billing', sessionId: 'lead-bill', presetId: null, model: 'anthropic/claude-opus-4-1', thinking: null, autonomy: 'high', mode: 'none', ticketIds: ['BIL-3'], ...over });
const STALLED = lead({ state: 'stalled', health: { kind: 'model', error: '429: quota exhausted, resets after 79h', attempts: 8, since: Date.now() - 14 * 60_000, nextProbeAt: Date.now() + 31 * 60_000 } });
const base = { enabled: true, projects: [{ projectId: 'billing', name: 'Billing', autoStart: false, lead: 'lead-bill' }] };

function mountState(raw: object) {
  setHostClientFactory(createMockHost(superagentLeads).client);
  setPlatformOverride('tauri-desktop'); setDesktopOverride(true);
  vi.spyOn(superagentApi, 'status').mockResolvedValue({ ok: true, value: StatusSchema.parse({ ...base, ...raw }) });
  vi.spyOn(superagentApi.presets, 'list').mockResolvedValue({ ok: true, value: { defaultPresetId: null, presets: [] } });
  return createSuperagentState(() => HOST);
}
/** The stepper commits on Enter. */
const typeNumber = (input: HTMLElement, value: string) => { fireEvent.input(input, { target: { value } }); fireEvent.keyDown(input, { key: 'Enter' }); };
const ok = <T,>(value: T) => ({ ok: true as const, value });
const refuse = (code: string, message: string) => ({ ok: false as const, error: { code, message, retryable: false } as never });

describe('lead rows', () => {
  it('names each state with the canvas icon and colour hook', async () => {
    const sa = mountState({ leads: [
      lead({ projectId: 'a', sessionId: 'l1' }), lead({ projectId: 'b', sessionId: 'l2', state: 'waiting', question: { taskId: 'B-1', excerpt: 'EUR or USD?' } }),
      lead({ projectId: 'c', sessionId: 'l3', state: 'idle', lastActiveAt: Date.now() - 12 * 60_000 }), lead({ projectId: 'd', sessionId: 'l4', state: 'unloaded' }), STALLED,
    ] });
    render(() => <LeadsSection hostId={HOST} sa={sa} onOpen={() => {}} />);
    await screen.findAllByTestId('sa-lead');
    const states = screen.getAllByTestId('sa-lead-state').map((el) => [el.getAttribute('data-state'), el.textContent]);
    // Stalled first, then waiting; equal states keep the host's order.
    expect(states).toEqual([['stalled', 'Stalled'], ['waiting', 'Waiting for you'], ['working', 'Working'], ['idle', 'Idle'], ['unloaded', 'Unloaded (wakeable)']]);
    expect(screen.getByTestId('sa-lead-ask').textContent).toBe('EUR or USD?');
    expect(screen.getByText(/Last active 12 min ago/)).toBeTruthy();
  });

  it('draws one health card per stalled lead, with error, attempts, since and next re-probe', async () => {
    const sa = mountState({ leads: [STALLED, lead({ projectId: 'oino', sessionId: 'l2' })] });
    render(() => <LeadsSection hostId={HOST} sa={sa} onOpen={() => {}} />);
    const cards = await screen.findAllByTestId('sa-health');
    expect(cards).toHaveLength(1);
    expect(screen.getByTestId('sa-health-error').textContent).toBe('429: quota exhausted, resets after 79h');
    expect(screen.getByTestId('sa-health-meta').textContent).toMatch(/^8 attempts · since 14m ago · next retry in (30|31)m$/);
    expect(within(cards[0]!).getAllByRole('button').map((b) => b.textContent)).toEqual(['Retry', 'Switch model', 'Stop']);
  });

  it('leaves out what the host did not report', async () => {
    const sa = mountState({ leads: [lead({ state: 'stalled', health: { error: 'Could not reopen lead: no such file', attempts: 1 } })] });
    render(() => <LeadsSection hostId={HOST} sa={sa} onOpen={() => {}} />);
    await screen.findByTestId('sa-health');
    expect(screen.getByTestId('sa-health-meta').textContent).toBe('1 attempt');
  });

  it('Retry calls leadAction with the lead session and refreshes', async () => {
    const sa = mountState({ leads: [STALLED] });
    const action = vi.spyOn(superagentApi, 'leadAction').mockResolvedValue(ok({ ok: true as const }));
    render(() => <LeadsSection hostId={HOST} sa={sa} onOpen={() => {}} />);
    fireEvent.click(await screen.findByText('Retry'));
    await waitFor(() => expect(action).toHaveBeenCalledWith(HOST, 'lead-bill', 'retry', undefined));
    expect(confirmSheet).not.toHaveBeenCalled();
    await waitFor(() => expect(superagentApi.status).toHaveBeenCalledTimes(2));
  });

  it('Switch model uses the shared model dialog and sends the picked model', async () => {
    const sa = mountState({ leads: [STALLED] });
    const action = vi.spyOn(superagentApi, 'leadAction').mockResolvedValue(ok({ ok: true as const }));
    render(() => <LeadsSection hostId={HOST} sa={sa} onOpen={() => {}} />);
    fireEvent.click(await screen.findByText('Switch model'));
    fireEvent.click(within(screen.getByRole('dialog', { name: 'Switch model' })).getByText('Pick GPT-5'));
    await waitFor(() => expect(action).toHaveBeenCalledWith(HOST, 'lead-bill', 'switch_model', 'openai/gpt-5'));
  });

  it('Stop asks first and only then calls leadAction', async () => {
    const sa = mountState({ leads: [STALLED] });
    const action = vi.spyOn(superagentApi, 'leadAction').mockResolvedValue(ok({ ok: true as const }));
    vi.mocked(confirmSheet).mockResolvedValueOnce(false);
    render(() => <LeadsSection hostId={HOST} sa={sa} onOpen={() => {}} />);
    const card = await screen.findByTestId('sa-health');
    fireEvent.click(within(card).getByText('Stop'));
    await waitFor(() => expect(confirmSheet).toHaveBeenCalledWith(expect.objectContaining({ title: 'Stop the Billing lead?', tone: 'danger' })));
    expect(action).not.toHaveBeenCalled();
    fireEvent.click(within(card).getByText('Stop'));
    await waitFor(() => expect(action).toHaveBeenCalledWith(HOST, 'lead-bill', 'stop', undefined));
  });

  it.each([
    ['E_LEAD_STATE', 'The lead is working again.', 'The lead is working again.'],
    ['E_LEAD_UNKNOWN', 'No lead lead-bill', 'No lead lead-bill'],
    ['E_UNKNOWN_MODEL', 'Unknown model nope/none', 'Unknown model nope/none'],
    ['E_LEAD_STATE', '', 'The lead is not in a state that allows that. Refresh and try again.'],
    ['E_SCHEMA', 'unknown variant `superagent.lead_action`', "This host can't do that yet (unknown variant `superagent.lead_action`). Update Unipi on the PC."],
    ['E_UNSUPPORTED', '', "This host can't do that yet. Update Unipi on the PC."],
  ])('shows %s inline and never as a success', async (code, message, shown) => {
    const sa = mountState({ leads: [STALLED] });
    vi.spyOn(superagentApi, 'leadAction').mockResolvedValue(refuse(code, message));
    render(() => <LeadsSection hostId={HOST} sa={sa} onOpen={() => {}} />);
    fireEvent.click(await screen.findByText('Retry'));
    expect((await screen.findByTestId('sa-health-action-error')).textContent).toBe(shown);
    expect(sa.notice()).toBe('');
    // The card stays, and the buttons work again.
    expect(screen.getByText('Retry').hasAttribute('disabled')).toBe(false);
  });

  it('offers a quiet Wake on idle and unloaded leads, and no Stop', async () => {
    const sa = mountState({ leads: [lead({ projectId: 'c', sessionId: 'l3', state: 'idle' }), lead({ projectId: 'd', sessionId: 'l4', state: 'unloaded' })] });
    const action = vi.spyOn(superagentApi, 'leadAction').mockResolvedValue(ok({ ok: true as const }));
    render(() => <LeadsSection hostId={HOST} sa={sa} onOpen={() => {}} />);
    const cards = await screen.findAllByTestId('sa-lead');
    expect(cards.flatMap((c) => within(c).queryAllByText('Stop'))).toHaveLength(0);
    fireEvent.click(within(cards[1]!).getByText('Wake'));
    await waitFor(() => expect(action).toHaveBeenCalledWith(HOST, 'l4', 'wake', undefined));
  });

  it('shows a refused Wake inline', async () => {
    const sa = mountState({ leads: [lead({ state: 'idle' })] });
    vi.spyOn(superagentApi, 'leadAction').mockResolvedValue(refuse('E_LEAD_STATE', 'The lead is already working.'));
    render(() => <LeadsSection hostId={HOST} sa={sa} onOpen={() => {}} />);
    fireEvent.click(await screen.findByText('Wake'));
    expect((await screen.findByTestId('sa-wake-error')).textContent).toBe('The lead is already working.');
  });
});

describe('a lead\'s own question', () => {
  const asked = (over: object = {}) => ({ projectId: 'website', taskId: 'WEB-9', status: 'in_progress', kind: 'question', question: 'Which currency first: EUR or USD?', askedAt: Date.now() - 4 * 60_000, expiresAt: Date.now() + 26 * 60_000, ...over });
  const mountQuestion = (item: object) => {
    const sa = mountState({ needsYou: [item] });
    const parsed = StatusSchema.parse({ ...base, needsYou: [item] }).needsYou[0]!;
    render(() => <Question sa={sa} item={parsed} hostId={HOST} />);
    return sa;
  };

  it('shows the excerpt, when it was asked and when it becomes a blocked ticket', async () => {
    mountQuestion(asked());
    expect(await screen.findByTestId('sa-lead-question')).toBeTruthy();
    expect(screen.getByTestId('sa-ask').textContent).toContain('Which currency first: EUR or USD?');
    expect(screen.getByTestId('sa-ask-timing').textContent).toMatch(/^Asked 4 min ago · Becomes a blocked ticket in (25|26) min$/);
    expect(screen.getByText(/Lead is asking/)).toBeTruthy();
  });

  it('says so when the wait has run out', async () => {
    mountQuestion(asked({ expiresAt: Date.now() - 1000 }));
    expect((await screen.findByTestId('sa-ask-timing')).textContent).toBe('Asked 4 min ago · Becoming a blocked ticket now');
  });

  it('answers through relay aimed at the ticket', async () => {
    const sa = mountQuestion(asked());
    const relay = vi.spyOn(superagentApi, 'relay').mockResolvedValue(ok({ delivered: true }));
    const onSent = vi.spyOn(sa, 'refresh');
    fireEvent.input(screen.getByLabelText('Reply to WEB-9'), { target: { value: 'EUR first' } });
    fireEvent.click(screen.getByLabelText('Send reply for WEB-9'));
    await waitFor(() => expect(relay).toHaveBeenCalledWith(HOST, { projectId: 'website', taskIds: ['WEB-9'], text: 'EUR first', delivery: 'steer' }));
    await waitFor(() => expect(onSent).toHaveBeenCalled());
    expect((screen.getByLabelText('Reply to WEB-9') as HTMLTextAreaElement).value).toBe('');
  });

  it('forwards sessionId and requestId on open-question relay so stale answers fail closed', async () => {
    mountQuestion(asked({ sessionId: 'lead-web', requestId: 'req-42' }));
    const relay = vi.spyOn(superagentApi, 'relay').mockResolvedValue(ok({ delivered: true, answered: 'question', sessionId: 'lead-web' }));
    fireEvent.input(screen.getByLabelText('Reply to WEB-9'), { target: { value: 'EUR first' } });
    fireEvent.click(screen.getByLabelText('Send reply for WEB-9'));
    await waitFor(() => expect(relay).toHaveBeenCalledWith(HOST, {
      projectId: 'website',
      taskIds: ['WEB-9'],
      text: 'EUR first',
      delivery: 'steer',
      sessionId: 'lead-web',
      requestId: 'req-42',
    }));
  });

  it('shows E_ALREADY_ANSWERED as a plain sentence and leaves nothing to send', async () => {
    mountQuestion(asked());
    vi.spyOn(superagentApi, 'relay').mockResolvedValueOnce({ ok: false, error: { code: 'E_ALREADY_ANSWERED', message: 'Question already answered', retryable: false } as never });
    fireEvent.input(screen.getByLabelText('Reply to WEB-9'), { target: { value: 'USD' } });
    fireEvent.click(screen.getByLabelText('Send reply for WEB-9'));
    expect((await screen.findByTestId('sa-ask-error')).textContent).toBe('This question was already answered.');
    expect(screen.getByLabelText('Reply to WEB-9').hasAttribute('disabled')).toBe(true);
    expect(screen.getByLabelText('Send reply for WEB-9').hasAttribute('disabled')).toBe(true);
  });

  it('also finds E_ALREADY_ANSWERED when the host carries it in detail.code', async () => {
    vi.spyOn(superagentApi, 'relay').mockResolvedValueOnce({ ok: false, error: { code: 'E_BOARD_REJECTED', message: 'x', retryable: false, detail: { code: 'E_ALREADY_ANSWERED' } } as never });
    expect(await answerLeadQuestion(HOST, { projectId: 'website', taskId: 'WEB-9' }, 'USD')).toEqual({ kind: 'answered', message: 'This question was already answered.' });
  });

  it('keeps another refusal as the host worded it, and the typed answer', async () => {
    mountQuestion(asked());
    vi.spyOn(superagentApi, 'relay').mockResolvedValue({ ok: false, error: { code: 'E_WORKER_NOT_LIVE', message: 'Lead has ended', retryable: false } as never });
    fireEvent.input(screen.getByLabelText('Reply to WEB-9'), { target: { value: 'EUR' } });
    fireEvent.click(screen.getByLabelText('Send reply for WEB-9'));
    expect((await screen.findByTestId('sa-ask-error')).textContent).toBe('Lead has ended');
    expect((screen.getByLabelText('Reply to WEB-9') as HTMLTextAreaElement).value).toBe('EUR');
  });

  it('keeps today\'s blocked question: its own card and the old relay path', async () => {
    const sa = mountState({ needsYou: [{ projectId: 'website', taskId: 'WEB-8', status: 'blocked', question: 'Which domain?' }] });
    const item = StatusSchema.parse({ ...base, needsYou: [{ projectId: 'website', taskId: 'WEB-8', status: 'blocked', question: 'Which domain?' }] }).needsYou[0]!;
    render(() => <Question sa={sa} item={item} hostId={HOST} />);
    expect(screen.getByTestId('sa-question')).toBeTruthy();
    expect(screen.queryByTestId('sa-ask-timing')).toBeNull();
    const relay = vi.spyOn(superagentApi, 'relay').mockResolvedValue(ok({ delivered: true }));
    fireEvent.input(screen.getByLabelText('Reply to WEB-8'), { target: { value: 'staging' } });
    fireEvent.click(screen.getByLabelText('Send reply for WEB-8'));
    await waitFor(() => expect(relay).toHaveBeenCalledWith(HOST, expect.objectContaining({ taskIds: ['WEB-8'], text: 'Re WEB-8: staging' })));
  });

  it('renders quick-pick buttons for the first question options while keeping free text', async () => {
    mountQuestion(asked({ options: [['EUR', 'USD'], ['Monthly', 'Annual']] }));
    const group = await screen.findByTestId('sa-ask-options');
    const buttons = within(group).getAllByRole('button');
    expect(buttons.map((b) => b.textContent)).toEqual(['EUR', 'USD']);
    expect(screen.getByLabelText('Reply to WEB-9')).toBeTruthy();
    const relay = vi.spyOn(superagentApi, 'relay').mockResolvedValue(ok({ delivered: true, answered: 'question' }));
    fireEvent.click(buttons[0]!);
    await waitFor(() => expect(relay).toHaveBeenCalledWith(HOST, { projectId: 'website', taskIds: ['WEB-9'], text: 'EUR', delivery: 'steer' }));
  });

  it('routes a blocked question holding the lead claim through superagent.answer', async () => {
    const claimed = { kind: 'question', projectId: 'billing', taskId: 'BIL-3', status: 'blocked', sessionId: 'lead-bill', question: 'Should totals be EUR or USD?' };
    const onSent = vi.fn();
    const sa = mountState({ needsYou: [claimed] });
    const parsed = StatusSchema.parse({ ...base, needsYou: [claimed] }).needsYou[0]!;
    render(() => <Question sa={sa} item={parsed} hostId={HOST} onSent={onSent} />);
    const answer = vi.spyOn(superagentApi, 'answer').mockResolvedValue(ok({ delivered: true, answered: 'blocked', sessionId: 'lead-bill' }));
    const relay = vi.spyOn(superagentApi, 'relay');
    fireEvent.input(screen.getByLabelText('Reply to BIL-3'), { target: { value: 'EUR' } });
    fireEvent.click(screen.getByLabelText('Send reply for BIL-3'));
    await waitFor(() => expect(answer).toHaveBeenCalledWith(HOST, 'lead-bill', 'BIL-3', 'EUR'));
    expect(relay).not.toHaveBeenCalled();
    await waitFor(() => expect(onSent).toHaveBeenCalled());
  });

  it.each([
    ['E_SCHEMA', 'unknown variant `superagent.answer`', "This host can't do that yet (unknown variant `superagent.answer`). Update Unipi on the PC."],
    ['E_UNSUPPORTED', '', "This host can't do that yet. Update Unipi on the PC."],
  ])('shows %s as an inline error (never a success) when an older host refuses superagent.answer', async (code, message, shown) => {
    const claimed = { kind: 'question', projectId: 'billing', taskId: 'BIL-3', status: 'blocked', sessionId: 'lead-bill', question: 'Should totals be EUR or USD?' };
    const onSent = vi.fn();
    const sa = mountState({ needsYou: [claimed] });
    const parsed = StatusSchema.parse({ ...base, needsYou: [claimed] }).needsYou[0]!;
    render(() => <Question sa={sa} item={parsed} hostId={HOST} onSent={onSent} />);
    vi.spyOn(superagentApi, 'answer').mockResolvedValue(refuse(code, message));
    fireEvent.input(screen.getByLabelText('Reply to BIL-3'), { target: { value: 'EUR' } });
    fireEvent.click(screen.getByLabelText('Send reply for BIL-3'));
    expect((await screen.findByTestId('sa-ask-error')).textContent).toBe(shown);
    expect(sa.notice()).toBe('');
    expect(onSent).not.toHaveBeenCalled();
    expect((screen.getByLabelText('Reply to BIL-3') as HTMLTextAreaElement).value).toBe('EUR');
  });
});

describe('board_created notice', () => {
  const notice = (over: object = {}) => ({ kind: 'board_created', projectId: 'orchard', name: 'Orchard', at: 1700000000000, ...over });

  it('shows one line and dismisses it for good', async () => {
    const sa = mountState({ notices: [notice(), { kind: 'something_new', projectId: 'orchard', at: 1 }] });
    const view = render(() => <LeadNotices sa={sa} />);
    await sa.refresh();
    expect((await screen.findByTestId('sa-notice')).textContent).toContain('Board created for Orchard');
    expect(screen.getAllByTestId('sa-notice')).toHaveLength(1); // unknown kinds are not drawn
    fireEvent.click(screen.getByLabelText('Dismiss: board created for Orchard'));
    expect(screen.queryByTestId('sa-notice')).toBeNull();
    expect(JSON.parse(localStorage.getItem('unipi.superagent.notices.dismissed.v1')!)).toEqual([noticeKey(notice())]);
    // Still gone after a reload of the view…
    view.unmount();
    render(() => <LeadNotices sa={sa} />);
    expect(screen.queryByTestId('sa-notice')).toBeNull();
  });

  it('shows the same project again for a new `at`, and uses the project name when the host sends none', async () => {
    localStorage.setItem('unipi.superagent.notices.dismissed.v1', JSON.stringify([noticeKey(notice())]));
    const sa = mountState({ notices: [notice(), notice({ at: 1700000999000, name: null })] });
    render(() => <LeadNotices sa={sa} />);
    await sa.refresh();
    const lines = await screen.findAllByTestId('sa-notice');
    expect(lines).toHaveLength(1);
    expect(lines[0]!.textContent).toContain('Board created for orchard');
  });
});

describe('Leads settings', () => {
  const mountSettings = async (lead?: object) => {
    const sa = mountState(lead ? { lead } : {});
    let stored: object | undefined = lead;
    vi.mocked(superagentApi.status).mockImplementation(async () => ok(StatusSchema.parse({ ...base, lead: stored })));
    await sa.refresh();
    const configure = vi.spyOn(superagentApi, 'configure').mockImplementation(async (_h, config) => { stored = config.lead; return ok({ enabled: config.enabled, lead: config.lead }) as never; });
    render(() => <LeadSettingsSection hostId={HOST} sa={sa} />);
    return { sa, configure };
  };

  it('shows the four settings with the spec labels, help and defaults', async () => {
    await mountSettings();
    await waitFor(() => expect(screen.getByTestId('sa-lead-settings')).toBeTruthy());
    for (const text of [
      'Lead retries before fallback', 'Lead fallback model', 'Keep idle leads loaded for (minutes)', 'Wait for your answer to a lead question (minutes)',
      'How many times a lead re-sends its own turn after a provider error (429, 5xx, network) before it switches to the fallback model. This is separate from Graph auto-continue.',
      'Used when a lead runs out of retries. Empty means your default model.',
      'Leads idle longer than this are unloaded. New work reopens the same chat.',
      'After this the question becomes a blocked ticket. The lead keeps the ticket.',
    ]) expect(screen.getByText(text)).toBeTruthy();
    expect((screen.getByLabelText('Lead retries before fallback value') as HTMLInputElement).value).toBe('3');
    expect((screen.getByLabelText('Keep idle leads loaded for (minutes) value') as HTMLInputElement).value).toBe('30');
    expect((screen.getByLabelText('Wait for your answer to a lead question (minutes) value') as HTMLInputElement).value).toBe('30');
    expect(screen.getByTestId('fallback-name').textContent).toBe('Your default model');
  });

  it('saves a changed number with the whole lead object', async () => {
    const { configure } = await mountSettings({ retries: 3, warmMinutes: 30 });
    const input = await screen.findByLabelText('Lead retries before fallback value');
    typeNumber(input, '5');
    await waitFor(() => expect(configure).toHaveBeenCalledWith(HOST, { enabled: true, pausedGraphPolicy: 'report', lead: { retries: 5, fallbackModel: null, warmMinutes: 30, questionTimeoutMinutes: 30 } }));
  });

  it('saves the fallback model, and an empty one as null', async () => {
    const { configure } = await mountSettings();
    fireEvent.click(await screen.findByLabelText('Lead fallback model: change'));
    await waitFor(() => expect(configure).toHaveBeenLastCalledWith(HOST, expect.objectContaining({ lead: expect.objectContaining({ fallbackModel: 'openai/gpt-5' }) })));
    await waitFor(() => expect(screen.getByTestId('fallback-name').textContent).toBe('openai/gpt-5'));
    await waitFor(() => expect(screen.getByLabelText('Lead retries before fallback value').hasAttribute('disabled')).toBe(false)); // the save finished
    fireEvent.click(screen.getByText('Use default'));
    await waitFor(() => expect(configure).toHaveBeenLastCalledWith(HOST, expect.objectContaining({ lead: expect.objectContaining({ fallbackModel: null }) })));
  });

  it('keeps a host refusal on its row and restores the old value', async () => {
    const { configure } = await mountSettings();
    configure.mockResolvedValueOnce(refuse('E_SCHEMA', 'warmMinutes must be 5 to 1440'));
    const input = await screen.findByLabelText('Keep idle leads loaded for (minutes) value');
    typeNumber(input, '60');
    expect((await screen.findByTestId('field-error')).textContent).toBe('warmMinutes must be 5 to 1440');
    await waitFor(() => expect((screen.getByLabelText('Keep idle leads loaded for (minutes) value') as HTMLInputElement).value).toBe('30'));
  });
});
