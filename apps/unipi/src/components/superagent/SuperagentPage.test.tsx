// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from '@solidjs/testing-library';
import { Show } from 'solid-js';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { openSuperagentSession, superagentApi } from './api';
import { setHostClientFactory } from '../../lib/host/factory';
import { createMockHost } from '../../dev/mock/host';
import { superagent } from '../../dev/mock/scenarios/superagent';
import { superagentLeads } from '../../dev/mock/scenarios/superagent-leads';
import { createSuperagentState } from './state';
import { SuperagentPane, type PaneTab } from './SuperagentPane';
import { ConfigSchema, StatusSchema } from './api';
import { confirmSheet } from '../phone/Sheet';
import { createSignal } from 'solid-js';
import { buildTurns, messageAuthor } from '../../lib/session/turns';
import { describeEvent } from '../../lib/timeline/eventKinds';
import { api as boardApi } from '../../lib/board/api';
import { applyEvent, initialState } from '../../lib/session/reducer';
import { Root } from '../../main';
import { resetSessions } from '../../lib/session/store';
import { resetHubs } from '../../lib/session/hub';
import { resetLayout, setDesktopOverride } from '../../lib/layout';
import { setPlatformOverride } from '../../lib/platform';
import { openArgs } from '../../lib/host/tauri';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri')), transformCallback: () => 0 }));
vi.mock('../phone/Sheet', () => ({
  confirmSheet: vi.fn(async () => true),
  SheetFrame: (props: { open: boolean; children: unknown; footer: unknown }) => <Show when={props.open}><div>{props.children as never}{props.footer as never}</div></Show>,
}));
afterEach(() => { cleanup(); vi.restoreAllMocks(); setHostClientFactory(undefined); resetSessions(); resetHubs(); resetLayout(); setDesktopOverride(undefined); setPlatformOverride(undefined); });

const lead = (over: object = {}) => ({ projectId: 'heartpick', sessionId: 'lead-hp', presetId: 'bug-batch', model: 'anthropic/claude-sonnet-4-5', thinking: 'high', autonomy: 'high', mode: 'swarm', ticketIds: ['HP-1', 'HP-4'], step: 'Fixing the sync race', tool: 'edit', startedAt: new Date(Date.now() - 18 * 60_000).toISOString(), cost: 0.84, progress: { running: 2, queued: 1, done: 3 }, ...over });
const approval = (n: number, project = 'heartpick', session = 'lead-hp') => ({ projectId: project, taskId: `T-${n}`, status: 'blocked', workerSessionId: session, requestId: `req-${n}`, sessionId: session, tool: 'bash', command: `git push origin branch-${n}`, summary: `Push ${n}`, risk: 'high', ceiling: 'medium', reason: 'needs Autonomy High' });
const raw = (over: object = {}) => ({
  enabled: true, concurrencyCap: 2,
  leads: [lead(), lead({ projectId: 'oino', sessionId: 'lead-oino', presetId: 'feature', mode: 'graph', ticketIds: ['OIN-14'], progress: null })],
  needsYou: [approval(1), approval(2), approval(3, 'oino', 'lead-oino'), { projectId: 'website', taskId: 'WEB-8', status: 'blocked', question: 'Which domain should I use?' }, { projectId: 'billing', taskId: 'BIL-3', status: 'blocked', question: 'EUR or USD?' }],
  review: [{ projectId: 'heartpick', taskId: 'HP-3', status: 'in_review' }, { projectId: 'oino', taskId: 'OIN-12', status: 'in_review' }, { projectId: 'notes', taskId: 'NOT-2', status: 'in_review' }],
  projects: [
    { projectId: 'heartpick', name: 'Heartpick', autoStart: true, lead: 'lead-hp' }, { projectId: 'oino', name: 'Oino', autoStart: false, lead: 'lead-oino' },
    { projectId: 'website', name: 'Website', autoStart: true, lead: null }, { projectId: 'billing', name: 'Billing', autoStart: false, lead: null }, { projectId: 'notes', name: 'Notes', autoStart: false, lead: null },
  ],
  ...over,
});
const presets = [
  { id: 'quick-fix', label: 'Quick fix', description: '', model: null, thinking: null, autonomy: 'high', mode: 'none', maxParallel: null, review: 'none', profiles: null, priority: false },
  { id: 'bug-batch', label: 'Bug batch', description: 'Batch independent bug fixes', model: null, thinking: null, autonomy: 'high', mode: 'swarm', maxParallel: 4, review: 'light', profiles: null, priority: false },
];
function Harness(p: { onOpen: (id: string) => void; onTicket: (project: string, task: string) => void; tab?: PaneTab }) {
  const sa = createSuperagentState(() => 'pc-coffee');
  const [tab, setTab] = createSignal<PaneTab>(p.tab ?? 'needs');
  return <SuperagentPane hostId="pc-coffee" sa={sa} tab={tab()} onTab={setTab} onOpen={p.onOpen} onTicket={p.onTicket} />;
}
function mount(value: object = raw(), opts: { tab?: PaneTab; phone?: boolean } = {}) {
  setHostClientFactory(createMockHost(superagentLeads).client);
  setPlatformOverride(opts.phone ? 'tauri-mobile' : 'tauri-desktop'); setDesktopOverride(!opts.phone);
  vi.spyOn(superagentApi, 'status').mockResolvedValue({ ok: true, value: StatusSchema.parse(value) });
  vi.spyOn(superagentApi.presets, 'list').mockResolvedValue({ ok: true, value: { defaultPresetId: 'quick-fix', presets: presets as never } });
  const onOpen = vi.fn(); const onTicket = vi.fn();
  render(() => <Harness onOpen={onOpen} onTicket={onTicket} tab={opts.tab} />);
  return { onOpen, onTicket };
}
const ok = <T,>(value: T) => ({ ok: true as const, value });
const fail = (message: string, detail?: unknown) => ({ ok: false as const, error: { code: 'E_BOARD_REJECTED' as const, message, retryable: false, detail } });

