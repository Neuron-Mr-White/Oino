/**
 * UNI-221: the bridge's `state.waiting` (wake-capable pending work while pi
 * is idle) — set on a push that carries it, CLEARED on a run-state push that
 * omits it (the bridge drops the key once work ends / a turn starts), with
 * its own "Working…" clock.
 */
import { describe, expect, it } from 'vitest';
import { applyMessage, type ChatState } from './store';

const fresh = (): ChatState => ({ conn: 'live', run: { running: false }, entries: [], truncated: false, tools: {}, commands: [], models: [], dialogs: [], queue: [], pending: [], hasOlder: false, loadingOlder: false, seams: [], btwPages: [], outbox: [], work: [], infoGroups: [] }) as unknown as ChatState;

describe('UNI-221 run.waiting', () => {
  it('start → clear on a push without the key, with a waitingSince clock', () => {
    const s = fresh();
    applyMessage(s, { t: 'state', running: false, waiting: 'bg: sleep 30' } as never);
    expect(s.run.waiting).toBe('bg: sleep 30');
    const since = s.waitingSince;
    expect(typeof since).toBe('number');
    applyMessage(s, { t: 'state', running: false, waiting: 'bg: sleep 30 · subagent working' } as never);
    expect(s.waitingSince).toBe(since);
    applyMessage(s, { t: 'state', running: false } as never);
    expect(s.run.waiting).toBeUndefined();
    expect(s.waitingSince).toBeUndefined();
  });
  it('agent_start (running:true, no waiting) clears it; a fusion-only push leaves it', () => {
    const s = fresh();
    applyMessage(s, { t: 'state', running: false, waiting: 'subagent working' } as never);
    applyMessage(s, { t: 'state', fusion: { leads: [] } } as never);
    expect(s.run.waiting).toBe('subagent working');
    applyMessage(s, { t: 'state', running: true } as never);
    expect(s.run.waiting).toBeUndefined();
  });
  it('hello carries it in', () => {
    const s = fresh();
    applyMessage(s, { t: 'hello', session: { id: 's', cwd: '/p' }, state: { running: false, waiting: 'bg: x' }, entries: [], tools: [], commands: [], models: [], dialogs: [], queue: [] } as never);
    expect(s.run.waiting).toBe('bg: x');
    expect(typeof s.waitingSince).toBe('number');
  });
});
