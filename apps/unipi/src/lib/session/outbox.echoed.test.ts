/**
 * w6-ui G2-4: an outbox bubble is hidden only by ITS OWN command echo (written after it was sent, claimed once),
 * never by an equal-text echo that already belonged to an earlier send.
 */
import { describe, expect, it } from 'vitest';
import { buildTurns } from './turns';
import { echoedRefs, settleByEntry } from './outbox';
import type { OutboxItem, SessionEntry } from './types';

const T0 = Date.UTC(2026, 9, 30, 9, 0, 0);
const echo = (id: string, at: number, name: string, args: string): SessionEntry =>
  ({ id, parentId: null, type: 'custom', customType: 'oino.command-echo', data: { name, args }, timestamp: new Date(at).toISOString() }) as never;
const item = (ref: string, text: string, at: number, status: OutboxItem['status'] = 'acked'): OutboxItem => ({ ref, text, mode: 'prompt', at, status });
const turnsOf = (entries: SessionEntry[]) => buildTurns({ entries, leafId: entries.at(-1)?.id ?? null, entriesRev: 1, liveTools: {}, run: { running: false } });

describe('echoedRefs', () => {
  it('hides the item whose own echo arrived', () => {
    const turns = turnsOf([echo('e1', T0 + 500, 'swarm', 'repeat')]);
    expect([...echoedRefs([item('a', '/swarm repeat', T0)], turns)]).toEqual(['a']);
  });

  it('keeps a distinct repeated command visible while only the EARLIER command has an echo', () => {
    const turns = turnsOf([echo('e1', T0 + 500, 'swarm', 'repeat')]);
    // the first send settled earlier; the second identical text was enqueued 60 s later, its echo not here yet
    expect(echoedRefs([item('b', '/swarm  repeat', T0 + 60_000)], turns).size).toBe(0);
  });

  it('two identical sends need two echoes: one echo hides only the oldest', () => {
    const turns = turnsOf([echo('e1', T0 + 500, 'swarm', 'repeat')]);
    const paired = echoedRefs([item('a', '/swarm repeat', T0), item('b', '/swarm repeat', T0 + 100)], turns);
    expect([...paired]).toEqual(['a']);
    const both = echoedRefs([item('a', '/swarm repeat', T0), item('b', '/swarm repeat', T0 + 100)], turnsOf([echo('e1', T0 + 500, 'swarm', 'repeat'), echo('e2', T0 + 900, 'swarm', 'repeat')]));
    expect([...both].sort()).toEqual(['a', 'b']);
  });

  it('a lost item always shows, and other text is never paired', () => {
    const turns = turnsOf([echo('e1', T0 + 500, 'swarm', 'repeat')]);
    expect(echoedRefs([item('a', '/swarm repeat', T0, 'lost'), item('b', '/swarm other', T0)], turns).size).toBe(0);
  });
});

/**
 * w7-ui (beqa G3-2): the host's `oino.command-echo` carries the request's `messageId`; the outbox item carries the id it
 * sent. Equal ids pair whatever the clocks say; the text/time heuristic above stays only for an echo without an id.
 */
