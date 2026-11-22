/**
 * `lib/status/running.ts` (WP-A16, Z-95): the shared Running selectors — the
 * summary's counts must equal the Session modal's (acceptance).
 */
import { describe, expect, it } from 'vitest';
import type { AgentInfo, RunningBash, TrayTab } from '../../components/session/modalData';
import { liveAgentsOf, runningCounts, runningRows, runningTotal, topicRunningCount, visibleTrayTabsOf, type RunningData } from './running';

const bash = (command: string): RunningBash => ({ toolCallId: `tc-${command}`, command, startedAt: 1 });
const agent = (id: string, state: AgentInfo['state']): AgentInfo => ({ id, label: `Agent ${id}`, owner: 'unipi.subagents', state, depth: 1 });
const tray = (id: string, total: number, running = 0, visible = true): TrayTab => ({ id, label: id, order: 10, counts: { total, running }, visible });

const data = (over: Partial<RunningData> = {}): RunningData => ({
  bash: [bash('cargo test')],
  agents: [agent('a1', 'running'), agent('a2', 'idle'), agent('a3', 'hibernated'), agent('a4', 'closed')],
  tabs: [tray('bg', 2, 1), tray('dream', 1, 0), tray('hidden', 3, 3, false), tray('empty', 0)],
  ...over,
});

describe('shared selectors (summary ≡ Session modal)', () => {
  it('liveAgentsOf / visibleTrayTabsOf are the one definition', () => {
    expect(liveAgentsOf(data().agents).map((a) => a.id)).toEqual(['a1', 'a2']);
    expect(visibleTrayTabsOf(data().tabs).map((t) => t.id)).toEqual(['bg', 'dream']);
  });

  it('runningCounts mirrors the modal sections', () => {
    expect(runningCounts(data())).toEqual({ bash: 1, agents: 2, sidekick: 0, tray: 1 });
    expect(runningTotal(runningCounts(data()))).toBe(4);
    expect(runningCounts(undefined)).toEqual({ bash: 0, agents: 0, sidekick: 0, tray: 0 });
    expect(runningCounts(data({ sidekick: { label: 'Fusion', detail: 'Handoff running' } })).sidekick).toBe(1);
  });

  it('rows: most salient first, capped at 3, +N more left for the link', () => {
    const rows = runningRows(data());
    expect(rows.map((r) => r.kind)).toEqual(['bash', 'agent', 'agent']);
    expect(rows[0]!.title).toBe('cargo test');
    expect(runningRows(data(), 5)).toHaveLength(4);
    expect(runningRows(undefined)).toEqual([]);
    // tabs route to the modal too (G-20) when the front rows are quiet
    expect(runningRows(data({ bash: [], agents: [] })).map((r) => r.kind)).toEqual(['tray']);
  });
});

describe('topicRunningCount (the capsule)', () => {
  const topics = (v: unknown) => ({ 'unipi:bg:tasks': { data: v, sticky: true, publisher: '' } });

  it('counts running bg tasks and subagent records', () => {
    expect(topicRunningCount(topics({ tasks: [{ status: 'running' }, { status: 'completed' }] }))).toBe(1);
    expect(topicRunningCount({ 'unipi:subagents:list': { data: { records: [{ status: 'running' }] }, sticky: true, publisher: '' } })).toBe(1);
    expect(topicRunningCount(topics({ tasks: [{ status: 'running' }, { status: 'running' }] }))).toBe(2);
    expect(topicRunningCount({})).toBe(0);
    expect(topicRunningCount(topics({ tasks: [{ status: 'completed' }] }))).toBe(0);
    expect(topicRunningCount(topics(undefined))).toBe(0);
  });
});
