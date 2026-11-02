// @vitest-environment jsdom
/**
 * Permission & plan control (WP-A13 acceptance, G-6): the chip shows the
 * workflow extension's live `WORKFLOW_STATUS` (`permissionMode`) and its menu
 * sends `/permission <mode>`; the Plan toggle reflects `planMode` and sends
 * `/plan on|off`. Commands go to the session stream (the MockHost log), never
 * into the draft. Without the extension's commands the control is absent.
 */
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { createRoot, createSignal } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createMockHost, type MockScenario, type MockWorld } from '../../dev/mock/host';
import { genSession } from '../../dev/mock/gen';
import { setHostClientFactory } from '../../lib/host/factory';
import { createSession, resetSessions, sessions, type SessionHandle } from '../../lib/session/store';
import { createDraftSession } from '../draft/draftSession';
import { toasts } from '../../lib/toasts';
import { PermissionControl, PlanToggle } from './PermissionControl';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));

const HOST = 'pc-one';
const CWD = '/home/you/projects/unipi';
const SESSION = 's-perm';
const WORKFLOW_STATUS = 'unipi:workflow:status';

const COMMANDS = [
  { name: 'permission', description: 'Permission mode', source: 'extension' },
  { name: 'plan', description: 'Plan mode', source: 'extension' },
  { name: 'compact', description: 'Compact', source: 'builtin' },
];

function scenario(opts: { commands?: typeof COMMANDS; status?: unknown } = {}): MockScenario {
  const session = genSession(SESSION, { seed: 5, turns: 1, toolsPerTurn: 0, cwd: CWD });
  session.snapshot.commands = (opts.commands ?? COMMANDS) as never;
  session.snapshot.topics = (opts.status ? [{ topic: WORKFLOW_STATUS, data: opts.status, sticky: true, publisher: 'unipi.workflow' }] : []) as never;
  return { name: 'permission-test', hosts: [{ id: HOST, name: 'coffee', conn: { state: 'connected' }, sessions: { [SESSION]: session } }] };
}

let world: MockWorld;
let disposers: Array<() => void> = [];

async function open(opts: Parameters<typeof scenario>[0] = {}): Promise<SessionHandle> {
  world = createMockHost(scenario(opts));
  setHostClientFactory(world.client);
  const handle = createRoot((dispose) => {
    disposers.push(dispose);
    return createSession(HOST, SESSION, { client: world.client(HOST) });
  });
  await waitFor(() => expect(handle.state.conn).toBe('live'));
  await new Promise((r) => setTimeout(r, 20));
  return handle;
}

const prompts = () => world.log.filter((e) => e.kind === 'cmd' && e.op === 'prompt').map((e) => (e.args as { message?: string }).message);

beforeEach(() => {
  localStorage.clear();
  setHostClientFactory(undefined);
});

afterEach(() => {
  cleanup();
  resetSessions();
  for (const dispose of disposers) dispose();
  disposers = [];
  setHostClientFactory(undefined);
  localStorage.clear();
});

describe('permission chip (G-6)', () => {
  it('shows the mode from the workflow topic', async () => {
    const s = await open({ status: { planMode: false, permissionMode: 'full' } });
    const view = render(() => <PermissionControl session={s} onCommand={() => {}} />);
    await waitFor(() => expect(view.getByTestId('permission-label').textContent).toBe('Full'));
    expect(view.getByTestId('permission-chip').getAttribute('data-mode')).toBe('full');
  });

  it('falls back to the extension default (Auto) before the topic arrives', async () => {
    const s = await open();
    const view = render(() => <PermissionControl session={s} onCommand={() => {}} />);
    await waitFor(() => expect(view.queryByTestId('permission-label')).not.toBeNull());
    expect(view.getByTestId('permission-label').textContent).toBe('Auto');
  });

  it('the menu always sends /permission <mode> for any picked mode (G-01)', async () => {
    const s = await open({ status: { planMode: false, permissionMode: 'auto' } });
    const view = render(() => <PermissionControl session={s} onCommand={(cmd) => s.send(cmd, { mode: 'prompt' })} />);
    fireEvent.click(view.getByTestId('permission-chip'));
    const rows = [...document.querySelectorAll('.perm-menu-row')] as HTMLElement[];
    expect(rows.map((r) => r.textContent)).toEqual([expect.stringContaining('Ask'), expect.stringContaining('Auto'), expect.stringContaining('Full'), expect.stringContaining('Make default for new chats')]);
    expect(document.querySelector('[data-testid=permission-scope]')?.textContent).toBe('Applies to this chat only.');
    // G-01: picking Auto explicitly still sends `/permission auto` so a desynced chat can always switch.
    fireEvent.click(rows[1]!);
    await waitFor(() => expect(prompts()).toEqual(['/permission auto']));
    fireEvent.click(view.getByTestId('permission-chip'));
    fireEvent.click([...document.querySelectorAll('.perm-menu-row')][2] as HTMLElement);
    await waitFor(() => expect(prompts()).toEqual(['/permission auto', '/permission full']));
    // F-07: the chip follows the pick at once, before the topic confirms.
    expect(view.getByTestId('permission-label').textContent).toBe('Full');
  });

  it('a session without the extension offers no control', async () => {
    const s = await open({ commands: [{ name: 'compact', description: 'Compact', source: 'builtin' }] });
    const view = render(() => (
      <>
        <PermissionControl session={s} onCommand={() => {}} />
        <PlanToggle session={s} onCommand={() => {}} />
      </>
    ));
    await new Promise((r) => setTimeout(r, 20));
    expect(view.queryByTestId('permission-chip')).toBeNull();
    expect(view.queryByTestId('plan-toggle')).toBeNull();
  });
});

