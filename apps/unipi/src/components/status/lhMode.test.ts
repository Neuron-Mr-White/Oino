/** UNI-269: the working-mode chip/popover/list-badge helpers, store wiring and list fields. */
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createRoot } from 'solid-js';
import { isLhModeState, listModeBadge, modeChipCount, modeErrorText, modeLabel, modeLiveText, modePercent, modeView, switchHint, type LhModeState } from './lhMode';
import type { LhProgress, LhRun } from './lhProgress';
import { createChat } from '../../lib/chat/store';
import { setChatTransport, type ChatTransport } from '../../lib/session/link';
import { chatGroups, withLive } from '../../lib/session/list';
import type { SessionListData } from '../../lib/session/link';

const counts = (total: number, done: number, running = 0, failed = 0) => ({ total, done, running, failed, queued: total - done - running - failed });
const graphRun: LhRun = { mode: 'graph', title: 'Repo health', status: 'running', items: [], counts: counts(7, 3, 2) };
const ralphRun: LhRun = { mode: 'ralph', title: 'chores', status: 'running', items: [], counts: counts(0, 0), ralph: { name: 'chores', iteration: 4, maxIterations: 10, checked: 5, total: 12 } };
const goalRun: LhRun = { mode: 'goal', title: 'ship it', status: 'running', items: [], counts: counts(0, 0), goal: { objective: 'ship it', status: 'active', turn: 3, maxTurns: 0, percent: 45 } };
const prog = (run: LhRun | undefined, last?: LhRun): LhProgress => ({ v: 1, mode: run?.status === 'running' ? run.mode : 'none', current: run, last, log: [], updatedAt: 1 });

describe('modeView', () => {
  it('plain: nothing (chip hidden)', () => {
    expect(modeView({ mode: 'none' }, undefined)).toBeUndefined();
    expect(modeView(undefined, undefined)).toBeUndefined();
    expect(modeView({ mode: 'none' }, prog(undefined, graphRun))).toBeUndefined();
  });

  it('a sticky mode with no work yet is "selected"; with its owner it is "active" + the run', () => {
    expect(modeView({ mode: 'graph' }, undefined)).toMatchObject({ mode: 'graph', label: 'Graph', phase: 'selected', live: false });
    const v = modeView({ mode: 'graph', owner: { kind: 'graph', status: 'active' } }, prog(graphRun));
    expect(v).toMatchObject({ mode: 'graph', phase: 'active', live: true });
    expect(v?.run?.title).toBe('Repo health');
  });

  it("the progress run of another mode is not this mode's run", () => {
    expect(modeView({ mode: 'swarm' }, prog(graphRun))?.run).toBeUndefined();
  });

  it('a parked owner while plain is "paused"', () => {
    expect(modeView({ mode: 'none', paused: 'goal', owner: { kind: 'goal', status: 'parked' } }, undefined)).toMatchObject({ mode: 'goal', label: 'Goal', phase: 'paused' });
  });

  it('older bridge (no lhMode): a running or paused run stands in', () => {
    expect(modeView(undefined, prog(ralphRun))).toMatchObject({ mode: 'ralph', phase: 'active', live: true });
    expect(modeView(undefined, prog({ ...ralphRun, status: 'paused' }))).toMatchObject({ mode: 'ralph', phase: 'paused' });
    expect(modeView(undefined, prog(undefined, graphRun))).toBeUndefined();
  });

  it('unknown future modes pass through, capitalised', () => {
    expect(modeView({ mode: 'pipeline' }, undefined)).toMatchObject({ mode: 'pipeline', label: 'Pipeline', phase: 'selected' });
    expect(modeLabel('none')).toBe('Plain');
  });
});