describe('v3 Superagent leads', () => {
  it('worker_and_needs_you', async () => {
    const { onOpen, onTicket } = mount();
    await screen.findByText('1 of 3');
    expect(document.activeElement === document.body || !screen.getByTestId('sa-needs').contains(document.activeElement)).toBe(true);
    // One approval at a time.
    expect(screen.getAllByTestId('approval-card')).toHaveLength(1);
    expect(screen.getByText('git push origin branch-1')).toBeTruthy();
    // Blocked questions carry their own reply box.
    await screen.findByText('Which domain should I use?');
    expect(screen.getByLabelText('Reply to WEB-8')).toBeTruthy();
    expect(screen.getByLabelText('Reply to BIL-3')).toBeTruthy();
    // Lead cards.
    const cards = screen.getAllByTestId('sa-lead');
    expect(cards).toHaveLength(2);
    expect(cards[0]!.textContent).toContain('Bug batch');
    expect(cards[0]!.textContent).toContain('claude-sonnet-4-5');
    expect(cards[0]!.textContent).toContain('Autonomy: High');
    expect(cards[0]!.textContent).toContain('Fixing the sync race');
    expect(cards[0]!.textContent).toContain('$0.84');
    expect(cards[0]!.textContent).toContain('18 min');
    expect(cards[0]!.textContent).toContain('2 running · 1 queued · 3 done · 2 tickets');
    expect((screen.getByLabelText('Mode for Heartpick') as HTMLSelectElement).value).toBe('swarm');
    expect((screen.getByLabelText('Mode for Oino') as HTMLSelectElement).value).toBe('graph');
    fireEvent.click(within(cards[0]!).getByText('Open')); expect(onOpen).toHaveBeenCalledWith('lead-hp');
    // Review is collapsed with a count, and opens the ticket.
    expect(screen.getByTestId('sa-review').textContent).toContain('3');
    expect(screen.queryByText('HP-3', { exact: false })).toBeNull();
    fireEvent.click(screen.getByText('Review'));
    await screen.findByLabelText('Open HP-3');
    fireEvent.click(screen.getByLabelText('Open HP-3')); expect(onTicket).toHaveBeenCalledWith('heartpick', 'HP-3');
  });
  it('steps through approvals and answers the current one', async () => {
    const respond = vi.spyOn(superagentApi, 'approvalResponse').mockResolvedValue(ok({ accepted: true as const }));
    mount();
    await screen.findByText('1 of 3');
    fireEvent.click(screen.getByLabelText('Next approval'));
    await screen.findByText('2 of 3');
    expect(screen.getByText('git push origin branch-2')).toBeTruthy();
    fireEvent.click(screen.getByText('Allow once'));
    await waitFor(() => expect(respond).toHaveBeenCalledWith('pc-coffee', 'heartpick', 'T-2', 'lead-hp', 'req-2', 'allow_once'));
    await screen.findByText('2 of 2');
    expect(screen.queryByText('git push origin branch-2')).toBeNull();
    expect(screen.getByText('git push origin branch-3')).toBeTruthy();
  });
  it('removes a stale approval and shows a notice', async () => {
    vi.spyOn(superagentApi, 'approvalResponse').mockResolvedValue(fail('stale or wrong-session approval response', { code: 'E_APPROVAL' }));
    mount(raw({ needsYou: [approval(1)] }));
    fireEvent.click(await screen.findByText('Allow once'));
    await screen.findByText('This request is no longer pending');
    expect(screen.queryByTestId('approval-card')).toBeNull();
  });
  it('keeps an approval visible when the host is unavailable', async () => {
    vi.spyOn(superagentApi, 'approvalResponse').mockResolvedValue({ ok: false, error: { code: 'E_TRANSPORT', message: 'Host offline', retryable: true } });
    mount(raw({ needsYou: [approval(1)] }));
    fireEvent.click(await screen.findByText('Allow once'));
    await screen.findByText('Host offline');
    expect(screen.getByTestId('approval-card')).toBeTruthy();
  });
  it('relays a reply to the project lead without taking focus', async () => {
    const relay = vi.spyOn(superagentApi, 'relay').mockResolvedValue(ok({ commentId: null, sessionId: 'lead-hp', delivered: true }));
    mount(raw({ needsYou: [{ projectId: 'website', taskId: 'WEB-8', status: 'blocked', question: 'Which domain should I use?' }] }));
    const box = await screen.findByLabelText('Reply to WEB-8') as HTMLTextAreaElement;
    expect(document.activeElement).not.toBe(box);
    expect((screen.getByLabelText('Send reply for WEB-8') as HTMLButtonElement).disabled).toBe(true);
    fireEvent.input(box, { target: { value: 'Use staging.example.com' } });
    fireEvent.keyDown(box, { key: 'Enter' });
    await waitFor(() => expect(relay).toHaveBeenCalledWith('pc-coffee', { projectId: 'website', text: 'Re WEB-8: Use staging.example.com', delivery: 'steer' }));
    await screen.findByText('Sent to the website lead.');
    await waitFor(() => expect(box.value).toBe(''));
  });
  it('shows a failed relay and keeps the reply text', async () => {
    vi.spyOn(superagentApi, 'relay').mockResolvedValue(ok({ commentId: 'c', sessionId: 'lead-hp', delivered: false, error: { code: 'E_RELAY_DELIVERY', message: 'Lead disconnected' } }));
    mount(raw({ needsYou: [{ projectId: 'website', taskId: 'WEB-8', status: 'blocked', question: 'Which domain?' }] }));
    const box = await screen.findByLabelText('Reply to WEB-8') as HTMLTextAreaElement;
    fireEvent.input(box, { target: { value: 'staging' } });
    fireEvent.click(screen.getByLabelText('Send reply for WEB-8'));
    await waitFor(() => expect(screen.getAllByRole('alert').some((e) => e.textContent?.includes('the lead did not receive it'))).toBe(true));
    expect(box.value).toBe('staging');
  });
  it('asks before a mode switch and only then calls set_mode', async () => {
    const setMode = vi.spyOn(superagentApi, 'setMode').mockResolvedValue(ok({ sessionId: 'lead-hp', mode: 'graph' }));
    mount();
    const select = await screen.findByLabelText('Mode for Heartpick') as HTMLSelectElement;
    vi.mocked(confirmSheet).mockResolvedValueOnce(false);
    fireEvent.change(select, { target: { value: 'graph' } });
    await waitFor(() => expect(confirmSheet).toHaveBeenCalledWith(expect.objectContaining({ title: 'Switch Heartpick to Graph?' })));
    await Promise.resolve();
    expect(setMode).not.toHaveBeenCalled();
    expect(select.value).toBe('swarm');
    fireEvent.change(select, { target: { value: 'graph' } });
    await waitFor(() => expect(setMode).toHaveBeenCalledWith('pc-coffee', 'heartpick', 'graph'));
  });
  it('stops a lead after confirming', async () => {
    const stop = vi.spyOn(superagentApi, 'stop').mockResolvedValue(ok({ stopped: true }));
    mount();
    fireEvent.click(within(await screen.findAllByTestId('sa-lead').then((c) => c[0]!)).getByText('Stop'));
    await waitFor(() => expect(stop).toHaveBeenCalledWith('pc-coffee', 'heartpick', 'HP-1'));
    await screen.findByText('Lead stopped.');
  });
  it('toggles a project auto-start on its own and reverts on failure', async () => {
    const configure = vi.spyOn(superagentApi, 'configureProject').mockResolvedValueOnce(ok({ projectId: 'website', autoStart: false })).mockResolvedValueOnce(fail('Host refused'));
    mount();
    const website = await screen.findByRole('switch', { name: 'Auto-start Website' });
    expect(website.getAttribute('aria-checked') ?? website.getAttribute('data-checked')).toBeTruthy();
    fireEvent.click(website);
    await waitFor(() => expect(configure).toHaveBeenCalledWith('pc-coffee', 'website', false));
    fireEvent.click(screen.getByRole('switch', { name: 'Auto-start Billing' }));
    await waitFor(() => expect(configure).toHaveBeenCalledWith('pc-coffee', 'billing', true));
    await screen.findByText('Host refused');
    await waitFor(() => expect((screen.getByRole('switch', { name: 'Auto-start Billing' }) as HTMLInputElement).checked).toBe(false));
  });
  it('shows ready counts and explains starting a lead when Auto is enabled', async () => {
    vi.spyOn(superagentApi, 'configureProject').mockResolvedValue(ok({ projectId: 'billing', autoStart: true }));
    const value = raw();
    mount({ ...value, projects: value.projects.map((p) => p.projectId === 'billing' ? { ...p, ready: 3 } : p) });
    await screen.findByText('No lead · 3 ready');
    fireEvent.click(screen.getByRole('switch', { name: 'Auto-start Billing' }));
    await screen.findByText('Starting a lead for 3 ready tickets');
  });
  it('explains when global auto-start is disabled', async () => {
    vi.spyOn(superagentApi, 'configureProject').mockResolvedValue(ok({ projectId: 'billing', autoStart: true }));
    mount(raw({ enabled: false }));
    fireEvent.click(await screen.findByRole('switch', { name: 'Auto-start Billing' }));
    await screen.findByText('Auto-start is off globally — turn it on in Superagent settings to start leads');
  });
  it('watcher_and_relay: master switch and cap live in the settings popover', async () => {
    const configure = vi.spyOn(superagentApi, 'configure').mockImplementation(async (_h, c) => ok(c));
    mount();
    await screen.findByTestId('sa-leads');
    fireEvent.click(screen.getByLabelText('Auto-start settings'));
    fireEvent.click(await screen.findByRole('switch', { name: 'Auto-start' }));
    await waitFor(() => expect(configure.mock.calls).toEqual([['pc-coffee', { enabled: false, concurrencyCap: 2 }]]));
    await waitFor(() => expect((screen.getByLabelText('Increase max leads') as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(screen.getByLabelText('Increase max leads'));
    await waitFor(() => expect(configure).toHaveBeenCalledWith('pc-coffee', { enabled: false, concurrencyCap: 3 }));
    await waitFor(() => expect((screen.getByLabelText('Increase max leads') as HTMLButtonElement).disabled).toBe(true));
    expect(ConfigSchema.safeParse({ enabled: true, concurrencyCap: 4 }).success).toBe(false);
    expect(ConfigSchema.safeParse({ enabled: true, concurrencyCap: 0 }).success).toBe(false);
  });
  it('starts ready tickets with the default preset', async () => {
    const start = vi.spyOn(superagentApi, 'start').mockResolvedValue(ok({ sessionId: 'lead-new' }));
    mount();
    fireEvent.click(await screen.findByLabelText('Start Website'));
    await screen.findByText('Pricing table copy');
    expect((screen.getByLabelText('Preset') as HTMLSelectElement).value).toBe('quick-fix');
    fireEvent.change(screen.getByLabelText('Preset'), { target: { value: 'bug-batch' } });
    fireEvent.click(screen.getByLabelText(/WEB-12/));
    fireEvent.click(screen.getByText('Start 2 tickets'));
    await waitFor(() => expect(start).toHaveBeenCalledWith('pc-coffee', 'website', ['WEB-9', 'WEB-11'], 'bug-batch'));
  });
  it('manages presets: default, delete, save', async () => {
    const setDefault = vi.spyOn(superagentApi.presets, 'setDefault').mockResolvedValue(ok({ defaultPresetId: 'bug-batch' }));
    const remove = vi.spyOn(superagentApi.presets, 'remove').mockResolvedValue(ok({ deleted: true }));
    const save = vi.spyOn(superagentApi.presets, 'save').mockImplementation(async (_h, preset) => ok({ preset }));
    mount();
    await screen.findByTestId('sa-leads');
    fireEvent.click(screen.getByLabelText('Auto-start settings'));
    fireEvent.click(await screen.findByText('Manage presets'));
    await screen.findByTestId('sa-preset-manager');
    expect((screen.getByLabelText('Delete Quick fix') as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByLabelText('Make Bug batch the default'));
    await waitFor(() => expect(setDefault).toHaveBeenCalledWith('pc-coffee', 'bug-batch'));
    await waitFor(() => expect((screen.getByLabelText('Delete Bug batch') as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(screen.getByLabelText('Delete Bug batch'));
    await waitFor(() => expect(remove).toHaveBeenCalledWith('pc-coffee', 'bug-batch'));
    await waitFor(() => expect((screen.getByLabelText('Make Bug batch the default') as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(screen.getByText('New preset'));
    fireEvent.click(screen.getByText('Save preset'));
    await screen.findByText('Name the preset.');
    fireEvent.input(screen.getByLabelText('Preset name'), { target: { value: 'Night shift' } });
    fireEvent.input(screen.getByLabelText('Preset parallel work'), { target: { value: '99' } });
    fireEvent.click(screen.getByText('Save preset'));
    await screen.findByText('Parallel work is 1 to 16, or empty.');
    fireEvent.input(screen.getByLabelText('Preset parallel work'), { target: { value: '3' } });
    fireEvent.click(screen.getByText('Save preset'));
    await waitFor(() => expect(save).toHaveBeenCalledWith('pc-coffee', expect.objectContaining({ id: 'night-shift', label: 'Night shift', mode: 'none', autonomy: 'high', maxParallel: 3 })));
  });
  it('collapses long lists behind show more', async () => {
    mount(raw({ needsYou: [] }));
    await screen.findByText('Nothing needs you.');
    expect(screen.getAllByText(/^(Heartpick|Oino|Website|Billing|Notes)$/).length).toBeGreaterThan(0);
    expect(screen.queryByLabelText('Start Notes')).toBeNull();
    fireEvent.click(screen.getByText('Show 1 more'));
    expect(screen.getByLabelText('Start Notes')).toBeTruthy();
  });
  it('empty_and_refresh_failure', async () => {
    mount(raw({ leads: [], needsYou: [], review: [], projects: [] }));
    await screen.findByText('Nothing needs you.');
    expect(screen.getByText(/No lead is running/)).toBeTruthy();
    expect(screen.getByText('No projects on this computer.')).toBeTruthy();
    vi.mocked(superagentApi.status).mockResolvedValue({ ok: false, error: { code: 'E_TRANSPORT', message: 'Host offline', retryable: true } });
    fireEvent.click(screen.getByLabelText('Refresh'));
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('Host offline'));
    expect(screen.getByText('Nothing needs you.')).toBeTruthy();
  });
  it('degrades an older worker-shaped host', async () => {
    const legacy = { enabled: false, concurrencyCap: 2, workers: [{ projectId: 'oino', taskId: 'OIN-14', sessionId: 'worker-14', autonomy: 'medium', step: 'Testing', tool: 'bash', startedAt: null, cost: null }], needsYou: [{ projectId: 'oino', taskId: 'OIN-12', status: 'in_review' }, { projectId: 'website', taskId: 'WEB-8', status: 'blocked', question: 'Which domain?' }] };
    const parsed = StatusSchema.parse(legacy);
    expect(parsed.legacy).toBe(true);
    expect(parsed.review).toEqual([{ projectId: 'oino', taskId: 'OIN-12', status: 'in_review' }]);
    expect(parsed.needsYou).toHaveLength(1);
    const stop = vi.spyOn(superagentApi, 'stop').mockResolvedValue(ok({ stopped: true }));
    const { onOpen } = mount(legacy);
    const card = await screen.findByTestId('sa-lead');
    expect(card.textContent).toContain('Testing');
    expect(card.textContent).toContain('1 ticket');
    expect(screen.queryByLabelText('Mode for oino')).toBeNull();
    expect(screen.getByText(/no per-project controls/)).toBeTruthy();
    fireEvent.click(within(card).getByText('Open')); expect(onOpen).toHaveBeenCalledWith('worker-14');
    fireEvent.click(within(card).getByText('Stop'));
    await waitFor(() => expect(stop).toHaveBeenCalledWith('pc-coffee', 'oino', 'OIN-14'));
  });
  it('phone shows one tab at a time', async () => {
    mount(raw(), { tab: 'needs', phone: true });
    await screen.findByRole('radiogroup', { name: 'Superagent sections' });
    expect(await screen.findByRole('radio', { name: 'Needs you · 5' })).toBeTruthy();
    expect(screen.queryByTestId('sa-leads')).toBeNull();
    fireEvent.click(screen.getByRole('radio', { name: 'Leads · 2' }));
    await screen.findByTestId('sa-leads');
    expect(screen.queryByTestId('sa-needs')).toBeNull();
    fireEvent.click(screen.getByRole('radio', { name: 'Projects' }));
    await screen.findByTestId('sa-projects');
  });
});
describe('v3 Superagent shared', () => {
  it('author_not_steer', () => {
    for (const author of ['host', 'superagent', 'agent']) {
      const entries = [
        { id: 'u', parentId: null, type: 'message', message: { role: 'user', content: 'Start' } },
        { id: 'a', parentId: 'u', type: 'message', message: { role: 'assistant', content: [], stopReason: 'toolUse' } },
        { id: 't', parentId: 'a', type: 'message', message: { role: 'toolResult', content: [] } },
        { id: 'relay', parentId: 't', type: 'message', message: { role: 'user', content: 'Follow up', details: { author } } },
      ];
      const input = { entries, leafId: 'relay', entriesRev: 0, liveTools: {}, run: { running: false } };
      const turns = buildTurns(input);
      expect(turns).toHaveLength(2);
      expect(turns[1]!.source).toBe('harness');
      expect(messageAuthor(turns[1]!.user?.message)).toBe(author);
      expect(turns.flatMap((t) => t.segments).some((s) => !!s.steer)).toBe(false);
      entries[3]!.message = { role: 'user', content: 'Actual user follow-up', details: { author: 'user' } };
      expect(buildTurns(input)).toHaveLength(1);
      expect(buildTurns(input)[0]!.segments[1]!.steer?.id).toBe('relay');
    }
    const state = initialState('h', 's');
    applyEvent(state, { session: 's', seq: 1, event: { type: 'message_end', message: { role: 'custom', customType: 'oino:superagent-relay', display: true, content: [{ type: 'text', text: 'Follow up' }], details: { author: 'superagent', oinoProvenance: { kind: 'superagent', author: 'superagent' } } } } });
    const entry = state.entries[0]!;
    expect(describeEvent(entry)?.source).toBe('superagent');
    expect(buildTurns({ entries: state.entries, leafId: state.leafId, entriesRev: 0, liveTools: {}, run: { running: false } }).flatMap((t) => t.segments).some((s) => s.steer)).toBe(false);
  });
  it('sidebar_route_and_history_keep_restricted_controls_hidden', async () => {
    const world = createMockHost(superagent);
    setHostClientFactory(world.client);
    setPlatformOverride('tauri-desktop'); setDesktopOverride(true);
    location.hash = '#/superagent?session=superagent-chat&host=pc-coffee';
    render(() => <Root />);
    await screen.findByTestId('superagent-page');
    await screen.findByText(/Oino is working on/);
    expect(screen.getByTestId('tl').hasAttribute('data-centered-empty')).toBe(false);
    expect(screen.getByTestId('rail-superagent').getAttribute('aria-current')).toBe('page');
    expect(screen.queryByTestId('permission-chip')).toBeNull();
    expect(screen.queryByTestId('workspace-picker')).toBeNull();
    expect(screen.queryByLabelText(/Plan mode/)).toBeNull();
  });
  it('centers the composer for an empty superagent conversation', async () => {
    const scenario = structuredClone(superagent.hosts[0]!.sessions!['superagent-chat']!.snapshot);
    scenario.entries = []; scenario.leaf_id = null;
    const world = createMockHost({ ...superagent, hosts: [{ ...superagent.hosts[0]!, sessions: { ...superagent.hosts[0]!.sessions, 'superagent-chat': { snapshot: scenario } } }] });
    setHostClientFactory(world.client); setPlatformOverride('tauri-desktop'); setDesktopOverride(true);
    location.hash = '#/superagent?session=superagent-chat&host=pc-coffee';
    render(() => <Root />);
    await screen.findByText('What needs doing?');
    expect(screen.getByTestId('tl').hasAttribute('data-centered-empty')).toBe(true);
    expect(screen.getByTestId('tl-dock').textContent).toContain('What needs doing?');
  });
  it('opens history and starts another restricted session', async () => {
    const world = createMockHost(superagent);
    setHostClientFactory(world.client);
    setPlatformOverride('tauri-desktop'); setDesktopOverride(true);
    location.hash = '#/superagent?session=superagent-chat&host=pc-coffee';
    render(() => <Root />);
    fireEvent.click(await screen.findByLabelText('Superagent history'));
    fireEvent.click(await screen.findByText('Superagent conversation'));
    expect(location.hash).toContain('session=superagent-chat');
    fireEvent.click(screen.getByLabelText('New superagent chat'));
    await waitFor(() => expect(world.log.some((r) => r.kind === 'open' && (r.args as { kind?: string } | undefined)?.kind === 'superagent')).toBe(true));
  });
  it('lists distinct titles in the history dropdown', async () => {
    const world = createMockHost(superagent);
    setHostClientFactory(world.client);
    setPlatformOverride('tauri-desktop'); setDesktopOverride(true);
    location.hash = '#/superagent?session=superagent-chat&host=pc-coffee';
    render(() => <Root />);
    fireEvent.click(await screen.findByLabelText('Superagent history'));
    const list = await screen.findByLabelText('Superagent history list');
    await within(list).findByText('Superagent · 2026-11-08 09:03');
    const titles = [...list.querySelectorAll('button > span')].map((el) => el.textContent ?? '');
    expect(titles.length).toBeGreaterThanOrEqual(4);
    expect(new Set(titles).size).toBe(titles.length);
    expect(titles).toContain('What is running and what needs my attention?');
    expect(titles).toContain('Superagent conversation');
  });
  it('routes superagent attention to the restricted page', async () => {
    const world = createMockHost(superagent); setHostClientFactory(world.client);
    const { openSessionRoute } = await import('../../lib/notifyInit');
    openSessionRoute('pc-coffee', 'superagent-chat');
    await waitFor(() => expect(location.hash).toContain('#/superagent?host=pc-coffee&session=superagent-chat'));
  });
  it('accepts the host wrapped history response with numeric timestamps', async () => {
    const world = createMockHost({ ...superagent, hosts: [{ ...superagent.hosts[0]!, ops: { ...superagent.hosts[0]!.ops, 'superagent.sessions': () => ({ sessions: [{ sessionId: 's', title: 'Old', createdAt: 1, updatedAt: 2, path: '/s' }] }) } }] });
    setHostClientFactory(world.client);
    expect(await superagentApi.sessions('pc-coffee')).toMatchObject({ ok: true, value: [{ sessionId: 's', updatedAt: 2 }] });
  });
  it('restricted_session_and_host_ops_use_the_exact_protocol', async () => {
    const world = createMockHost(superagent);
    setHostClientFactory(world.client);
    expect(await openSuperagentSession('pc-coffee')).toBe('superagent-chat');
    expect(world.log.find((r) => r.kind === 'open')?.args).toEqual({ kind: 'superagent' });
    expect(openArgs('h', { kind: 'superagent' })).toEqual({ command: 'superagent_session', args: { hostId: 'h' } });
    await superagentApi.start('pc-coffee', 'oino', ['OIN-14'], 'feature');
    expect(world.log.some((r) => r.op === 'superagent.start' && JSON.stringify(r.args) === JSON.stringify({ projectId: 'oino', taskIds: ['OIN-14'], presetId: 'feature' }))).toBe(true);
    expect(openArgs('h', { kind: 'superagent' })).toEqual({ command: 'superagent_session', args: { hostId: 'h' } });
  });
});
