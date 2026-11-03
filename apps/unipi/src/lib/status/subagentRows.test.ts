/**
 * [running-tab] the one Subagents list: records + host agents merged into one
 * row per run, structured statuses (never TUI badge text), chips, stat parts.
 */
import { describe, expect, it } from 'vitest';
import type { AgentInfo, SubagentRun } from '../../components/session/modalData';
import { parseSubagentRuns, parseTrayTabs } from '../../components/session/modalData';
import { compactTokens, filterCounts, matchesFilter, mergeRuns, profileChip, rowDurationMs, shortModel, statParts } from './subagentRows';
import { visibleTrayTabsOf } from './running';

const rec = (id: string, status: SubagentRun['status'], over: Partial<SubagentRun> = {}): SubagentRun => ({
  id, title: `Run ${id}`, profile: 'subagent_explore', model: 'zai/glm-5.3', status, background: false, startedAt: 1000, toolCalls: 0, depth: 1, sessionFile: `/s/${id}.jsonl`, ...over,
});
const agent = (id: string, state: AgentInfo['state'], over: Partial<AgentInfo> = {}): AgentInfo => ({ id, label: `Agent ${id}`, owner: 'unipi.subagents', state, depth: 1, ...over });
const seen = () => 5000;

describe('parseSubagentRuns', () => {
  it('reads the topic ({records}) and the service reply (list); status from the structured field', () => {
    const raw = { id: 'a1', title: 'Map', profile: 'subagent_explore', model: 'p/m', status: 'completed', background: true, startedAt: 10, endedAt: 20, toolCalls: 3, usage: { input: 5, output: 6, cost: 0.5 } };
    expect(parseSubagentRuns({ records: [raw] })).toEqual(parseSubagentRuns([raw]));
    const [r] = parseSubagentRuns([raw])!;
    expect(r).toMatchObject({ id: 'a1', status: 'completed', background: true, endedAt: 20, toolCalls: 3, usage: { input: 5, output: 6, cacheRead: 0, cacheWrite: 0, cost: 0.5 } });
    expect(parseSubagentRuns(undefined)).toBeUndefined();
    expect(parseSubagentRuns({})).toBeUndefined();
    // A TUI badge never becomes a status: unknown words fall back to running, the label is not read.
    expect(parseSubagentRuns([{ id: 'x', status: 'RUN ', badge: 'DONE' }])![0]!.status).toBe('running');
    expect(parseSubagentRuns([{ id: 'y', task: 'a long task text' }])![0]!.title).toBe('a long task text');
  });

  it('tray tabs carry their surfaces; tui-only and the subagents producer never list in the app', () => {
    const tabs = parseTrayTabs([
      { id: 'subagents', label: 'Subagents', order: 1, counts: { total: 2, running: 1 }, surfaces: ['tui'] },
      { id: 'bg', label: 'Background tasks', order: 0, counts: { total: 1, running: 1 } },
      { id: 'x', label: 'GUI only', order: 2, counts: { total: 1, running: 0 }, surfaces: ['gui'] },
    ]);
    expect(tabs[0]!.surfaces).toEqual(['tui']);
    expect(visibleTrayTabsOf(tabs).map((t) => t.id)).toEqual(['bg', 'x']);
    // An older subagents build without `surfaces`: still filtered by its owner.
    expect(visibleTrayTabsOf([{ ...tabs[0]!, surfaces: undefined, owner: 'unipi.subagents' }])).toEqual([]);
  });
});

describe('mergeRuns', () => {
  it('one row per run: a record and its live host agent are one row; others are their own rows; newest first', () => {
    const rows = mergeRuns(
      [rec('r1', 'running', { startedAt: 3000 }), rec('r2', 'completed', { startedAt: 2000, endedAt: 2600 }), rec('r3', 'failed', { startedAt: 1000, error: 'boom' }), rec('r4', 'cancelled', { startedAt: 500 })],
      [agent('ag-1', 'running', { sessionFile: '/s/r1.jsonl' }), agent('ag-2', 'idle', { depth: 2 }), agent('btw-1', 'running', { owner: 'unipi.btw', role: 'btw' })],
      seen,
    );
    expect(rows.map((r) => r.key)).toEqual(['agent:btw-1', 'run:r1', 'run:r2', 'run:r3', 'run:r4']);
    expect(rows.map((r) => r.status)).toEqual(['running', 'running', 'done', 'failed', 'cancelled']);
    expect(rows[1]!.agentId).toBe('ag-1');
    expect(rows[2]!.agentId).toBeUndefined();
    expect(rows[3]!.error).toBe('boom');
  });

  it('no record source (older extension): every nested agent is a row; idle is live, hibernated/closed are not', () => {
    const rows = mergeRuns(undefined, [agent('a', 'running'), agent('b', 'idle'), agent('c', 'hibernated'), agent('d', 'closed')], seen);
    expect(rows.map((r) => r.status)).toEqual(['running', 'idle', 'done', 'done']);
    expect(filterCounts(rows)).toEqual({ all: 4, running: 2, done: 2, failed: 0, cancelled: 0 });
    expect(rows.filter((r) => matchesFilter(r, 'running')).map((r) => r.id)).toEqual(['a', 'b']);
  });
});

describe('row text', () => {
  it('short model, profile chip, compact tokens, stat parts, duration', () => {
    expect(shortModel('omniroute/cc/claude-opus-5-5')).not.toContain('/');
    expect(shortModel('')).toBe('');
    expect(profileChip('subagent_explore')).toBe('Explore');
    expect(profileChip('reviewer')).toBe('Reviewer');
    expect(compactTokens(238)).toBe('238');
    expect(compactTokens(1234)).toBe('1.2k');
    expect(compactTokens(88_000)).toBe('88k');
    expect(compactTokens(1_500_000)).toBe('1.5M');
    const [r] = mergeRuns([rec('r', 'completed', { toolCalls: 122, usage: { input: 238, output: 88_000, cacheRead: 0, cacheWrite: 0, cost: 14.23 }, startedAt: 1, endedAt: 1_596_001 })], [], seen);
    expect(statParts(r!).map((p) => p.text)).toEqual(['122 tools', '238 in · 88k out', '$14.23']);
    expect(rowDurationMs(r!, 9_999_999)).toBe(1_596_000);
    const [live] = mergeRuns([rec('l', 'running', { startedAt: 1000, toolCalls: 1 })], [], seen);
    expect(rowDurationMs(live!, 4000)).toBe(3000);
    expect(statParts(live!).map((p) => p.text)).toEqual(['1 tool']);
  });
});