describe('counts and live text', () => {
  it('chip count per mode', () => {
    expect(modeChipCount(modeView({ mode: 'graph' }, prog(graphRun)))).toBe('3/7');
    expect(modeChipCount(modeView({ mode: 'ralph' }, prog(ralphRun)))).toBe('5/12');
    expect(modeChipCount(modeView({ mode: 'ralph' }, prog({ ...ralphRun, ralph: { ...ralphRun.ralph!, total: 0, checked: 0 } })))).toBe('#4');
    expect(modeChipCount(modeView({ mode: 'goal' }, prog(goalRun)))).toBe('45%');
    expect(modeChipCount(modeView({ mode: 'goal' }, prog({ ...goalRun, goal: { ...goalRun.goal!, percent: undefined } })))).toBeUndefined();
    expect(modeChipCount(modeView({ mode: 'swarm' }, prog({ ...graphRun, mode: 'swarm', counts: counts(0, 0) })))).toBeUndefined();
    expect(modeChipCount(undefined)).toBeUndefined();
  });

  it('popover live line + bar', () => {
    expect(modeLiveText(graphRun)).toBe('3/7 settled · 2 running');
    expect(modeLiveText({ ...graphRun, counts: counts(4, 2, 0, 1) })).toBe('3/4 settled · 1 failed');
    expect(modeLiveText({ ...graphRun, counts: counts(0, 0) })).toBe('No items yet');
    expect(modeLiveText(ralphRun)).toBe('Iteration 4/10 · 5/12 checked');
    expect(modeLiveText({ ...ralphRun, ralph: { ...ralphRun.ralph!, maxIterations: 0, total: 0 } })).toBe('Iteration 4');
    expect(modeLiveText(goalRun)).toBe('Turn 3 · 45% done');
    expect(modePercent(graphRun)).toBe(43);
    expect(modePercent(ralphRun)).toBe(42);
    expect(modePercent(goalRun)).toBe(45);
    expect(modePercent({ ...graphRun, counts: counts(0, 0) })).toBeUndefined();
  });

  it('switch hints only while work runs', () => {
    const active = modeView({ mode: 'graph', owner: { kind: 'graph', status: 'active' } }, prog(graphRun));
    expect(switchHint(active, 'none')).toBe('Stops the graph');
    expect(switchHint(active, 'goal')).toBe('Pauses the graph');
    expect(switchHint(active, 'graph')).toBeUndefined();
    expect(switchHint(modeView({ mode: 'ralph', owner: { kind: 'ralph-loop', status: 'active' } }, prog(ralphRun)), 'none')).toBe('Stops the loop');
    expect(switchHint(modeView({ mode: 'graph' }, undefined), 'none')).toBeUndefined();
    expect(switchHint(undefined, 'goal')).toBeUndefined();
  });

  it('error text: an older bridge reads as "needs a newer unipi"', () => {
    expect(modeErrorText('unknown type set_lh_mode')).toMatch(/newer unipi/);
    expect(modeErrorText('A graph is already parked')).toBe('A graph is already parked');
  });

  it('list badge', () => {
    expect(listModeBadge(undefined, undefined)).toBeUndefined();
    expect(listModeBadge('none', undefined)).toBeUndefined();
    expect(listModeBadge('swarm', 'goal')).toEqual({ mode: 'swarm', label: 'Swarm', paused: false });
    expect(listModeBadge(undefined, 'graph')).toEqual({ mode: 'graph', label: 'Graph · paused', paused: true });
  });

  it('isLhModeState', () => {
    expect(isLhModeState({ mode: 'goal' })).toBe(true);
    expect(isLhModeState({ mode: 3 })).toBe(false);
    expect(isLhModeState(null)).toBe(false);
  });
});

