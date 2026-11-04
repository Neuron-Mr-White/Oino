/** [ask-user] `ask_user` calls leave the "Worked for …" fold: one inline card where they were asked. */
import { describe, expect, it } from 'vitest';
import { buildTurns, type TurnsInput } from './turns';
import type { AgentMessage, SessionEntry } from './types';

const T0 = Date.UTC(2026, 10, 4, 9, 0, 0);

function chat() {
  const entries: SessionEntry[] = [];
  let t = T0;
  const msg = (message: AgentMessage, dt = 1000) => {
    t += dt;
    const e = { type: 'message', message: { timestamp: t, ...message }, id: `e${entries.length + 1}`, parentId: entries.at(-1)?.id ?? null, timestamp: new Date(t).toISOString() } as SessionEntry;
    entries.push(e);
    return e;
  };
  const call = (id: string, name: string, args: Record<string, unknown>, text?: string) =>
    msg({ role: 'assistant', content: [...(text ? [{ type: 'text', text }] : []), { type: 'toolCall', id, name, arguments: args }], stopReason: 'toolUse' });
  const result = (id: string, name: string, text: string, details?: unknown, dt?: number) =>
    msg({ role: 'toolResult', toolCallId: id, toolName: name, content: [{ type: 'text', text }], isError: false, ...(details ? { details } : {}) }, dt);
  return { entries, msg, call, result };
}

const input = (entries: SessionEntry[], over: Partial<TurnsInput> = {}): TurnsInput => ({ entries, leafId: entries.at(-1)?.id ?? null, entriesRev: 1, liveTools: {}, run: { running: false }, ...over });
const Q = { questions: [{ question: 'Which DB?', header: 'DB', options: [{ label: 'Postgres' }, { label: 'SQLite' }] }] };

describe('turns: ask_user', () => {
  it('a waiting question is the segment ask (not fold work); the fold is not "Working" under it', () => {
    const c = chat();
    c.msg({ role: 'user', content: [{ type: 'text', text: 'set it up' }] });
    c.call('r1', 'read', { path: 'a.ts' });
    c.result('r1', 'read', 'ok');
    c.call('a1', 'ask_user', Q, 'I need one choice first.');
    const [turn] = buildTurns(input(c.entries, { run: { running: true, startedAt: T0 } }));
    expect(turn!.live).toBe(true);
    expect(turn!.segments).toHaveLength(1);
    const seg = turn!.segments[0]!;
    expect(seg.ask).toMatchObject({ id: 'a1', name: 'ask_user', state: 'running' });
    expect(seg.work.some((w) => w.kind === 'tool' && w.tool.name === 'ask_user')).toBe(false);
    expect(seg.answer?.text).toBe('I need one choice first.');
    expect(seg.fold).toMatchObject({ state: 'done', forcedOpen: false, detail: 'read 1 file' });
  });

  it('work after the answer opens the next segment (`ask:<id>`), so the card stays where it was asked', () => {
    const c = chat();
    c.msg({ role: 'user', content: [{ type: 'text', text: 'set it up' }] });
    c.call('a1', 'ask_user', Q);
    c.result('a1', 'ask_user', 'User answered your questions:', { outcome: 'answered', answers: [{ selected: ['Postgres'], skipped: false }] }, 60_000);
    c.call('b1', 'bash', { command: 'make' });
    c.result('b1', 'bash', 'ok');
    c.msg({ role: 'assistant', content: [{ type: 'text', text: 'Done with Postgres.' }], stopReason: 'stop' });
    const [turn] = buildTurns(input(c.entries));
    expect(turn!.segments.map((s) => s.id)).toEqual([c.entries[0]!.id, 'ask:a1']);
    const [first, second] = turn!.segments;
    expect(first!.ask).toMatchObject({ id: 'a1', state: 'ok' });
    expect(first!.fold).toBeUndefined();
    expect(second!.fold?.detail).toBe('ran 1 command');
    expect(second!.answer?.text).toBe('Done with Postgres.');
    // The minute the user took to answer is not counted as the first segment's work.
    expect(second!.fold?.durationMs).toBeLessThan(10_000);
  });

  it('a steer right after the question opens its own segment (no empty ask segment)', () => {
    const c = chat();
    c.msg({ role: 'user', content: [{ type: 'text', text: 'go' }] });
    c.call('a1', 'ask_user', Q);
    c.result('a1', 'ask_user', 'The user cancelled the questions and stopped the turn.', { outcome: 'cancelled' });
    const steer = c.msg({ role: 'user', content: [{ type: 'text', text: 'use sqlite' }] });
    c.msg({ role: 'assistant', content: [{ type: 'text', text: 'ok' }], stopReason: 'stop' });
    const [turn] = buildTurns(input(c.entries));
    expect(turn!.segments.map((s) => s.id)).toEqual([c.entries[0]!.id, steer.id]);
  });

  it('a live ask_user that never streamed its call part still becomes the card', () => {
    const c = chat();
    c.msg({ role: 'user', content: [{ type: 'text', text: 'go' }] });
    const [turn] = buildTurns(
      input(c.entries, { run: { running: true }, liveTools: { z: { id: 'z', name: 'ask_user', args: Q, startedAt: T0, done: false } as never } }),
    );
    expect(turn!.segments.at(-1)!.ask).toMatchObject({ id: 'z', state: 'running' });
    expect(turn!.segments.flatMap((s) => s.work).some((w) => w.kind === 'tool')).toBe(false);
  });
});
