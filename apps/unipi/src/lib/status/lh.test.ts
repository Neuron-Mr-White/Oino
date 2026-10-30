/**
 * `lib/status/lh.ts` (WP-A16): topic parsing, the summary view and the mode
 * commands the section sends.
 */
import { describe, expect, it } from 'vitest';
import { graphCountLine, graphSettled, lhModeCommand, lhSummary, parseLhProgress, parseLhState } from './lh';

const state = (v: unknown) => parseLhState(v);
const progress = (v: unknown) => parseLhProgress(v);

describe('parseLhState / parseLhProgress', () => {
  it('narrows the sticky payloads and drops junk', () => {
    const s = state({ mode: 'graph', paused: undefined, owner: { kind: 'graph', status: 'active' }, lastStop: { kind: 'ralph', at: 1_760_000_000 } });
    expect(s).toEqual({ mode: 'graph', owner: { kind: 'graph', status: 'active' }, lastStop: { kind: 'ralph', at: 1_760_000_000 } });
    expect(state({ mode: 'none', paused: 'goal' })).toEqual({ mode: 'none', paused: 'goal' });
    expect(state(undefined)).toBeUndefined();
    expect(state({ owner: 'graph' })).toBeUndefined();
    expect(state({ mode: 'graph', owner: { kind: 'x', status: 'parked' } })!.owner!.status).toBe('parked');

    const p = progress({
      mode: 'ralph',
      ralph: { name: 'port-status', iteration: 3, checked: 4, total: 7 },
      todos: '4/7 · ▶ next',
      graph: null,
      goal: undefined,
    });
    expect(p).toEqual({ mode: 'ralph', ralph: { name: 'port-status', iteration: 3, checked: 4, total: 7 }, todos: '4/7 · ▶ next' });
    expect(progress({ mode: 'graph', graph: { graphId: 'g1', mode: 'graph', task: 't', counts: { queued: 2, completed: 'x' } } })!.graph).toEqual({
      graphId: 'g1',
      mode: 'graph',
      task: 't',
      counts: { queued: 2 },
    });
    expect(progress('x')).toBeUndefined();
  });
});

describe('lhSummary', () => {
  it('active graph: phase active, settled count, percent', () => {
    const m = lhSummary(state({ mode: 'graph', owner: { kind: 'graph', status: 'active' } }), progress({ mode: 'graph', graph: { mode: 'graph', task: 'Ship it', counts: { completed: 3, failed: 1, running: 2, queued: 1 } } }));
    expect(m!.phase).toBe('active');
    expect(m!.label).toBe('Graph');
    expect(m!.countText).toBe('4/7');
    expect(m!.percent).toBe(57);
  });

  it('parked mode (paused) without an active owner', () => {
    const m = lhSummary(state({ mode: 'none', paused: 'goal', owner: { kind: 'goal', status: 'parked' } }), progress({ mode: 'none' }));
    expect(m).toMatchObject({ mode: 'goal', phase: 'paused' });
  });

  it('selected: mode on but no owner yet', () => {
    const m = lhSummary(state({ mode: 'ralph' }), progress({ mode: 'ralph', ralph: { name: 'x', iteration: 1, checked: 0, total: 5 } }));
    expect(m).toMatchObject({ mode: 'ralph', phase: 'selected', countText: '0/5' });
  });

  it('plain mode with a progress topic of another mode keeps the state view; without topics → undefined', () => {
    expect(lhSummary(state({ mode: 'none' }), progress({ mode: 'none' }))).toBeUndefined();
    expect(lhSummary(undefined, undefined)).toBeUndefined();
    expect(lhSummary(undefined, progress({ mode: 'goal', goal: { objective: 'o', status: 'active', turn: 2, maxTurns: 10 } }))!.phase).toBe('active');
    // a stale topic of the previous mode does not paint numbers onto the new one
    const m = lhSummary(state({ mode: 'goal' }), progress({ mode: 'ralph', ralph: { name: 'x', iteration: 2, checked: 1, total: 3 } }));
    expect(m!.ralph).toBeUndefined();
  });

  it('goal counts turns, ralph counts the checklist', () => {
    const goal = lhSummary(state({ mode: 'goal', owner: { kind: 'goal', status: 'active' } }), progress({ mode: 'goal', goal: { objective: 'Ship A16', status: 'active', turn: 4, maxTurns: 12 } }));
    expect(goal!.countText).toBe('t4');
    expect(goal!.goal!.objective).toBe('Ship A16');
    const ralph = lhSummary(state({ mode: 'ralph', owner: { kind: 'ralph', status: 'active' } }), progress({ mode: 'ralph', ralph: { name: 'loop', iteration: 2, checked: 3, total: 0 } }));
    expect(ralph!.countText).toBe('#2');
    expect(ralph!.percent).toBeUndefined();
  });
});

describe('graph counts + commands', () => {
  it('settled counts every terminal status', () => {
    const c = graphSettled({ queued: 1, running: 2, blocked: 1, completed: 4, failed: 1, stopped: 1, cancelled: 1, superseded: 1 });
    expect(c).toEqual({ settled: 8, total: 12, running: 2, failed: 1 });
    expect(graphCountLine({ queued: 1, running: 2, blocked: 1, completed: 4, failed: 1, stopped: 1, cancelled: 1, superseded: 1 })).toBe('8/12 settled · 2 running · 1 failed');
  });

  it('the mode commands are the extension’s slash commands', () => {
    expect(lhModeCommand('graph', 'stop')).toBe('/graph stop');
    expect(lhModeCommand('ralph', 'resume')).toBe('/ralph resume');
    expect(lhModeCommand('swarm', 'history')).toBe('/swarm history');
  });
});
