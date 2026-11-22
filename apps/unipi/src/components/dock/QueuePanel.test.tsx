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
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createMockHost, type MockScenario, type MockWorld } from '../../dev/mock/host';
import type { RpcCommand, SessionSnapshot } from '../../lib/host/client';
import { createSession, createSessionRegistry, type SessionHandle } from '../../lib/session/store';
import { clearScrollMemory } from '../../lib/timeline/scrollMemory';
import { composerDraft, setComposerText, clearComposer } from '../../lib/composer/drafts';
import { CONFIRM_MS, QueuePanel, applyMove, moveEdit, queueItems, queueTextParts } from './QueuePanel';
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
  it('shows restore failures with Send again and sends the retained text', async () => {
    const w = world();
    const s = await open(w, false);
    w.push('h1', 's1', [{ session: 's1', seq: 2, event: { kind: 'queue_restore_failed', error: 'disk failure', items: [{ commandId: 'b', command: { type: 'follow_up', message: 'recover this' } }] } }]);
    render(() => <QueuePanel session={s} />);
    await waitFor(() => expect(screen.getByTestId('queue-restore-failed').textContent).toContain('recover this'));
    expect(JSON.parse(localStorage.getItem('unipi.session.outbox.h1.s1') ?? '[]')).toEqual([expect.objectContaining({ text: 'recover this', status: 'lost' })]);
    fireEvent.click(screen.getByRole('button', { name: 'Send again' }));
    await waitFor(() => expect(cmds(w, 'prompt').some((c) => c.message === 'recover this')).toBe(true));
  });

  it('shows a notice even when no text can be recovered', async () => {
    const w = world();
    const s = await open(w, false);
    w.push('h1', 's1', [{ session: 's1', seq: 2, event: { kind: 'queue_restore_failed', error: 'Unreadable queue; new messages can still be sent', items: [] } }]);
    render(() => <QueuePanel session={s} />);
    expect(screen.getByRole('alert').textContent).toContain('new messages can still be sent');
  });

  it('uses durable identities for edits when supplied by the host', async () => {
    const w = world();
    const s = await open(w, false);
    w.push('h1', 's1', [{ session: 's1', seq: 2, event: { type: 'queue_update', steering: [], followUp: ['durable'], followUpIds: ['019id'] } }]);
    render(() => <QueuePanel session={s} />);
    fireEvent.click(screen.getByRole('button', { name: /Remove/ }));
    await waitFor(() => expect(edits(w)[0]).toMatchObject({ op: 'byId', messageId: '019id', edit: { op: 'remove', queue: 'followUp', index: 0 } }));
  });

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

  it('rows say when the message goes out, in plain words, and the buttons are named', async () => {
    const w = world();
    const s = await open(w, true);
    render(() => <QueuePanel session={s} />);
    const kinds = screen.getAllByTestId('queue-kind');
    expect(kinds.map((k) => k.textContent)).toEqual(['Next step', 'After turn', 'After turn']);
    expect(kinds[0]!.getAttribute('title')).toBe('Sent to the agent after its current step');
    expect(kinds[1]!.getAttribute('title')).toBe('Sent when the agent finishes this turn');
    expect(screen.getByTestId('queue-count').textContent).toContain('Queue · 3');
    expect(screen.getAllByTestId('queue-edit')[0]!.getAttribute('aria-label')).toBe('Edit');
    expect(screen.getAllByTestId('queue-remove')[0]!.getAttribute('aria-label')).toBe('Remove from queue');
    expect(screen.getAllByTestId('queue-remove')[0]!.getAttribute('title')).toBe('Remove from queue');
    expect(screen.getAllByTestId('queue-send-now')[0]!.getAttribute('aria-label')).toBe('Stop the current step and send now');
    expect(screen.queryByText(/^Steer$/i)).toBeNull();
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
    expect(btn()[1]!.getAttribute('aria-label')).toMatch(/^Send at next step/);
    expect(btn()[0]!.getAttribute('aria-label')).toBe('Stop the current step and send now');
    fireEvent.click(btn()[1]!);
    await waitFor(() => expect(edits(w)[0]).toMatchObject({ op: 'sendNow', queue: 'followUp', index: 0 }));
    fireEvent.click(btn()[0]!);
    fireEvent.click(btn()[0]!); // D-8: the first press only arms "Confirm"
    await waitFor(() => expect(cmds(w, 'abort')).toHaveLength(1));
    expect(edits(w)[1]).toMatchObject({ op: 'remove', queue: 'steering', index: 0 });
    await waitFor(() => expect(cmds(w, 'prompt')).toHaveLength(1));
  });

  it('D-8: Send now on a Next step row asks twice (Confirm for 3 s) before stopping the run', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    try {
      const w = world();
      const s = await open(w, true);
      render(() => <QueuePanel session={s} />);
      const btn = () => screen.getAllByTestId('queue-send-now')[0]!;
      fireEvent.click(btn());
      expect(btn().textContent).toBe('Confirm stop & send'); // N-4: visible text, not only the tooltip
      expect(btn().getAttribute('aria-label')).toMatch(/^Confirm: stop the current step/);
      expect(cmds(w, 'abort')).toHaveLength(0);
      expect(edits(w)).toHaveLength(0);
      // no second press inside 3 s: it falls back to the plain button, nothing was sent
      vi.advanceTimersByTime(CONFIRM_MS + 50);
      expect(btn().textContent).toBe('');
      expect(btn().getAttribute('aria-label')).toBe('Stop the current step and send now');
      fireEvent.click(btn());
      vi.advanceTimersByTime(CONFIRM_MS - 500);
      expect(btn().textContent).toBe('Confirm stop & send');
      fireEvent.click(btn());
      await vi.waitFor(() => expect(cmds(w, 'abort')).toHaveLength(1));
      expect(edits(w)[0]).toMatchObject({ op: 'remove', queue: 'steering', index: 0 });
    } finally {
      vi.useRealTimers();
    }
  });

  it('D-8: a follow-up row promotes to Next step in one press (nothing is interrupted)', async () => {
    const w = world();
    const s = await open(w, true);
    render(() => <QueuePanel session={s} />);
    fireEvent.click(screen.getAllByTestId('queue-send-now')[1]!);
    await waitFor(() => expect(edits(w)[0]).toMatchObject({ op: 'sendNow', queue: 'followUp', index: 0 }));
    expect(cmds(w, 'abort')).toHaveLength(0);
  });

  it('D-7 phone: the ⋮ menu carries the confirm too, and Escape-less outside press closes it', async () => {
    const w = world();
    const s = await open(w, true);
    const { container } = render(() => <QueuePanel session={s} phone />);
    const row = container.querySelectorAll('.dock-queue-row')[0]!;
    fireEvent.click(row.querySelector('[data-testid="queue-more"]')!);
    const now = () => row.querySelector('[data-testid="queue-send-now"]') as HTMLElement;
    expect(now().textContent).toContain('Stop the current step and send now');
    fireEvent.click(now());
    expect(now().textContent).toContain('Confirm');
    expect(cmds(w, 'abort')).toHaveLength(0);
    fireEvent.click(now());
    await waitFor(() => expect(cmds(w, 'abort')).toHaveLength(1));
    expect(container.querySelector('[data-testid="queue-menu"]')).toBeNull();
    fireEvent.click(row.querySelector('[data-testid="queue-more"]')!);
    expect(container.querySelector('[data-testid="queue-menu"]')).toBeTruthy();
    fireEvent.pointerDown(document.body);
    expect(container.querySelector('[data-testid="queue-menu"]')).toBeNull();
  });

  it('N-2 phone: Escape with the ⋮ menu open closes only the menu — it never reaches the run-stop rule', async () => {
    const w = world();
    const s = await open(w, true);
    const { container } = render(() => <QueuePanel session={s} phone />);
    const reachedRoot = vi.fn();
    const root = container.parentElement!;
    root.addEventListener('keydown', reachedRoot); // stands in for installEscapeStop on the conversation root
    const row = container.querySelectorAll('.dock-queue-row')[0]!;
    const more = row.querySelector('[data-testid="queue-more"]') as HTMLElement;
    fireEvent.click(more);
    expect(container.querySelector('[data-testid="queue-menu"]')).toBeTruthy();
    fireEvent.keyDown(more, { key: 'Escape' });
    expect(container.querySelector('[data-testid="queue-menu"]')).toBeNull();
    expect(reachedRoot).not.toHaveBeenCalled();
    expect(cmds(w, 'abort')).toHaveLength(0);
    // with no menu open Escape is not ours: it bubbles to the stop rule
    fireEvent.keyDown(more, { key: 'Escape' });
    expect(reachedRoot).toHaveBeenCalledTimes(1);
    root.removeEventListener('keydown', reachedRoot);
  });

  it('N-2 phone: Escape is consumed wherever focus is (the menu closes even from the composer)', async () => {
    const w = world();
    const s = await open(w, true);
    const { container } = render(() => <QueuePanel session={s} phone />);
    const row = container.querySelectorAll('.dock-queue-row')[0]!;
    fireEvent.click(row.querySelector('[data-testid="queue-more"]')!);
    const outside = document.createElement('textarea');
    document.body.appendChild(outside);
    const seen = vi.fn();
    outside.addEventListener('keydown', seen);
    fireEvent.keyDown(outside, { key: 'Escape' });
    expect(container.querySelector('[data-testid="queue-menu"]')).toBeNull();
    expect(seen).not.toHaveBeenCalled();
    outside.remove();
  });

  it('D-4: a held queue chips its rows "Held", not "After turn"', async () => {
    const w = world();
    const s = await open(w, false);
    render(() => <QueuePanel session={s} />);
    expect(screen.getAllByTestId('queue-kind').map((k) => k.textContent)).toEqual(['Held', 'Held', 'Held']);
    expect(screen.getByTestId('queue-count').textContent).toContain('Held · 3');
  });

  it('D-2: a queued image token shows the image this client holds, else an image chip (never the raw token)', async () => {
    const w = world();
    const s = await open(w, true);
    w.push('h1', 's1', [{ session: 's1', seq: 3, event: { type: 'queue_update', steering: [], followUp: ['[image 1] look at this', '[image 1] and [image 2] lost'], followUpIds: ['m1', 'm2'] } }]);
    await waitFor(() => expect(s.state.queue.followUp).toHaveLength(2));
    // what the store keeps when the outbox twin settles by queue id (see the outbox test)
    const held = { m1: [{ mimeType: 'image/png', data: 'AAAA' }] };
    const view = render(() => <QueuePanel session={{ ...s, state: new Proxy(s.state, { get: (t, k) => (k === 'queueImages' ? held : (t as never)[k]) }) } as SessionHandle} />);
    const rows = view.container.querySelectorAll('.dock-queue-row');
    const thumb = rows[0]!.querySelector('[data-testid="queue-thumb"]') as HTMLImageElement;
    expect(thumb.getAttribute('src')).toBe('data:image/png;base64,AAAA');
    expect(rows[0]!.textContent).not.toContain('[image');
    expect(rows[0]!.textContent).toContain('look at this');
    expect(rows[1]!.querySelectorAll('[data-testid="queue-image-chip"]')).toHaveLength(2);
    expect(rows[1]!.textContent).not.toContain('[image');
    expect(rows[1]!.textContent).toContain('and');
  });

  it('queueTextParts pairs image N with the Nth held image', () => {
    const img = { mimeType: 'image/png', data: 'x' };
    expect(queueTextParts('a [image 2]b', [img])).toEqual([{ text: 'a ' }, { image: undefined, n: 2 }, { text: 'b' }]);
    expect(queueTextParts('[image 1]', [img])).toEqual([{ image: img, n: 1 }]);
    expect(queueTextParts('plain')).toEqual([{ text: 'plain' }]);
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
    // D-7: one ⋮ per row instead of five icon buttons; the actions are in its menu.
    expect(rows[1]!.querySelectorAll('button')).toHaveLength(1);
    fireEvent.click(rows[1]!.querySelector('[data-testid="queue-more"]')!);
    const items = [...rows[1]!.querySelectorAll('[role="menuitem"]')].map((b) => b.textContent!.trim());
    expect(items).toEqual(['Move up', 'Move down', 'Edit', 'Send at next step', 'Remove from queue']);
    fireEvent.click([...rows[1]!.querySelectorAll('[role="menuitem"]')][1]!);
    expect(container.querySelector('[data-testid="queue-menu"]')).toBeNull();
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
