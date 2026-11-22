/**
 * w5-ui D4: the user's own text after `/goal`, `/swarm`, `/graph` or `/ralph` arrives as an extension-authored user
 * message long after the command echo (hooks and notes land in between). It must pair with that echo (the typed-command
 * bubble) instead of rendering as a grey "System" notification, and the Mode / Memory rows belong to the new turn.
 */
import { describe, expect, it } from 'vitest';
import { buildTurns, createTurnsIndex, type TurnsInput } from './turns';
import type { AgentMessage, SessionEntry } from './types';

const T0 = Date.UTC(2026, 9, 30, 9, 0, 0);

function session() {
  const entries: SessionEntry[] = [];
  let t = T0;
  const add = (e: Omit<SessionEntry, 'id' | 'parentId' | 'timestamp'>, dt = 1000): SessionEntry => {
    t += dt;
    const entry = { ...e, id: `e${entries.length + 1}`, parentId: entries.at(-1)?.id ?? null, timestamp: new Date(t).toISOString() } as SessionEntry;
    entries.push(entry);
    return entry;
  };
  const msg = (message: AgentMessage, dt?: number) => add({ type: 'message', message: { timestamp: t + (dt ?? 1000), ...message } }, dt);
  return {
    entries,
    add,
    user: (text: string, extra: Record<string, unknown> = {}) => msg({ role: 'user', content: [{ type: 'text', text }], ...extra }),
    answer: (text: string) => msg({ role: 'assistant', content: [{ type: 'text', text }], stopReason: 'stop', model: 'm', usage: { cost: { total: 0.01 } } }, 2000),
    echo: (name: string, args: string) => add({ type: 'custom', customType: 'oino.command-echo', data: { name, args } } as never),
    decision: (mode: string) => add({ type: 'custom', customType: 'long-horizon-decision', data: { mode } } as never),
    response: (text: string) => add({ type: 'custom_message', customType: 'unipi-response', content: text, display: true, details: { source: 'Long-horizon' } } as never),
    note: (text: string) => add({ type: 'custom_message', customType: 'unipi-note', content: text, display: true } as never),
  };
}

const input = (entries: SessionEntry[], over: Partial<TurnsInput> = {}): TurnsInput => ({ entries, leafId: entries.at(-1)?.id ?? null, entriesRev: 1, liveTools: {}, run: { running: false }, ...over });

/** One finished plain turn, then `/swarm <text>` with hook entries between the echo and the extension's user message. */
function swarmChat() {
  const s = session();
  s.user('earlier question');
  s.answer('earlier answer');
  const echo = s.echo('swarm', 'refactor the parser');
  const decision = s.decision('swarm');
  s.add({ type: 'custom', customType: 'compactor-origin', data: {} } as never);
  s.response('Swarm started');
  s.add({ type: 'custom_message', customType: 'unipi-run-begun', content: 'run', display: false } as never);
  const user = s.user('refactor the parser', { author: 'extension' });
  s.answer('on it');
  return { s, echo, decision, user };
}

