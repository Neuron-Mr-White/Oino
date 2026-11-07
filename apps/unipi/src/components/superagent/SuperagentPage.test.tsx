// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { Show } from 'solid-js';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { WorkersPane } from './SuperagentPage';
import { openSuperagentSession, superagentApi, type SuperagentStatus } from './api';
import { setHostClientFactory } from '../../lib/host/factory';
import { createMockHost } from '../../dev/mock/host';
import { superagent } from '../../dev/mock/scenarios/superagent';
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
const status: SuperagentStatus = { enabled: false, concurrencyCap: 2, workers: [{ projectId: 'oino', taskId: 'OIN-14', sessionId: 'worker-14', autonomy: 'high', step: 'Testing', tool: 'bash', startedAt: new Date().toISOString(), cost: 1.25 }], needsYou: [{ projectId: 'oino', taskId: 'OIN-12', status: 'in_review' }, { projectId: 'website', taskId: 'WEB-8', status: 'blocked', question: 'Which domain should I use?' }] };
function mount(value = status) {
  setHostClientFactory(createMockHost(superagent).client);
  vi.spyOn(superagentApi, 'status').mockResolvedValue({ ok: true, value });
  vi.spyOn(boardApi, 'task').mockImplementation(async (_host, _project, id) => ({ id, title: `Title ${id}`, autonomy: id === 'WEB-8' ? 'low' : 'medium' }) as never);
  const onOpen = vi.fn(); const onTicket = vi.fn();
  render(() => <WorkersPane hostId="pc-coffee" onOpen={onOpen} onTicket={onTicket} />);
  return { onOpen, onTicket };
}
describe('v3 Superagent', () => {
  it('worker_and_needs_you', async () => {
    const { onOpen, onTicket } = mount();
    await screen.findByText('Which domain should I use?');
    expect(screen.getByText('Testing')).toBeTruthy();
    expect(screen.getByText('bash')).toBeTruthy();
    expect(screen.getByText('$1.25')).toBeTruthy();
    expect(screen.getByText('Autonomy: High')).toBeTruthy();
    await screen.findByText('Autonomy: Low');
    fireEvent.click(screen.getByText('Open')); expect(onOpen).toHaveBeenCalledWith('worker-14');
    fireEvent.click(screen.getByText('Which domain should I use?')); expect(onTicket).toHaveBeenCalledWith('website', 'WEB-8');
    fireEvent.click(screen.getByText('OIN-12 · Title OIN-12')); expect(onTicket).toHaveBeenCalledWith('oino', 'OIN-12');
  });
  it('accepts parked approval status and opens the worker', async () => {
    const parked = { ...status, needsYou: [{ projectId: 'oino', taskId: 'OIN-14', status: 'blocked' as const, workerSessionId: 'worker-14', requestId: 'req-1', sessionId: 'worker-14', tool: 'bash', command: 'git push', summary: 'Push branch', risk: 'high' as const, ceiling: 'medium' as const, reason: 'needs Autonomy High' }] };
    const { StatusSchema } = await import('./api');
    expect(StatusSchema.parse(parked).needsYou[0]?.requestId).toBe('req-1');
    const respond = vi.spyOn(superagentApi, 'approvalResponse').mockResolvedValue({ ok: true, value: { accepted: true } });
    const { onOpen } = mount(parked);
    fireEvent.click(await screen.findByText('Open worker'));
    expect(onOpen).toHaveBeenCalledWith('worker-14');
    fireEvent.click(screen.getByText('Allow once'));
    await waitFor(() => expect(respond).toHaveBeenCalledWith('pc-coffee', 'oino', 'OIN-14', 'worker-14', 'req-1', 'allow_once'));
  });
  it('removes a stale approval and shows an inline notice', async () => {
    const parked = { ...status, needsYou: [{ projectId: 'oino', taskId: 'OIN-14', status: 'blocked' as const, workerSessionId: 'worker-14', requestId: 'req-stale', sessionId: 'worker-14', tool: 'bash', ceiling: 'medium' as const, reason: 'needs Autonomy High' }] };
    vi.spyOn(superagentApi, 'approvalResponse').mockResolvedValue({ ok: false, error: { code: 'E_BOARD_REJECTED', message: 'Already answered', retryable: false } });
    mount(parked);
    fireEvent.click(await screen.findByText('Allow once'));
    await screen.findByText('This request is no longer pending');
    expect(screen.queryByTestId('approval-card')).toBeNull();
  });
  it('watcher_and_relay', async () => {
    const configure = vi.spyOn(superagentApi, 'configure').mockResolvedValue({ ok: true, value: { enabled: true, concurrencyCap: 2 } });
    const relay = vi.spyOn(superagentApi, 'relay').mockResolvedValue({ ok: true, value: { commentId: 'c', sessionId: 'worker-14', delivered: true } });
    mount();
    const watcher = await screen.findByRole('switch', { name: 'Work watcher' });
    fireEvent.click(watcher);
    await waitFor(() => expect(configure).toHaveBeenCalledWith('pc-coffee', { enabled: true, concurrencyCap: 2 }));
    fireEvent.input(screen.getByLabelText('Concurrency cap'), { target: { value: '0' } });
    fireEvent.click(screen.getByText('Save'));
    await screen.findByText('Concurrency cap must be a whole number of at least 1.');
    expect(configure).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByText('Relay'));
    fireEvent.input(screen.getByLabelText('Message to worker'), { target: { value: 'Keep the public API unchanged.' } });
    fireEvent.click(screen.getByText('Send relay'));
    await waitFor(() => expect(relay).toHaveBeenCalledWith('pc-coffee', 'oino', 'OIN-14', 'Keep the public API unchanged.'));
    await screen.findByText('Message delivered to OIN-14 and saved as a ticket comment.');
  });
  it('relay_partial_failure_and_stop_refusal_are_visible', async () => {
    vi.spyOn(superagentApi, 'relay').mockResolvedValue({ ok: true, value: { commentId: 'saved', sessionId: 'worker-14', delivered: false, error: { code: 'E_RELAY_DELIVERY', message: 'Worker disconnected' } } });
    const stop = vi.spyOn(superagentApi, 'stop').mockResolvedValue({ ok: false, error: { code: 'E_BOARD_REJECTED', message: 'Claim changed', retryable: false } });
    mount();
    fireEvent.click(await screen.findByText('Stop'));
    await waitFor(() => expect(stop).toHaveBeenCalledWith('pc-coffee', 'oino', 'OIN-14'));
    await screen.findByText('Claim changed');
    fireEvent.click(screen.getByText('Relay'));
    fireEvent.input(screen.getByLabelText('Message to worker'), { target: { value: 'Check the logs.' } });
    fireEvent.click(screen.getByText('Send relay'));
    await waitFor(() => expect(screen.getAllByRole('alert').some((e) => e.textContent?.includes('Comment saved, but the worker did not receive it'))).toBe(true));
    expect(screen.queryByText('Message delivered to OIN-14 and saved as a ticket comment.')).toBeNull();
  });
  it('empty_unknown_telemetry_and_refresh_failure', async () => {
    mount({ ...status, workers: [{ ...status.workers[0]!, step: null, tool: null, cost: null, startedAt: null }], needsYou: [] });
    await screen.findByText('Nothing waiting for you.');
    expect(screen.getAllByText('Not reported')).toHaveLength(4);
    await screen.findByText('OIN-14 · Title OIN-14');
    await Promise.resolve();
    vi.mocked(superagentApi.status).mockResolvedValue({ ok: false, error: { code: 'E_TRANSPORT', message: 'Host offline', retryable: true } });
    fireEvent.click(screen.getByLabelText('Refresh workers'));
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('Host offline'));
    expect(screen.getByText('Open')).toBeTruthy();
  });
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
  it('routes superagent attention to the restricted page', async () => {
    const world = createMockHost(superagent); setHostClientFactory(world.client);
    const { openSessionRoute } = await import('../../lib/notifyInit');
    openSessionRoute('pc-coffee', 'superagent-chat');
    await waitFor(() => expect(location.hash).toContain('#/superagent?host=pc-coffee&session=superagent-chat'));
  });
  it('restricted_session_and_host_ops_use_the_exact_protocol', async () => {
    const world = createMockHost(superagent);
    setHostClientFactory(world.client);
    expect(await openSuperagentSession('pc-coffee')).toBe('superagent-chat');
    expect(world.log.find((r) => r.kind === 'open')?.args).toEqual({ kind: 'superagent' });
    expect(openArgs('h', { kind: 'superagent' })).toEqual({ command: 'superagent_session', args: { hostId: 'h' } });
    await superagentApi.start('pc-coffee', 'oino', 'OIN-14');
    expect(world.log.some((r) => r.op === 'superagent.start' && JSON.stringify(r.args) === JSON.stringify({ projectId: 'oino', taskId: 'OIN-14' }))).toBe(true);
  });
});