describe('chat store — lh_mode / set_lh_mode (UNI-269)', () => {
  let sent: Array<Record<string, unknown>>;
  let push: (m: Record<string, unknown>) => void;
  beforeEach(() => {
    sent = [];
    const transport: ChatTransport = {
      list: async () => ({ sessions: [] }),
      open: (_h, _p, onLine, onState) => {
        push = (m) => onLine(JSON.stringify(m));
        queueMicrotask(() => onState('open'));
        return { send: async (line) => void sent.push(JSON.parse(line)), close: () => {} };
      },
    };
    setChatTransport(transport);
  });
  afterEach(() => setChatTransport(undefined));
  const hello = (extra: Record<string, unknown> = {}) => ({ t: 'hello', session: { id: 's', cwd: '/p' }, state: { running: false }, entries: [], commands: [], models: [], dialogs: [], queue: [], ...extra });
  const run = (body: (chat: ReturnType<typeof createChat>) => Promise<void>) =>
    new Promise<void>((resolve, reject) =>
      createRoot((dispose) => {
        const chat = createChat('h', 's1');
        void (async () => {
          await new Promise((r) => setTimeout(r, 0));
          try {
            await body(chat);
            resolve();
          } catch (e) {
            reject(e);
          } finally {
            dispose();
          }
        })();
      }),
    );

  it('hello.lhMode seeds it; lh_mode pushes replace it live; garbage clears it', () =>
    run(async (chat) => {
      push(hello({ lhMode: { mode: 'graph', owner: { kind: 'graph', status: 'active' } } }));
      expect(chat.state.lhMode?.mode).toBe('graph');
      push({ t: 'lh_mode', mode: { mode: 'none', paused: 'graph' } satisfies LhModeState });
      expect(chat.state.lhMode).toEqual({ mode: 'none', paused: 'graph' });
      push({ t: 'lh_mode', mode: { nope: 1 } });
      expect(chat.state.lhMode).toBeUndefined();
      push(hello({ lhMode: { mode: 'goal' } }));
      expect(chat.state.lhMode?.mode).toBe('goal');
      push(hello());
      expect(chat.state.lhMode).toBeUndefined();
    }));

  it('setLhMode sends set_lh_mode; resolves on ack, rejects with the reason', () =>
    run(async (chat) => {
      push(hello({ lhMode: { mode: 'none' } }));
      const ok = chat.setLhMode('swarm');
      const req = sent.find((m) => m.t === 'set_lh_mode');
      expect(req).toMatchObject({ t: 'set_lh_mode', mode: 'swarm' });
      push({ t: 'ack', ref: req!.ref });
      await expect(ok).resolves.toBeUndefined();
      const bad = chat.setLhMode('goal');
      const req2 = sent.filter((m) => m.t === 'set_lh_mode').at(-1)!;
      push({ t: 'error', message: 'A graph is already parked', ref: req2.ref });
      await expect(bad).rejects.toThrow('A graph is already parked');
    }));
});

describe('chat list — mode badge fields (UNI-269)', () => {
  const list = (card: Record<string, unknown>): SessionListData => ({ sessions: [{ session_id: 's7', cwd: '/home/u/p', run_state: 'idle' as const, ...card }] });

  it('carries lh_mode / lh_paused onto the row', () => {
    const [row] = chatGroups('h', list({ lh_mode: 'graph', lh_paused: 'goal' }))[0]!.items;
    expect(row).toMatchObject({ lhMode: 'graph', lhPaused: 'goal' });
    const [plain] = chatGroups('h', list({}))[0]!.items;
    expect('lhMode' in plain! || 'lhPaused' in plain!).toBe(false);
  });

  it("the open chat's live mode beats the polled one (both ways)", () => {
    const [row] = chatGroups('h', list({ lh_mode: 'graph' }))[0]!.items;
    expect(withLive(row!, { hostId: 'h', session: 's7', running: false, lhMode: { mode: 'none' } }).lhMode).toBeUndefined();
    expect(withLive(row!, { hostId: 'h', session: 's7', running: false, lhMode: { mode: 'swarm', paused: 'graph' } })).toMatchObject({ lhMode: 'swarm', lhPaused: 'graph' });
    expect(withLive(row!, { hostId: 'h', session: 's7', running: false }).lhMode).toBe('graph');
    expect(withLive(row!, { hostId: 'h', session: 's8', running: false, lhMode: { mode: 'none' } }).lhMode).toBe('graph');
  });
});
