// @vitest-environment jsdom
import { createRoot, createSignal } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createDraftSession } from './draftSession';
import { setHostClientFactory } from '../../lib/host/factory';
import { createMockHost, type MockHostSpec } from '../../dev/mock/host';
import { allCommands, isKnownCommand } from '../../lib/chat/commands';

const rows = ['swarm', 'graph', 'goal', 'ralph'].map((name) => ({ name, source: 'extension', description: `Start ${name}` }));
let dispose: (() => void) | undefined;
beforeEach(() => localStorage.clear());
afterEach(() => { dispose?.(); setHostClientFactory(undefined); });
function setup(intercept?: (cwd?: string) => Promise<unknown>) {
  const host: MockHostSpec = { id: 'fresh', name: 'Fresh PC', cards: [], commands: rows };
  const world = createMockHost({ name: 'fresh', hosts: [host] });
  const requests: Array<string | undefined> = [];
  const wrapped = new Set<string>();
  setHostClientFactory((id) => {
    const client = world.client(id);
    if (wrapped.has(id)) return client;
    wrapped.add(id);
    const request = client.request.bind(client);
    client.request = async <T>(op: string, args?: Record<string, unknown>): Promise<T> => {
      const cmd = args?.cmd as { kind?: string; cwd?: string } | undefined;
      if (op === 'host_cmd' && cmd?.kind === 'get_commands') {
        requests.push(cmd.cwd);
        if (intercept) return await intercept(cmd.cwd) as T;
      }
      return request<T>(op, args);
    };
    return client;
  });
  const submit = vi.fn();
  const result = createRoot((d) => {
    dispose = d;
    const [cwd, setCwd] = createSignal<string | undefined>('/project/first');
    return { session: createDraftSession('fresh', submit, cwd), setCwd };
  });
  return { ...result, requests, submit };
}
describe('first-chat command discovery', () => {
  it('lists every extension command without a peer or cache and submits the first swarm', async () => {
    const { session, requests, submit } = setup();
    await vi.waitFor(() => expect(session.commandsLoading()).toBe(false));
    expect(requests).toEqual(['/project/first']);
    expect(allCommands(session.state.commands).map((r) => r.name)).toEqual(expect.arrayContaining(rows.map((r) => r.name)));
    for (const row of rows) expect(isKnownCommand(row.name, session.state.commands)).toBe(true);
    session.send('/swarm first task');
    expect(submit).toHaveBeenCalledWith('/swarm first task', []);
  });
  it('waits beyond the toolbar grace and discards a late reply from a previous folder', async () => {
    const replies = new Map<string | undefined, (value: unknown) => void>();
    const { session, setCwd } = setup((cwd) => new Promise((resolve) => replies.set(cwd, resolve)));
    await vi.waitFor(() => expect(replies.has('/project/first')).toBe(true));
    await new Promise((resolve) => setTimeout(resolve, 1550));
    expect(session.commandsLoading()).toBe(true);
    setCwd('/project/second');
    await vi.waitFor(() => expect(replies.has('/project/second')).toBe(true));
    replies.get('/project/second')!({ commands: [{ name: 'second', source: 'prompt' }] });
    await vi.waitFor(() => expect(session.commandsLoading()).toBe(false));
    replies.get('/project/first')!({ commands: rows });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(session.state.commands.map((r) => r.name)).toEqual(['second']);
  });
  it('reports a discovery failure without pretending the catalogue is complete', async () => {
    const { session } = setup(async () => { throw new Error('offline'); });
    await vi.waitFor(() => expect(session.commandsError()).toContain('could not be loaded'));
    expect(session.state.commands).toEqual([]);
  });
  it('replaces cached commands instead of admitting disabled or foreign-project commands', async () => {
    localStorage.setItem('unipi.draft.commands.fresh', JSON.stringify([{ name: 'stale' }]));
    const { session } = setup();
    await vi.waitFor(() => expect(session.commandsLoading()).toBe(false));
    expect(isKnownCommand('stale', session.state.commands)).toBe(false);
  });
});
