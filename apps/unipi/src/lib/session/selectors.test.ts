import { describe, expect, it } from 'vitest';
import type { SessionCard } from '../host/client';
import { initialState } from './reducer';
import { enqueue, markAcked, PENDING_ACKED_MS, PENDING_LOST_MS, settleByMessage, settleByQueue, sweep } from './outbox';
import { connBanner, isHomeCwd, isRunning, needsYou, NO_WORKSPACE_LABEL, outboxLabel, projectsOf, sessionCost, sessionCwd, sessionTitle, shortCwd } from './selectors';

const card = (id: string, over: Partial<SessionCard> = {}): SessionCard => ({ session_id: id, run_state: 'idle', ...over });

describe('selectors', () => {
  it('title: session name, else card name, else project folder (or New chat when in No workspace, D-18)', () => {
    expect(sessionTitle(card('a', { name: ' Fix ', cwd: '/x/proj' }))).toBe('Fix');
    expect(sessionTitle(card('a', { cwd: '/x/proj/' }))).toBe('proj');
    expect(sessionTitle(card('a', { cwd: '/home/oi' }))).toBe('New chat');
    expect(sessionTitle(card('a', { name: 'Card' }), { name: 'Live' })).toBe('Live');
    expect(isHomeCwd('/home/oi')).toBe(true);
    expect(isHomeCwd('/home/oi/qa1ws')).toBe(false);
    expect(sessionCwd(card('a', { cwd: '/x/proj/' }))).toBe('/x/proj');
    expect(shortCwd('/home/dev/p')).toBe('~/p');
  });

  it('running / needs-you prefer the open session over the card', () => {
    const s = initialState('h', 'a');
    expect(isRunning(card('a', { run_state: 'working' }))).toBe(true);
    expect(isRunning(card('a', { run_state: 'working' }), s)).toBe(false);
    s.run = { phase: 'compacting', running: false };
    expect(isRunning(undefined, s)).toBe(true);
    expect(needsYou(card('a', { needs_you: true }))).toBe(true);
    s.dialogs = ['d1'];
    expect(needsYou(card('a'), s)).toBe(true);
  });

  it('cost sums assistant usage', () => {
    const s = initialState('h', 'a');
    s.entries = [
      { type: 'message', id: '1', message: { role: 'assistant', usage: { cost: { total: 0.01 } } } },
      { type: 'message', id: '2', message: { role: 'user' } },
      { type: 'message', id: '3', message: { role: 'assistant', usage: { cost: { total: 0.025 } } } },
    ];
    expect(sessionCost(s)).toBeCloseTo(0.035);
  });

  it('projects (D-9): pinned → running → needs-you → recent; projects by latest activity', () => {
    const groups = projectsOf(
      [
        card('old', { cwd: '/p/a', updated_at: 10 }),
        card('run', { cwd: '/p/a', run_state: 'working', updated_at: 5 }),
        card('ask', { cwd: '/p/a', needs_you: true, updated_at: 7 }),
        card('pin', { cwd: '/p/a', updated_at: 1 }),
        card('new', { cwd: '/p/a', updated_at: 20 }),
        card('b1', { cwd: '/p/b', updated_at: 30 }),
        card('home1', { cwd: '/home/oi', updated_at: 3 }),
        card('none', { updated_at: 2 }),
      ],
      { pin: { pinned: true } },
    );
    expect(groups.map((g) => [g.label, g.cards.map((c) => c.session_id)])).toEqual([
      ['b', ['b1']],
      ['a', ['pin', 'run', 'ask', 'new', 'old']],
      [NO_WORKSPACE_LABEL, ['home1', 'none']],
    ]);
    expect(groups[1]).toMatchObject({ running: 1, needsYou: 1, lastActivity: 20 });
  });

  it('connection banner and outbox words (Z-50)', () => {
    const s = initialState('h', 'a');
    s.conn = 'live';
    expect(connBanner(s)).toBeUndefined();
    s.conn = 'reconnecting';
    expect(connBanner(s)).toEqual({ text: 'Lost the connection · reconnecting…', action: 'retry' });
    s.conn = 'lost';
    expect(connBanner(s)).toEqual({ text: 'Lost the connection', action: 'retry' });
    expect(outboxLabel('waiting')).toBe('Waiting for connection…');
    expect(outboxLabel('lost')).toBe('Not sent');
    expect(outboxLabel('acked')).toBeUndefined();
  });
});

describe('outbox (pure)', () => {
  it('acked items go quietly after 20 s; unacked ones become lost after 30 s', () => {
    const s = initialState('h', 'a');
    const a = enqueue(s, { text: 'a' }, true, 0);
    const b = enqueue(s, { text: 'b' }, true, 0);
    markAcked(s, a.ref, 0);
    expect(sweep(s, PENDING_ACKED_MS)).toEqual([]);
    expect(sweep(s, PENDING_LOST_MS + 1)).toEqual([b.ref]);
    expect(s.outbox).toEqual([expect.objectContaining({ ref: b.ref, status: 'lost' })]);
  });

  it('settles by the echoed user text (whitespace-insensitive) and by the queue for steers only', () => {
    const s = initialState('h', 'a');
    enqueue(s, { text: 'fix  it\n' }, true);
    enqueue(s, { text: 'steer me', mode: 'steer' }, true);
    enqueue(s, { text: 'later', mode: 'prompt' }, false);
    settleByMessage(s, { role: 'user', content: 'fix it' });
    settleByQueue(s, ['steer me', 'later']);
    expect(s.outbox.map((o) => o.text)).toEqual(['later']);
  });
});