describe('echoedRefs by request id', () => {
  const ID_A = '019b0000-0000-7000-8000-00000000000a';
  const ID_B = '019b0000-0000-7000-8000-00000000000b';
  const echoId = (id: string, at: number, name: string, args: string, messageId?: string): SessionEntry =>
    ({ id, parentId: null, type: 'custom', customType: 'oino.command-echo', data: { name, args, ...(messageId ? { messageId } : {}) }, timestamp: new Date(at).toISOString() }) as never;
  const withId = (ref: string, text: string, at: number, messageId: string, status: OutboxItem['status'] = 'acked'): OutboxItem => ({ ...item(ref, text, at, status), messageId });

  it('the turn reads the echo\'s messageId', () => {
    expect(turnsOf([echoId('e1', T0, 'swarm', 'repeat', ID_A)]).at(-1)?.echoMessageId).toBe(ID_A);
    expect(turnsOf([echo('e1', T0, 'swarm', 'repeat')]).at(-1)?.echoMessageId).toBeUndefined();
  });

  it('rapid repeat, first item already retired: the old echo cannot hide the new pending bubble (beqa repro)', () => {
    // item `second` sent at T0+10 s; the first send's echo (id A) is still in history; `second` has id B, its echo not here yet
    const turns = turnsOf([echoId('e1', T0 + 9_000, 'swarm', 'repeat', ID_A)]);
    expect(echoedRefs([withId('second', '/swarm repeat', T0 + 10_000, ID_B)], turns).size).toBe(0);
    // …and the second's own echo then hides it, in any order of arrival
    const both = turnsOf([echoId('e1', T0 + 9_000, 'swarm', 'repeat', ID_A), echoId('e2', T0 + 11_000, 'swarm', 'repeat', ID_B)]);
    expect([...echoedRefs([withId('second', '/swarm repeat', T0 + 10_000, ID_B)], both)]).toEqual(['second']);
  });

  it('a phone clock ahead of the host does not leave both the echo and the pending bubble visible', () => {
    const turns = turnsOf([echoId('e1', T0 + 10_000, 'swarm', 'repeat', ID_A)]);
    expect([...echoedRefs([withId('a', '/swarm repeat', T0 + 20_000, ID_A)], turns)]).toEqual(['a']);
  });

  it('pairs two identical sends each with its own echo, by id, independent of order', () => {
    const turns = turnsOf([echoId('e2', T0 + 900, 'swarm', 'repeat', ID_B), echoId('e1', T0 + 500, 'swarm', 'repeat', ID_A)]);
    const items = [withId('a', '/swarm repeat', T0, ID_A), withId('b', '/swarm repeat', T0 + 100, ID_B)];
    expect([...echoedRefs(items, turns)].sort()).toEqual(['a', 'b']);
    expect([...echoedRefs([items[1]!], turns)]).toEqual(['b']);
  });

  it('an echo that has an id is never claimed by the text heuristic when another item owns the id', () => {
    const turns = turnsOf([echoId('e1', T0 + 500, 'swarm', 'repeat', ID_A)]);
    // `b` (id B, same text, sent a moment after) must not take A's echo just because the text and time fit
    expect(echoedRefs([withId('a', '/swarm repeat', T0, ID_A), withId('b', '/swarm repeat', T0 + 100, ID_B)], turns).has('b')).toBe(false);
  });

  it('an echo without an id (older host) still pairs through the old heuristic, even with an id on the item', () => {
    const turns = turnsOf([echo('e1', T0 + 500, 'swarm', 'repeat')]);
    expect([...echoedRefs([withId('a', '/swarm repeat', T0, ID_A)], turns)]).toEqual(['a']);
    // …and the old rules still hold there: an echo older than the send does not pair
    expect(echoedRefs([withId('b', '/swarm repeat', T0 + 60_000, ID_B)], turns).size).toBe(0);
  });

  it('an item without an id (a persisted waiting item from an older app) pairs with an id-less echo only', () => {
    const turns = turnsOf([echoId('e1', T0 + 500, 'swarm', 'repeat', ID_A)]);
    expect([...echoedRefs([item('a', '/swarm repeat', T0)], turns)]).toEqual(['a']); // nobody owns A: heuristic applies
    // A is owned by x: x takes it by id, the id-less item `a` may not take it by text
    expect([...echoedRefs([item('a', '/swarm repeat', T0), withId('x', '/swarm repeat', T0 + 50, ID_A)], turns)]).toEqual(['x']);
  });
});

describe('settleByEntry', () => {
  const ID = '019b0000-0000-7000-8000-0000000000aa';
  const state = (items: OutboxItem[]) => ({ outbox: items }) as never;
  const user = (id: string) => ({ id, type: 'message', message: { role: 'user', content: 'x' } }) as never;

  it('removes the bubble whose id the entry carries, for a plain prompt and for an extension handoff', () => {
    const a = { ...item('a', '/goal ship it', T0), messageId: ID };
    const s1 = state([a, { ...item('b', 'other', T0), messageId: '019b0000-0000-7000-8000-0000000000bb' }]);
    expect(settleByEntry(s1, user(ID))).toBe(true);
    expect((s1 as { outbox: OutboxItem[] }).outbox.map((o) => o.ref)).toEqual(['b']);
    const s2 = state([a]);
    expect(settleByEntry(s2, user(`${ID}:extension:abcdef:0`))).toBe(true);
    expect((s2 as { outbox: OutboxItem[] }).outbox).toEqual([]);
  });

  it('leaves everything alone for an entry whose id is nobody\'s (another client, an older host), or a non-user entry', () => {
    const s = state([{ ...item('a', 'hi', T0), messageId: ID }]);
    expect(settleByEntry(s, user('c0000007'))).toBe(false);
    expect(settleByEntry(s, { id: ID, type: 'message', message: { role: 'assistant', content: [] } } as never)).toBe(false);
    expect((s as { outbox: OutboxItem[] }).outbox).toHaveLength(1);
  });
});
