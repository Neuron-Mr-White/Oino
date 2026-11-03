/**
 * Running summary data (WP-A16, RULINGS Z-95): the status pane's compact
 * Running summary counts + rows and the Session modal's Running tab read the
 * **same** `fetchRunning` snapshot (components/session/RunningTab), and the
 * counts come from these shared selectors — the summary can never disagree
 * with the full view (A16 acceptance). The capsule reads the cheap sticky
 * topics instead (`unipi:bg:tasks`, `unipi:subagents:list`) — no round trips.
 */
import type { AgentInfo, TrayTab } from '../../components/session/modalData';
import type { TopicValue } from '../session/types';
import type { fetchRunning } from '../../components/session/RunningTab';
import { shownInApp } from '../surfaces';
import { isLive, mergeRuns, SUBAGENTS_EXT, type RunRow } from './subagentRows';

export type RunningData = Awaited<ReturnType<typeof fetchRunning>>;

/** When the app first saw each host agent (`_oino/agents` carries no start time): kept for the
 * app's lifetime, keyed by agent id (a handful). */
const agentSeen = new Map<string, number>();
export function agentSeenAt(agentId: string, now = Date.now()): number {
  let at = agentSeen.get(agentId);
  if (at === undefined) {
    at = now;
    agentSeen.set(agentId, at);
  }
  return at;
}

/** The one Subagents list (records + host agents, one row per run, newest first). */
export function runRowsOf(data: RunningData | undefined): RunRow[] {
  return data ? mergeRuns(data.runs, data.agents, (id) => agentSeenAt(id)) : [];
}

/** The live rows (running or idle) — what the Running counts call "agents". */
export function liveRunRowsOf(data: RunningData | undefined): RunRow[] {
  return runRowsOf(data).filter(isLive);
}
export type RunningCounts = { bash: number; agents: number; sidekick: number; tray: number };

/** The agents the Running views count as live (running or idle) — one definition. */
export function liveAgentsOf(agents: readonly AgentInfo[]): AgentInfo[] {
  return agents.filter((a) => a.state === 'running' || a.state === 'idle');
}

/** The work-tray tabs the Running views list (visible, non-empty, meant for the app) — one
 * definition. The subagents producer's tab is the terminal's (`surfaces: ["tui"]`); its runs are
 * the native Subagents list, so it never shows here (older builds without `surfaces` too). */
export function visibleTrayTabsOf(tabs: readonly TrayTab[]): TrayTab[] {
  return tabs.filter((t) => t.visible !== false && t.counts.total > 0 && shownInApp(t.surfaces) && t.owner !== SUBAGENTS_EXT);
}

/** E-17: the tabs that count as "running" — a finished history alone is not running work. */
export function runningTrayTabsOf(tabs: readonly TrayTab[]): TrayTab[] {
  return visibleTrayTabsOf(tabs).filter((t) => t.counts.running > 0);
}

/** The summary's counts — the same numbers the Session modal's sections show. */
export function runningCounts(data: RunningData | undefined): RunningCounts {
  return {
    bash: data?.bash.length ?? 0,
    agents: liveRunRowsOf(data).length,
    sidekick: data?.sidekick ? 1 : 0,
    tray: data ? runningTrayTabsOf(data.tabs).length : 0,
  };
}

export function runningTotal(counts: RunningCounts): number {
  return counts.bash + counts.agents + counts.sidekick + counts.tray;
}

/** One row of the summary's 3-row list. */
export type RunningRow = { kind: 'bash' | 'agent' | 'sidekick' | 'tray'; id: string; title: string; sub?: string; state: AgentInfo['state'] };

/** The first `limit` rows of running work, most salient first (bash → agents → sidekick → tabs). */
export function runningRows(data: RunningData | undefined, limit = 3): RunningRow[] {
  if (!data) return [];
  const rows: RunningRow[] = [];
  for (const call of data.bash) rows.push({ kind: 'bash', id: call.toolCallId, title: call.command, state: 'running' });
  for (const run of liveRunRowsOf(data)) rows.push({ kind: 'agent', id: run.key, title: run.title, ...(run.profile ? { sub: run.profile } : {}), state: run.status === 'idle' ? 'idle' : 'running' });
  if (data.sidekick) rows.push({ kind: 'sidekick', id: 'fusion', title: data.sidekick.label, sub: data.sidekick.detail, state: 'running' });
  for (const tab of runningTrayTabsOf(data.tabs))
    rows.push({ kind: 'tray', id: tab.id, title: tab.label, sub: tab.counts.running > 0 ? `${tab.counts.running} running · ${tab.counts.total}` : `${tab.counts.total}`, state: 'running' });
  return rows.slice(0, limit);
}

// ── The capsule's cheap count (sticky topics, no round trips) ────────────

export const BG_TASKS_TOPIC = 'unipi:bg:tasks';
export const SUBAGENTS_LIST_TOPIC = 'unipi:subagents:list';

const runningCountIn = (v: unknown): number => {
  const list = (v as { tasks?: unknown[]; records?: unknown[]; agents?: unknown[] } | undefined | null) ?? {};
  const items = Array.isArray(list.tasks) ? list.tasks : Array.isArray(list.records) ? list.records : Array.isArray(list.agents) ? list.agents : [];
  return items.filter((it) => !!it && typeof it === 'object' && (it as { status?: unknown }).status === 'running').length;
};

/** Running bg tasks + running subagents from the session's sticky topics. */
export function topicRunningCount(topics: Record<string, TopicValue>): number {
  return (runningCountIn(topics[BG_TASKS_TOPIC]?.data) > 0 ? 1 : 0) + (runningCountIn(topics[SUBAGENTS_LIST_TOPIC]?.data) > 0 ? 1 : 0);
}

/** G-08: live running-only label from the session's sticky topics (survives app restarts even when `unipi.core`'s strip doesn't fire). */
export function topicRunningTrayLabel(topics: Record<string, TopicValue>): string | undefined {
  const bg = runningCountIn(topics[BG_TASKS_TOPIC]?.data);
  const sa = runningCountIn(topics[SUBAGENTS_LIST_TOPIC]?.data);
  const parts: string[] = [];
  if (bg > 0) parts.push(`Background tasks ${bg} running`);
  if (sa > 0) parts.push(`Subagents ${sa} running`);
  return parts.length ? parts.join(' · ') : undefined;
}