describe('plan toggle (G-6)', () => {
  it('reflects planMode and sends /plan off when it is on', async () => {
    const s = await open({ status: { planMode: true, permissionMode: 'auto' } });
    const sent: string[] = [];
    const view = render(() => <PlanToggle session={s} onCommand={(cmd) => (sent.push(cmd), s.send(cmd, { mode: 'prompt' }))} />);
    const button = () => view.getByTestId('plan-toggle');
    await waitFor(() => expect(button().getAttribute('aria-pressed')).toBe('true'));
    fireEvent.click(button());
    await waitFor(() => expect(prompts()).toEqual(['/plan off']));
    await waitFor(() => expect(s.state.topics[WORKFLOW_STATUS]?.data).toBeDefined());
  });

  it('sends /plan on when plan mode is off', async () => {
    const s = await open({ status: { planMode: false, permissionMode: 'ask' } });
    const view = render(() => <PlanToggle session={s} onCommand={(cmd) => s.send(cmd, { mode: 'prompt' })} />);
    await waitFor(() => expect(view.getByTestId('plan-toggle').getAttribute('aria-pressed')).toBe('false'));
    fireEvent.click(view.getByTestId('plan-toggle'));
    await waitFor(() => expect(prompts()).toEqual(['/plan on']));
  });

  it('follows a live WORKFLOW_STATUS topic event', async () => {
    const s = await open({ status: { planMode: false, permissionMode: 'ask' } });
    const view = render(() => <PlanToggle session={s} onCommand={() => {}} />);
    await waitFor(() => expect(view.getByTestId('plan-toggle').getAttribute('aria-pressed')).toBe('false'));
    world.push(HOST, SESSION, [
      { session: SESSION, seq: 9_001, event: { type: 'topic', topic: WORKFLOW_STATUS, data: { planMode: true, permissionMode: 'auto' }, sticky: true, publisher: 'unipi.workflow' } } as never,
    ]);
    await waitFor(() => expect(view.getByTestId('plan-toggle').getAttribute('aria-pressed')).toBe('true'));
  });

  it('toggles on then off explicitly on consecutive clicks without leaking across sessions (G-01/G-03/G-09)', async () => {
    const sA = genSession('s-a', { seed: 1, turns: 1, toolsPerTurn: 0, cwd: CWD });
    sA.snapshot.commands = COMMANDS as never;
    sA.snapshot.topics = [{ topic: WORKFLOW_STATUS, data: { planMode: false, permissionMode: 'ask' }, sticky: true, publisher: 'unipi.workflow' }] as never;
    const sB = genSession('s-b', { seed: 2, turns: 1, toolsPerTurn: 0, cwd: CWD });
    sB.snapshot.commands = COMMANDS as never;
    sB.snapshot.topics = [{ topic: WORKFLOW_STATUS, data: { planMode: false, permissionMode: 'auto' }, sticky: true, publisher: 'unipi.workflow' }] as never;
    world = createMockHost({ name: 'multi', hosts: [{ id: HOST, name: 'coffee', conn: { state: 'connected' }, sessions: { 's-a': sA, 's-b': sB } }] });
    setHostClientFactory(world.client);
    const a = sessions().ensure(HOST, 's-a');
    const b = sessions().ensure(HOST, 's-b');
    await waitFor(() => {
      expect(a.state.conn).toBe('live');
      expect(b.state.conn).toBe('live');
    });

    const [cur, setCur] = createSignal<SessionHandle>(b);
    const sent: string[] = [];
    const view = render(() => (
      <>
        <PermissionControl session={cur()} onCommand={(c) => sent.push(`${cur().key}:${c}`)} />
        <PlanToggle session={cur()} onCommand={(c) => sent.push(`${cur().key}:${c}`)} />
      </>
    ));
    // Pick Full in B, then immediately switch to A -> chip must show A's real Ask, never B's Full.
    fireEvent.click(view.getByTestId('permission-chip'));
    fireEvent.click(document.querySelector('[data-testid=permission-option-full]')!);
    expect(view.getByTestId('permission-label').textContent).toBe('Full');
    setCur(a);
    await waitFor(() => expect(view.getByTestId('permission-label').textContent).toBe('Ask'));

    // Click Plan twice on A: sends /plan on then /plan off.
    fireEvent.click(view.getByTestId('plan-toggle'));
    expect(view.getByTestId('plan-toggle').getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(view.getByTestId('plan-toggle'));
    expect(view.getByTestId('plan-toggle').getAttribute('aria-pressed')).toBe('false');
    expect(sent).toEqual(['pc-one/s-b:/permission full', 'pc-one/s-a:/plan on', 'pc-one/s-a:/plan off']);

    // G-09: Make default for new chats works on the New-chat DraftSession too.
    toasts().clear();
    const draft = createRoot((d) => (disposers.push(d), createDraftSession(HOST, () => {})));
    await waitFor(() => expect(draft.state.commands.length).toBeGreaterThan(0));
    draft.send('/permission full', { mode: 'prompt' });
    setCur(draft);
    await waitFor(() => expect(view.getByTestId('permission-label').textContent).toBe('Full'));
    fireEvent.click(view.getByTestId('permission-chip'));
    fireEvent.click(document.querySelector('[data-testid=permission-make-default]')!);
    await waitFor(() => expect(toasts().list().some((t) => t.message === 'New chats start in Full')).toBe(true));
  });
});
