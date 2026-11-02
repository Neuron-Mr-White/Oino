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

export type RunningData = Awaited<ReturnType<typeof fetchRunning>>;
export type RunningCounts = { bash: number; agents: number; sidekick: number; tray: number };

/** The agents the Running views count as live (running or idle) — one definition. */
export function liveAgentsOf(agents: readonly AgentInfo[]): AgentInfo[] {
  return agents.filter((a) => a.state === 'running' || a.state === 'idle');
}

/** The work-tray tabs the Running views list (visible, non-empty) — one definition. */
export function visibleTrayTabsOf(tabs: readonly TrayTab[]): TrayTab[] {
  return tabs.filter((t) => t.visible !== false && t.counts.total > 0);
}

/** E-17: the tabs that count as "running" — a finished history alone is not running work. */
export function runningTrayTabsOf(tabs: readonly TrayTab[]): TrayTab[] {
  return visibleTrayTabsOf(tabs).filter((t) => t.counts.running > 0);
}

/** The summary's counts — the same numbers the Session modal's sections show. */
export function runningCounts(data: RunningData | undefined): RunningCounts {
  return {
    bash: data?.bash.length ?? 0,
    agents: data ? liveAgentsOf(data.agents).length : 0,
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
  for (const agent of liveAgentsOf(data.agents)) rows.push({ kind: 'agent', id: agent.id, title: agent.label, sub: agent.role || agent.owner, state: agent.state });
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