describe('D4: extension prompt pairs with its command echo', () => {
  it('the user message becomes the typed command of its turn, whatever sits between', () => {
    const { s, echo, user } = swarmChat();
    const turns = buildTurns(input(s.entries));
    expect(turns).toHaveLength(2);
    expect(turns[1]!.turnId).toBe(user.id);
    expect(turns[1]!.command).toMatchObject({ name: 'swarm', args: 'refactor the parser', entryId: echo.id, hidden: false });
    expect(turns[1]!.source).toBe('harness');
  });

  it('the new turn owns the command and the hook entries; the previous turn ends at its answer ("Done in …")', () => {
    const { s, echo, decision, user } = swarmChat();
    const [prev, next] = buildTurns(input(s.entries));
    expect(prev!.entries.map((e) => e.id)).toEqual(['e1', 'e2']);
    expect(next!.entries[0]).toBe(echo);
    expect(next!.entries.map((e) => e.id)).toContain(decision.id);
    expect(next!.user).toBe(user);
    // The Mode row is in the new turn's rows, never above the previous turn's answer.
    const keys = (t: typeof prev) => t!.segments.flatMap((g) => [...(g.before ?? []), ...g.work, ...(g.after ?? [])]).map((w) => (w.kind === 'card' ? w.entry.id : ''));
    expect(keys(next)).toContain(decision.id);
    expect(keys(prev)).not.toContain(decision.id);
    expect(prev!.durationMs).toBeDefined();
  });

  it('works with a busy-refusal note between the echo and the message', () => {
    const s = session();
    s.user('one');
    s.answer('a');
    const echo = s.echo('graph', 'ship it');
    s.note('Graph not started: the chat “Other” is running the graph “X”. Your message was sent as a normal message.');
    s.user('ship it', { author: 'extension' });
    s.answer('b');
    const turns = buildTurns(input(s.entries));
    expect(turns).toHaveLength(2);
    expect(turns[1]!.command).toMatchObject({ name: 'graph', args: 'ship it', entryId: echo.id });
  });

  it('stops at the next real message: an echo before an earlier user/assistant message is not stolen', () => {
    const s = session();
    s.user('one');
    s.answer('a');
    s.echo('ralph', 'old');
    s.user('typed by the human'); // a real user message ends the search
    s.answer('b');
    s.user('Agent graph checkpoint: ready', { author: 'extension' });
    const turns = buildTurns(input(s.entries));
    expect(turns.at(-1)!.command).toBeUndefined();
    expect(turns[1]!.command).toMatchObject({ name: 'ralph' });
  });

  it('an echo with no extension message after it leaves the previous turn alone', () => {
    const s = session();
    s.user('one');
    s.answer('a');
    s.echo('plan', 'on');
    const turns = buildTurns(input(s.entries));
    expect(turns).toHaveLength(1);
  });

  it('the incremental index builds the same turns as a full build, as entries stream in', () => {
    const all = swarmChat().s.entries;
    const index = createTurnsIndex();
    const grown: SessionEntry[] = []; // one growing array, like the store: the index takes its `tail` path
    const shape = (turns: ReturnType<typeof buildTurns>) => turns.map((t) => [t.turnId, t.entries.map((e) => e.id), t.command?.name]);
    let modes = '';
    for (const e of all) {
      grown.push(e);
      const incremental = index.update({ ...input(grown), entriesRev: 1 });
      modes += index.lastMode[0];
      expect(shape(incremental)).toEqual(shape(buildTurns(input(grown.slice()))));
    }
    expect(modes).toContain('t');
  });
});

describe('D3: a mode command with text keeps its words on screen before (and without) the extension message', () => {
  it('the bare echo is a turn of its own: the typed command, no user message yet', () => {
    const s = session();
    s.user('one');
    s.answer('a');
    const echo = s.echo('swarm', 'refactor the parser');
    s.decision('swarm'); // hooks are still running
    const turns = buildTurns(input(s.entries));
    expect(turns).toHaveLength(2);
    expect(turns[0]!.entries.map((e) => e.id)).toEqual(['e1', 'e2']);
    expect(turns[1]!).toMatchObject({ kind: 'turn', turnId: echo.id, source: 'harness', echoKey: '/swarm refactor the parser', command: { name: 'swarm', args: 'refactor the parser' } });
    expect(turns[1]!.user).toBeUndefined();
  });

  it('the swap to the message\'s turn happens in one update (same words, no empty frame)', () => {
    const s = session();
    s.user('one');
    s.answer('a');
    s.echo('graph', 'ship it');
    const index = createTurnsIndex();
    const grown: SessionEntry[] = [...s.entries];
    const before = index.update(input(grown));
    expect(before.at(-1)!.echoKey).toBe('/graph ship it');
    grown.push(s.add({ type: 'custom_message', customType: 'unipi-response', content: 'started', display: true } as never));
    const user = s.add({ type: 'message', message: { role: 'user', author: 'extension', timestamp: Date.now(), content: [{ type: 'text', text: 'ship it' }] } } as never);
    grown.push(user);
    const after = index.update(input(grown));
    expect(after).toHaveLength(2);
    expect(after.at(-1)).toMatchObject({ turnId: user.id, command: { name: 'graph', args: 'ship it' } });
    expect(after.at(-1)!.echoKey).toBeUndefined();
  });

  it('plain commands without text, hidden ones and echoes claimed by a user message are not echo turns', () => {
    const s = session();
    s.user('one');
    s.answer('a');
    s.echo('plan', 'on');
    s.echo('swarm', '');
    s.echo('compact', 'now');
    expect(buildTurns(input(s.entries))).toHaveLength(1);
    const t = session();
    t.echo('goal', 'ship');
    t.user('ship the thing'); // a normal message directly after the echo: the old link
    t.answer('ok');
    const turns = buildTurns(input(t.entries));
    expect(turns).toHaveLength(2);
    expect(turns.every((x) => x.echoKey === undefined)).toBe(true);
  });
});
