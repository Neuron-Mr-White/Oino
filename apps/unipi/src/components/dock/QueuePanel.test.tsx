// @vitest-environment jsdom
/**
 * QueuePanel (WP-A11, Z-53): the host's steering / follow-up queue in the
 * dock — reorder (drag on desktop, arrows on phone), Edit = recall into the
 * composer with the draft guard, Remove, Send now, Clear, and the held
 * queue after a stop with Resume. Every edit goes out as `_oino/queue_edit`
 * / `clear_queue` on the session stream.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { createRoot } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createMockHost, type MockScenario, type MockWorld } from '../../dev/mock/host';
import type { RpcCommand, SessionSnapshot } from '../../lib/host/client';
import { createSession, createSessionRegistry, type SessionHandle } from '../../lib/session/store';
import { clearScrollMemory } from '../../lib/timeline/scrollMemory';
import { composerDraft, setComposerText, clearComposer } from '../../lib/composer/drafts';
import { QueuePanel, applyMove, moveEdit, queueItems } from './QueuePanel';
import roundtrip from '../../lib/session/fixtures/roundtrip.json';

const rt = roundtrip as unknown as { fresh: SessionSnapshot };

let disposers: Array<() => void> = [];
beforeEach(() => {
  localStorage.clear();
  clearScrollMemory();
});
afterEach(() => {
  disposers.forEach((d) => d());
  disposers = [];
  cleanup();
});

const QUEUE = { steering: ['Steer: keep the API unchanged'], followUp: ['Run the linter too', 'Then update the README'] };

function world(): MockWorld {
  const scenario: MockScenario = {
    name: 'queue',
    hosts: [{ id: 'h1', name: 'coffee', sessions: { s1: { snapshot: { ...rt.fresh, seq: 0, entries: [] } as SessionSnapshot, events: [] } } }],
  };
  return createMockHost(scenario);
}

async function open(w: MockWorld, running: boolean): Promise<SessionHandle> {
  const root = createRoot((d) => {
    const s = createSession('h1', 's1', { client: w.client('h1'), schedule: (f) => f(), now: () => Date.now() });
    disposers.push(() => {
      s.close();
      d();
    });
    return s;
  });
  await waitFor(() => expect(root.state.conn).toBe('live'));
  let seq = 0;
  if (running) w.push('h1', 's1', [{ session: 's1', seq: ++seq, event: { type: 'agent_start' } }]);
  w.push('h1', 's1', [{ session: 's1', seq: ++seq, event: { type: 'queue_update', ...QUEUE } }]);
  return root;
}

const cmds = (w: MockWorld, op: string): RpcCommand[] => w.log.filter((e) => e.kind === 'cmd' && e.op === op).map((e) => e.args as RpcCommand);
const edits = (w: MockWorld) => cmds(w, '_oino/queue_edit');

describe('pure queue rules', () => {
  it('flattens steering then follow-up with per-queue indexes', () => {
    expect(queueItems(QUEUE)).toEqual([
      { queue: 'steering', index: 0, text: QUEUE.steering[0] },
      { queue: 'followUp', index: 0, text: QUEUE.followUp[0] },
      { queue: 'followUp', index: 1, text: QUEUE.followUp[1] },
    ]);
  });

  it('moveEdit stays inside one queue and skips no-ops', () => {
    const items = queueItems(QUEUE);
    expect(moveEdit(items[1]!, items[2]!)).toEqual({ op: 'move', queue: 'followUp', from: 0, to: 1 });
    expect(moveEdit(items[1]!, items[1]!)).toBeUndefined();
    expect(moveEdit(items[0]!, items[2]!)).toBeUndefined(); // steering → followUp refused
  });

  it('applyMove mirrors the host (remove at from, insert at to)', () => {
    expect(applyMove(['a', 'b', 'c'], 0, 2)).toEqual(['b', 'c', 'a']);
    expect(applyMove(['a', 'b', 'c'], 2, 0)).toEqual(['c', 'a', 'b']);
    expect(applyMove(['a', 'b'], 0, 5)).toEqual(['a', 'b']);
  });
});

describe('QueuePanel', () => {
  it('renders both queues with kind chips and no Resume while running', async () => {
    const w = world();
    const s = await open(w, true);
    render(() => <QueuePanel session={s} />);
    expect(screen.getByTestId('queue-panel')).toBeTruthy();
    expect(screen.getByText('Steer: keep the API unchanged')).toBeTruthy();
    expect(screen.getByText('Run the linter too')).toBeTruthy();
    expect(screen.getByText('Then update the README')).toBeTruthy();
    expect(screen.queryByTestId('queue-resume')).toBeNull();
  });

  it('Remove and Send now edit one item (_oino/queue_edit)', async () => {
    const w = world();
    const s = await open(w, true);
    render(() => <QueuePanel session={s} />);
    fireEvent.click(screen.getAllByTestId('queue-remove')[1]!);
    fireEvent.click(screen.getAllByTestId('queue-send-now')[2]!);
    await waitFor(() => expect(edits(w)).toHaveLength(2));
    expect(edits(w)[0]).toMatchObject({ type: '_oino/queue_edit', op: 'remove', queue: 'followUp', index: 0 });
    expect(edits(w)[1]).toMatchObject({ type: '_oino/queue_edit', op: 'sendNow', queue: 'followUp', index: 1 });
  });

  it('send escalates: queued → steer, steer → abort + prompt', async () => {
    const w = world();
    const s = await open(w, true);
    render(() => <QueuePanel session={s} />);
    const btn = () => screen.getAllByTestId('queue-send-now');
    expect(btn()[1]!.getAttribute('aria-label')).toMatch(/^Steer now/);
    expect(btn()[0]!.getAttribute('aria-label')).toMatch(/^Send now \(interrupts\)/);
    fireEvent.click(btn()[1]!);
    await waitFor(() => expect(edits(w)[0]).toMatchObject({ op: 'sendNow', queue: 'followUp', index: 0 }));
    fireEvent.click(btn()[0]!);
    await waitFor(() => expect(cmds(w, 'abort')).toHaveLength(1));
    expect(edits(w)[1]).toMatchObject({ op: 'remove', queue: 'steering', index: 0 });
    await waitFor(() => expect(cmds(w, 'prompt')).toHaveLength(1));
  });

  it('Clear sends clear_queue', async () => {
    const w = world();
    const s = await open(w, true);
    render(() => <QueuePanel session={s} />);
    fireEvent.click(screen.getByTestId('queue-clear'));
    await waitFor(() => expect(cmds(w, 'clear_queue')).toHaveLength(1));
  });

  it('desktop rows drag to reorder within a queue', async () => {
    const w = world();
    const s = await open(w, true);
    const { container } = render(() => <QueuePanel session={s} />);
    const rows = container.querySelectorAll('.dock-queue-row');
    const dt = { setData: () => undefined, effectAllowed: '' };
    fireEvent.dragStart(rows[2]!, { dataTransfer: dt });
    fireEvent.dragOver(rows[1]!, { dataTransfer: dt });
    expect(rows[1]!.hasAttribute('data-over')).toBe(true);
    fireEvent.drop(rows[1]!, { dataTransfer: dt });
    await waitFor(() => expect(edits(w)).toHaveLength(1));
    expect(edits(w)[0]).toMatchObject({ op: 'move', queue: 'followUp', from: 1, to: 0 });
  });

  it('phone rows use arrows (no drag)', async () => {
    const w = world();
    const s = await open(w, true);
    const { container } = render(() => <QueuePanel session={s} phone />);
    expect(container.querySelector('.dock-queue-row')!.hasAttribute('draggable')).toBe(false);
    const rows = container.querySelectorAll('.dock-queue-row');
    const up = rows[1]!.querySelectorAll('button[aria-label="Move up"], button[aria-label="Move down"]');
    expect(up).toHaveLength(2);
    fireEvent.click(rows[1]!.querySelector('button[aria-label="Move down"]')!);
    await waitFor(() => expect(edits(w)).toHaveLength(1));
    expect(edits(w)[0]).toMatchObject({ op: 'move', queue: 'followUp', from: 0, to: 1 });
  });

  it('Edit recalls into the composer and removes the item — refused while a draft exists', async () => {
    const w = world();
    const s = await open(w, true);
    render(() => <QueuePanel session={s} />);
    // No draft: recall moves the text into the composer draft store.
    fireEvent.click(screen.getAllByTestId('queue-edit')[0]!);
    await waitFor(() => expect(edits(w)).toHaveLength(1));
    expect(edits(w)[0]).toMatchObject({ op: 'remove', queue: 'steering', index: 0 });
    await waitFor(() => expect(composerDraft(s.key).text).toBe('Steer: keep the API unchanged'));
    // The recalled text is now the draft: another Edit refuses.
    fireEvent.click(screen.getAllByTestId('queue-edit')[0]!);
    await waitFor(() => expect(screen.getByText(/The composer has a draft/)).toBeTruthy());
    expect(edits(w)).toHaveLength(1);
    clearComposer(s.key);
    // Guard lifted again once the composer is free.
    setComposerText(s.key, '');
    fireEvent.click(screen.getAllByTestId('queue-edit')[0]!);
    await waitFor(() => expect(edits(w)).toHaveLength(2));
  });

  it('a stopped run leaves the queue held; Resume delivers the first item and the drain shows', async () => {
    const w = world();
    const s = await open(w, false); // idle: the queue was left behind by a stop
    render(() => <QueuePanel session={s} />);
    expect(screen.getByTestId('queue-panel').hasAttribute('data-held')).toBe(true);
    expect(screen.getByTestId('queue-resume')).toBeTruthy();
    fireEvent.click(screen.getByTestId('queue-resume'));
    await waitFor(() => expect(edits(w)).toHaveLength(1));
    expect(edits(w)[0]).toMatchObject({ op: 'sendNow', queue: 'steering', index: 0 });
    // The run it starts takes the panel out of held, and the host draining
    // the queue empties it (seqs stay contiguous: the store rejects gaps).
    w.push('h1', 's1', [
      { session: 's1', seq: 2, event: { type: 'agent_start' } },
      { session: 's1', seq: 3, event: { type: 'queue_update', steering: [], followUp: QUEUE.followUp } },
    ]);
    await waitFor(() => expect(screen.queryByText('Steer: keep the API unchanged')).toBeNull());
    expect(screen.getByTestId('queue-panel').hasAttribute('data-held')).toBe(false);
  });

  it('renders nothing when the queues are empty', async () => {
    const w = world();
    const s = await createSessionSetup(w);
    render(() => <QueuePanel session={s} />);
    expect(screen.queryByTestId('queue-panel')).toBeNull();
  });

  it('restores a queued steer from host state across session switch + re-attach and holds it on abort (OIN-75)', async () => {
    const q = { steering: [] as string[], followUp: [] as string[] };
    let running = true;
    let seq = 1;
    const snapshotForS1 = (): SessionSnapshot => ({
      ...structuredClone(rt.fresh),
      session_id: 's1',
      seq,
      entries: [],
      state: {
        ...((rt.fresh.state ?? {}) as Record<string, unknown>),
        sessionId: 's1',
        isStreaming: running,
        pendingMessageCount: q.steering.length + q.followUp.length,
        ...(q.steering.length ? { steering: [...q.steering] } : {}),
        ...(q.followUp.length ? { followUp: [...q.followUp] } : {}),
      },
    });
    const sessionS1 = {
      get snapshot() {
        return snapshotForS1();
      },
      events: [] as never[],
      cmd: {
        prompt: (cmd: Record<string, unknown>, host: { hostId: string; world: MockWorld }) => {
          if (cmd.streamingBehavior === 'steer') {
            q.steering.push(String(cmd.message ?? ''));
            host.world.push(host.hostId, 's1', [{ session: 's1', seq: ++seq, event: { type: 'queue_update', steering: [...q.steering], followUp: [...q.followUp] } }]);
            return { disposition: 'queued' };
          }
          return { disposition: 'started' };
        },
        abort: (_cmd: Record<string, unknown>, host: { hostId: string; world: MockWorld }) => {
          running = false;
          host.world.push(host.hostId, 's1', [
            { session: 's1', seq: ++seq, event: { type: 'agent_end', messages: [] } },
            { session: 's1', seq: ++seq, event: { type: 'agent_settled', aborted: true } },
          ]);
          return {};
        },
      },
    };
    const w = createMockHost({
      name: 'steer-reattach',
      hosts: [
        {
          id: 'h1',
          name: 'coffee',
          sessions: {
            s1: sessionS1,
            s2: { snapshot: { ...structuredClone(rt.fresh), session_id: 's2', seq: 0, entries: [] } as SessionSnapshot, events: [] },
          },
        },
      ],
    });
    const registry = createSessionRegistry(1, (hostId, sessionId) =>
      createSession(hostId, sessionId, { client: w.client(hostId), schedule: (f) => f(), now: () => Date.now() }),
    );
    disposers.push(() => registry.closeAll());

    // 1. Open s1 during a running turn and send a steer.
    const first = registry.ensure('h1', 's1');
    const releaseS1 = first.acquire();
    await waitFor(() => expect(first.state.conn).toBe('live'));
    first.send('Hold off on touching public types', { mode: 'steer' });
    await waitFor(() => expect(first.state.queue.steering).toEqual(['Hold off on touching public types']));
    expect(first.state.outbox).toEqual([]);

    // 2. Switch to s2: s1 is released and evicted from the in-memory registry (limit = 1).
    releaseS1();
    const second = registry.ensure('h1', 's2');
    const releaseS2 = second.acquire();
    await waitFor(() => expect(second.state.conn).toBe('live'));
    expect(first.closed).toBe(true);

    // 3. Switch back to s1: re-attaches via a fresh session_open snapshot (no client-side cache, no new queue_update).
    releaseS2();
    const reopened = registry.ensure('h1', 's1');
    const releaseReopened = reopened.acquire();
    disposers.push(releaseReopened);
    expect(reopened).not.toBe(first);
    await waitFor(() => expect(reopened.state.conn).toBe('live'));

    render(() => <QueuePanel session={reopened} />);
    expect(screen.getByTestId('queue-panel')).toBeTruthy();
    expect(screen.getByText('Hold off on touching public types')).toBeTruthy();
    expect(screen.getByTestId('queue-panel').hasAttribute('data-held')).toBe(false);

    // 4. Stop/abort the run: steer is NOT sent into the transcript and remains held in QueuePanel.
    await reopened.abort();
    await waitFor(() => expect(reopened.state.run.running).toBe(false));
    expect(reopened.state.entries).toEqual([]);
    expect(screen.getByTestId('queue-panel').hasAttribute('data-held')).toBe(true);
    expect(screen.getByTestId('queue-resume')).toBeTruthy();
    expect(screen.getByText('Hold off on touching public types')).toBeTruthy();
  });
});

async function createSessionSetup(w: MockWorld): Promise<SessionHandle> {
  const root = createRoot((d) => {
    const s = createSession('h1', 's1', { client: w.client('h1'), schedule: (f) => f(), now: () => Date.now() });
    disposers.push(() => {
      s.close();
      d();
    });
    return s;
  });
  await waitFor(() => expect(root.state.conn).toBe('live'));
  return root;
}
