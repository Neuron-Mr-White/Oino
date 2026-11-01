/**
 * Notification routing tests (WP-A19 acceptance): focused session suppressed,
 * seed on first look, max 3, no replay of cold snapshots; Z-86 foreground
 * completions (seen in front ≠ unread, blur right after marks it).
 */
import { describe, expect, it } from 'vitest';
import type { SessionCard } from '../host/client';
import { attentionItem, attentionOf, attentionText, ForegroundCompletions, NotifyRouter, planNotifications, type Attention } from './route';

const card = (over: Partial<SessionCard> = {}): SessionCard => ({
  session_id: 's1',
  name: 'math helpers',
  cwd: '/home/you/unipi',
  run_state: 'idle',
  ...over,
});

const attention = (over: Partial<Attention> = {}): Attention => ({
  kind: 'waiting',
  hostId: 'this-pc',
  hostName: 'workstation',
  card: card(),
  prev: card({ run_state: 'working' }),
  ...over,
});

describe('attentionOf', () => {
  it('first sighting seeds (no attention — cold snapshots never replay)', () => {
    expect(attentionOf(undefined, card({ needs_you: true }))).toBeUndefined();
  });

  it('needs_you rising is waiting; working→idle finished; working→crashed crashed', () => {
    expect(attentionOf(card(), card({ needs_you: true }))).toBe('waiting');
    expect(attentionOf(card({ run_state: 'working' }), card({ run_state: 'idle' }))).toBe('finished');
    expect(attentionOf(card({ run_state: 'working' }), card({ run_state: 'crashed', error: 'boom' }))).toBe('crashed');
  });

  it('a repeat of the same state, a fall, an idle→working and a takeover are nothing', () => {
    expect(attentionOf(card({ needs_you: true }), card({ needs_you: true }))).toBeUndefined();
    expect(attentionOf(card({ needs_you: true }), card())).toBeUndefined();
    expect(attentionOf(card(), card({ run_state: 'working' }))).toBeUndefined();
    expect(attentionOf(card({ run_state: 'working' }), card({ run_state: 'taken_over' }))).toBeUndefined();
  });
});

describe('planNotifications', () => {
  const a = (n: number, kind: Attention['kind'] = 'waiting', sessionId = `s${n}`): Attention =>
    attention({ kind, card: card({ session_id: sessionId }), prev: undefined });

  it('the focused session is suppressed', () => {
    const plan = planNotifications([a(1), a(2)], { focused: { hostId: 'this-pc', sessionId: 's1' }, windowFocused: false });
    expect(plan.map((p) => p.card.session_id)).toEqual(['s2']);
  });

  it('a focused window suppresses everything (the bell is visible)', () => {
    expect(planNotifications([a(1)], { windowFocused: true })).toEqual([]);
  });

  it('max 3 per drain, newest-first order kept', () => {
    const plan = planNotifications([a(1), a(2), a(3), a(4), a(5)], { windowFocused: false });
    expect(plan).toHaveLength(3);
    expect(plan.map((p) => p.card.session_id)).toEqual(['s1', 's2', 's3']);
  });

  it('wording (unipi seed text)', () => {
    expect(attentionText('finished', card(), 'coffee')).toEqual({ title: 'math helpers finished', body: 'The agent on coffee is done.' });
    expect(attentionText('waiting', card(), 'coffee')).toEqual({ title: 'math helpers needs you', body: 'The agent on coffee is waiting for your answer.' });
    expect(attentionText('crashed', card({ error: 'overloaded' }), 'coffee')).toEqual({ title: 'math helpers stopped', body: 'overloaded' });
  });
});

describe('attentionItem (Z-83 drawer feed)', () => {
  it('waiting → the Waiting group, finished → Finished, crashed → Other with the error', () => {
    const waiting = attentionItem(attention(), 1000);
    expect(waiting).toMatchObject({ hostId: 'this-pc', hostName: 'workstation', session: 's1', kind: 'input_needed', group: 'waiting', since: 1000 });
    const finished = attentionItem(attention({ kind: 'finished' }), 2000);
    expect(finished).toMatchObject({ kind: 'agent_end', group: 'finished', since: 2000 });
    const crashed = attentionItem(attention({ kind: 'crashed', card: card({ error: 'OOM' }) }), 3000);
    expect(crashed).toMatchObject({ kind: 'crashed', group: 'other', what: 'OOM' });
  });
});

describe('NotifyRouter', () => {
  it('cold cards seed, then only transitions carry attention (no replay after a restart)', () => {
    const router = new NotifyRouter();
    // First look: the whole cold list seeds.
    expect(router.observe('h', 'pc', card({ needs_you: true }))).toEqual({ seeded: true });
    expect(router.observe('h', 'pc', card({ session_id: 's2', run_state: 'working' }))).toEqual({ seeded: true });
    // The same states again: nothing.
    expect(router.observe('h', 'pc', card({ needs_you: true }))).toEqual({ seeded: false });
    // The wait falls, then rises again: attention, with the previous card.
    expect(router.observe('h', 'pc', card())).toEqual({ seeded: false });
    const { attention: a } = router.observe('h', 'pc', card({ needs_you: true, name: 'math helpers v2' }));
    expect(a?.kind).toBe('waiting');
    expect(a?.prev?.name).toBe('math helpers');
    // A brand-new session first seen waiting seeds (cold), its next wait notifies.
    expect(router.observe('h', 'pc', card({ session_id: 's9', needs_you: true }))).toEqual({ seeded: true });
    expect(router.observe('h', 'pc', card({ session_id: 's9' }))).toEqual({ seeded: false });
    const again = router.observe('h', 'pc', card({ session_id: 's9', needs_you: true, last_activity: 5 }));
    expect(again.attention?.kind).toBe('waiting');
  });

  it('forget drops the baseline so a re-listed session seeds again', () => {
    const router = new NotifyRouter();
    router.observe('h', 'pc', card({ run_state: 'working' }));
    router.forget('h', 's1');
    expect(router.observe('h', 'pc', card({ run_state: 'idle' }))).toEqual({ seeded: true });
  });
});

describe('ForegroundCompletions (Z-86)', () => {
  const ref = { hostId: 'h', sessionId: 's1' };

  it('a completion seen in front is only recorded; leaving the window drains it once', () => {
    const fg = new ForegroundCompletions();
    expect(fg.size).toBe(0);
    fg.record(ref);
    fg.record(ref); // dedup
    expect(fg.size).toBe(1);
    expect(fg.takeAll()).toEqual([ref]);
    expect(fg.size).toBe(0);
    expect(fg.takeAll()).toEqual([]); // only the first blur marks it
  });

  it('forget drops a pending record (the user moved on inside the app)', () => {
    const fg = new ForegroundCompletions();
    fg.record(ref);
    fg.forget(ref);
    expect(fg.takeAll()).toEqual([]);
  });
});
