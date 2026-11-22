/** [OIN-98 bug 2] A question must not leave later turns stuck on "Working". */
import { describe, expect, it } from 'vitest';
import { buildTurns, createTurnsIndex, type Turn, type TurnsInput } from './turns';
import type { AgentMessage, SessionEntry } from './types';

const T0 = Date.UTC(2026, 10, 12, 9, 0, 0);
const Q = { questions: [{ question: 'Which DB?', header: 'DB', options: [{ label: 'Postgres' }, { label: 'SQLite' }] }] };

function chat() {
  const entries: SessionEntry[] = [];
  let t = T0;
  const msg = (message: AgentMessage, dt = 1000) => {
    t += dt;
    entries.push({ type: 'message', message: { timestamp: t, ...message }, id: `e${entries.length + 1}`, parentId: entries.at(-1)?.id ?? null, timestamp: new Date(t).toISOString() } as SessionEntry);
  };
  const user = (text: string) => msg({ role: 'user', content: [{ type: 'text', text }] });
  const ask = () => msg({ role: 'assistant', content: [{ type: 'toolCall', id: 'a1', name: 'ask_user', arguments: Q }], stopReason: 'toolUse' });
  const answer = () => msg({ role: 'toolResult', toolCallId: 'a1', toolName: 'ask_user', content: [{ type: 'text', text: 'answered' }], isError: false, details: { outcome: 'answered', answers: [{ selected: ['Postgres'], skipped: false }] } }, 30_000);
  const reply = (text: string) => msg({ role: 'assistant', content: [{ type: 'text', text }], stopReason: 'stop' });
  return { entries, user, ask, answer, reply };
}

const input = (entries: SessionEntry[], running: boolean, rev: number): TurnsInput => ({
  entries,
  leafId: entries.at(-1)?.id ?? null,
  entriesRev: rev,
  liveTools: {},
  run: running ? { running: true, startedAt: T0 } : { running: false },
});

/** No turn and no segment of it reads "Working". */
const settled = (t: Turn) => !t.live && t.segments.every((s) => !s.live && s.fold?.state !== 'running' && s.fold?.label !== 'Working');

describe('turns: a question does not wedge later turns in "Working"', () => {
  it('requested -> answered -> run ends -> new prompt -> reply -> run ends (incremental index)', () => {
    const c = chat();
    const idx = createTurnsIndex();
    c.user('set it up');
    c.ask();
    let turns = idx.update(input(c.entries, true, 1));
    expect(turns[0]!.segments[0]!.fold?.label).not.toBe('Working');
    c.answer();
    c.reply('Done with Postgres.');
    turns = idx.update(input(c.entries, true, 2));
    turns = idx.update(input(c.entries, false, 2));
    expect(turns.every(settled)).toBe(true);
    c.user('thanks, now add auth');
    turns = idx.update(input(c.entries, true, 3));
    expect(turns).toHaveLength(2);
    expect(turns[0]!.live).toBe(false);
    c.reply('Auth added.');
    turns = idx.update(input(c.entries, true, 4));
    turns = idx.update(input(c.entries, false, 4));
    expect(turns).toHaveLength(2);
    expect(turns.every(settled)).toBe(true);
    expect(turns[1]!.segments.at(-1)!.answer?.text).toBe('Auth added.');
    expect(turns[1]!.segments.at(-1)!.fold?.label ?? 'Worked').toMatch(/^Worked/);
    expect(idx.update(input(c.entries, false, 4))).toEqual(buildTurns(input(c.entries, false, 4)));
  });

  it('a question left unanswered when its run ends does not swallow the next prompts into its turn', () => {
    const c = chat();
    c.user('set it up');
    c.ask();
    // Run ended (stopped / host restart) with the call never answered; the owner moves on.
    c.user('forget it, do something else');
    c.reply('Sure.');
    const turns = buildTurns(input(c.entries, false, 1));
    expect(turns).toHaveLength(2);
    expect(turns.every(settled)).toBe(true);
    expect(turns[0]!.segments[0]!.ask?.state).toBe('stopped');
    expect(turns[1]!.segments.at(-1)!.answer?.text).toBe('Sure.');
  });

  it('a question still pending while later events arrive: only its own turn waits', () => {
    const c = chat();
    c.user('set it up');
    c.ask();
    const idx = createTurnsIndex();
    let turns = idx.update(input(c.entries, true, 1));
    expect(turns[0]!.segments[0]!.ask?.state).toBe('running');
    expect(turns[0]!.segments[0]!.fold?.state).not.toBe('running');
    // The run is cut off, then the owner sends a new prompt; the new run replies.
    turns = idx.update(input(c.entries, false, 1));
    c.user('never mind');
    c.reply('Ok.');
    turns = idx.update(input(c.entries, true, 2));
    turns = idx.update(input(c.entries, false, 2));
    expect(turns).toHaveLength(2);
    expect(turns.every(settled)).toBe(true);
  });

  it('a note typed while the question is live (run still going) stays a steer of its own turn', () => {
    const c = chat();
    c.user('set it up');
    c.ask();
    c.user('prefer postgres');
    const [turn, ...rest] = buildTurns(input(c.entries, true, 1));
    expect(rest).toHaveLength(0);
    expect(turn!.segments.map((s) => !!s.steer)).toEqual([false, true]);
    c.answer();
    c.reply('Done.');
    expect(buildTurns(input(c.entries, false, 2))).toHaveLength(1);
  });
});
