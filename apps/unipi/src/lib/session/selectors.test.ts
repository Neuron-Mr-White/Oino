import { describe, expect, it } from 'vitest';
import type { SessionCard } from '../host/client';
import { initialState } from './reducer';
import { enqueue, markAcked, PENDING_ACKED_MS, PENDING_LOST_MS, settleByMessage, settleByQueue, sweep } from './outbox';
import { connBanner, connDown, isBareHomeCwd, isHomeCwd, isMovableCwd, isRunning, needsYou, NO_WORKSPACE_LABEL, outboxLabel, projectsOf, sessionCost, sessionCwd, sessionTitle, shortCwd } from './selectors';

const card = (id: string, over: Partial<SessionCard> = {}): SessionCard => ({ session_id: id, run_state: 'idle', ...over });

describe('selectors', () => {
  it('title: session name, else card name, else project folder (or New chat when in No workspace, D-18)', () => {
    expect(sessionTitle(card('a', { name: ' Fix ', cwd: '/x/proj' }))).toBe('Fix');
    expect(sessionTitle(card('a', { cwd: '/x/proj/' }))).toBe('proj');
    expect(sessionTitle(card('a', { cwd: '/home/example' }))).toBe('New chat');
    expect(sessionTitle(card('a', { name: 'Card' }), { name: 'Live' })).toBe('Live');
    expect(isHomeCwd('/home/example')).toBe(true);
    expect(isHomeCwd('/home/example/qa1ws')).toBe(false);
    expect(isHomeCwd('/')).toBe(true);
    expect(isBareHomeCwd('/home/example/')).toBe(true);
    expect(isBareHomeCwd('C:\\Users\\oi')).toBe(true);
    expect(isBareHomeCwd('/home/example/.local/share/oino/scratch/abc')).toBe(false);
    expect(isBareHomeCwd('/')).toBe(false);
    // Move to a workspace: a scratch chat, or an older chat that ran in the bare home — nothing else.
    expect(isMovableCwd('/home/example/.local/share/oino/scratch/abc')).toBe(true);
    expect(isMovableCwd('/home/example')).toBe(true);
    expect(isMovableCwd('/Users/oi')).toBe(true);
    expect(isMovableCwd('/home/example/proj')).toBe(false);
    expect(isMovableCwd('/')).toBe(false);
    expect(isMovableCwd(undefined)).toBe(false);
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
        card('home1', { cwd: '/home/example', updated_at: 3 }),
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

describe('connDown / connBanner after the chat was live (r8 P2-A)', () => {
  const live = () => {
    const s = initialState('h', 'a');
    s.conn = 'live';
    s.everLive = true;
    return s;
  };
  it('every state of a chat that WAS live except live is down: the banner stays and the run is stale', () => {
    const s = live();
    expect(connDown(s)).toBe(false);
    s.conn = 'connecting';
    expect(connDown(s)).toBe(true);
    expect(connBanner(s)?.text).toBe('Lost the connection · reconnecting…');
    s.conn = 'closed';
    s.connError = { code: 'E_UNAUTHORIZED', message: 'Pair this device first', retryable: false };
    expect(connDown(s)).toBe(true);
    // The closed-with-error banner is the Conversation's own (it says the error), not this one.
    expect(connBanner(s)).toBeUndefined();
  });
  it('a first open, a hang-up, a missing chat and a take-over are not "lost"', () => {
    const first = initialState('h', 'a');
    expect(first.conn).toBe('connecting');
    expect(connDown(first)).toBe(false);
    expect(connBanner(first)).toBeUndefined();
    const s = live();
    s.conn = 'closed';
    expect(connDown(s)).toBe(false);
    s.connError = { code: 'E_NOT_FOUND', message: 'gone', retryable: false };
    expect(connDown(s)).toBe(false);
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

  it('legacy id-less items settle by echoed text (whitespace-insensitive) and queue for steers only', () => {
    const s = initialState('h', 'a');
    enqueue(s, { text: 'fix  it\n' }, true);
    enqueue(s, { text: 'steer me', mode: 'steer' }, true);
    enqueue(s, { text: 'later', mode: 'prompt' }, false);
    for (const item of s.outbox) delete item.messageId;
    settleByMessage(s, { role: 'user', content: 'fix it' });
    settleByQueue(s, ['steer me', 'later']);
    expect(s.outbox.map((o) => o.text)).toEqual(['later']);
  });

  it('settles a skill prompt by its expanded <skill> echo', () => {
    const s = initialState('h', 'a');
    enqueue(s, { text: '/skill:impeccable [image 1] reduce colors with $impeccable' }, true, 1000);
    settleByMessage(s, { role: 'user', content: '<skill name="impeccable" location="/x/SKILL.md">\nlong body\n</skill>\n\n[image 1] reduce colors with $impeccable', timestamp: 1500 } as never);
    expect(s.outbox).toEqual([]);
  });

  it('settles an expanded /command by the next newer user message, never by an older one', () => {
    const s = initialState('h', 'a');
    enqueue(s, { text: '/summary' }, true, 10_000);
    settleByMessage(s, { role: 'user', content: 'an older message', timestamp: 1_000 } as never);
    expect(s.outbox).toHaveLength(1);
    settleByMessage(s, { role: 'user', content: 'Summarize this session: …template body…', timestamp: 10_400 } as never);
    expect(s.outbox).toEqual([]);
    enqueue(s, { text: 'plain text' }, true, 20_000);
    settleByMessage(s, { role: 'user', content: 'something else', timestamp: 20_500 } as never);
    expect(s.outbox).toHaveLength(1);
  });
});
