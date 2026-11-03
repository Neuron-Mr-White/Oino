/**
 * The Running tab's one Subagents list ([running-tab], owner 2026-11-03): the
 * subagent run records (`unipi:subagents:list` / `subagents.list`, structured)
 * merged with the host's nested-agent tree (`_oino/agents`). Before, the tab
 * showed the same runs twice — once as the agent tree, once as the subagents
 * extension's TUI work-tray pane ('RUN ' badges, run-on stat lines). Now:
 * one row per run; a record and its live host agent (same child session file)
 * are one row; agents no record knows (btw, a workflow child, an older
 * subagents build) are rows of their own.
 *
 * Statuses come from the record's structured `status` / the agent's `state`,
 * never from a label text. Pure helpers (no Solid) so the views and tests share them.
 */
import type { AgentInfo, RunUsage, SubagentRun } from '../../components/session/modalData';
import { modelDisplayName } from '../model/displayName';

/** A row's status (the timeline AgentCard's icon set: spinner / check / x / ban; idle = moon). */
export type RunRowStatus = 'running' | 'idle' | 'done' | 'failed' | 'cancelled';

export type RunRow = {
  /** Stable key (`run:<record id>` / `agent:<host agent id>`). */
  key: string;
  kind: 'record' | 'agent';
  /** Record id (subagents services) or host agent id. */
  id: string;
  /** The live host agent (`_oino/agent_*`), when one is open. */
  agentId?: string;
  title: string;
  profile?: string;
  model?: string;
  status: RunRowStatus;
  /** Runs without the parent waiting on it. */
  background: boolean;
  startedAt?: number;
  endedAt?: number;
  toolCalls?: number;
  usage?: RunUsage;
  error?: string;
};

/** The extension whose runs the records describe (its agents are the records' children). */
export const SUBAGENTS_EXT = 'unipi.subagents';

const RECORD_STATUS: Record<SubagentRun['status'], RunRowStatus> = { running: 'running', completed: 'done', failed: 'failed', cancelled: 'cancelled' };
// Hibernated agents are parked (not running, nothing to stop): listed with the finished ones.
const AGENT_STATUS: Record<AgentInfo['state'], RunRowStatus> = { running: 'running', idle: 'idle', hibernated: 'done', closed: 'done' };

export const ROW_STATUS_LABEL: Record<RunRowStatus, string> = { running: 'Running', idle: 'Idle', done: 'Done', failed: 'Failed', cancelled: 'Cancelled' };

/** A provider error as the record keeps it (`500: {"message":"…"}`) → `… (500)`; other text as is. */
export function errorLine(error: string): string {
  const m = /^(\d{3}):\s*(\{[\s\S]*\})\s*$/.exec(error.trim());
  if (!m) return error;
  try {
    const body = JSON.parse(m[2]!) as { message?: unknown; error?: { message?: unknown } };
    const msg = typeof body.message === 'string' ? body.message : typeof body.error?.message === 'string' ? body.error.message : undefined;
    return msg ? `${msg} (${m[1]})` : error;
  } catch {
    return error;
  }
}

/** Live = can be stopped (running or idle). */
export const isLive = (r: RunRow): boolean => r.status === 'running' || r.status === 'idle';

/**
 * One row per run, newest first. `seenAt(agentId)` dates agents the records
 * do not cover (`_oino/agents` carries no start time). `records` undefined =
 * no record source (topic not there, an older extension): every nested agent is a row.
 */
export function mergeRuns(records: readonly SubagentRun[] | undefined, agents: readonly AgentInfo[], seenAt: (agentId: string) => number): RunRow[] {
  const byFile = new Map<string, AgentInfo>();
  for (const a of agents) if (a.sessionFile) byFile.set(a.sessionFile, a);
  const linked = new Set<string>();
  const rows: RunRow[] = [];
  for (const r of records ?? []) {
    const agent = r.sessionFile ? byFile.get(r.sessionFile) : undefined;
    if (agent) linked.add(agent.id);
    const live = r.status === 'running';
    rows.push({
      key: `run:${r.id}`,
      kind: 'record',
      id: r.id,
      ...(agent && live ? { agentId: agent.id } : {}),
      title: r.title,
      ...(r.profile ? { profile: r.profile } : {}),
      ...(r.model ? { model: r.model } : {}),
      status: RECORD_STATUS[r.status],
      background: r.background,
      ...(r.startedAt ? { startedAt: r.startedAt } : {}),
      ...(r.endedAt ? { endedAt: r.endedAt } : {}),
      toolCalls: r.toolCalls,
      ...(r.usage ? { usage: r.usage } : {}),
      ...(r.error ? { error: errorLine(r.error) } : {}),
    });
  }
  for (const a of agents) {
    if (linked.has(a.id)) continue;
    // The records cover every run of the subagents extension (a child's own agent shows in its record).
    if (records && a.owner === SUBAGENTS_EXT) continue;
    rows.push({
      key: `agent:${a.id}`,
      kind: 'agent',
      id: a.id,
      ...(a.state === 'running' || a.state === 'idle' ? { agentId: a.id } : {}),
      title: a.label,
      ...(a.role && a.role !== a.owner ? { profile: a.role } : {}),
      status: AGENT_STATUS[a.state],
      background: false,
      startedAt: seenAt(a.id),
    });
  }
  return rows.sort((x, y) => (y.startedAt ?? 0) - (x.startedAt ?? 0));
}

/** The filter chips (sidebar subagent tree pattern). `running` includes idle (live). */
export type RunFilter = 'all' | 'running' | 'done' | 'failed' | 'cancelled';

export const RUN_FILTERS: ReadonlyArray<{ id: RunFilter; label: string }> = [
  { id: 'all', label: 'All' },
  { id: 'running', label: 'Running' },
  { id: 'done', label: 'Done' },
  { id: 'failed', label: 'Failed' },
  { id: 'cancelled', label: 'Cancelled' },
];

export function matchesFilter(r: RunRow, f: RunFilter): boolean {
  if (f === 'all') return true;
  if (f === 'running') return isLive(r);
  return r.status === f;
}

export function filterCounts(rows: readonly RunRow[]): Record<RunFilter, number> {
  const c: Record<RunFilter, number> = { all: rows.length, running: 0, done: 0, failed: 0, cancelled: 0 };
  for (const r of rows) {
    if (isLive(r)) c.running += 1;
    else if (r.status === 'done' || r.status === 'failed' || r.status === 'cancelled') c[r.status] += 1;
  }
  return c;
}

/** `provider/id` → a short readable name (`gateway/cc/claude-opus-5-5` → "Claude Opus 5.5"); the full id is the tooltip. */
export function shortModel(model: string | undefined): string {
  const m = (model ?? '').trim();
  if (!m) return '';
  const slash = m.indexOf('/');
  const id = slash > 0 ? m.slice(slash + 1) : m;
  return modelDisplayName({ id }) || id;
}

/** Profile id → chip text (`subagent_explore` → "Explore"). */
export function profileChip(profile: string | undefined): string {
  const base = (profile ?? '').replace(/^subagent_/, '');
  return base ? base.charAt(0).toUpperCase() + base.slice(1) : '';
}

/** `845` / `12.4k` / `88k` / `1.2M` — token counts for the stat line. */
export function compactTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(n >= 10_000_000 ? 0 : 1)}M`;
  if (n >= 10_000) return `${Math.round(n / 1000)}k`;
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
  return String(n);
}

/** The muted second line's parts: tools, tokens in/out, cost (each its own item, no run-on string). */
export function statParts(r: RunRow): Array<{ key: 'tools' | 'tokens' | 'cost'; text: string; title: string }> {
  const out: Array<{ key: 'tools' | 'tokens' | 'cost'; text: string; title: string }> = [];
  if (r.toolCalls !== undefined) out.push({ key: 'tools', text: `${r.toolCalls} tool${r.toolCalls === 1 ? '' : 's'}`, title: 'Tool calls' });
  const u = r.usage;
  if (u && (u.input > 0 || u.output > 0)) out.push({ key: 'tokens', text: `${compactTokens(u.input)} in · ${compactTokens(u.output)} out`, title: `Tokens: ${u.input.toLocaleString()} in, ${u.output.toLocaleString()} out` });
  if (u && u.cost > 0) out.push({ key: 'cost', text: u.cost < 0.01 ? `$${u.cost.toFixed(4)}` : `$${u.cost.toFixed(2)}`, title: 'Cost' });
  return out;
}

/** Elapsed ms of a row at `now` (running rows tick; finished rows are fixed). */
export function rowDurationMs(r: RunRow, now: number): number | undefined {
  if (!r.startedAt) return undefined;
  return Math.max(0, (isLive(r) ? now : (r.endedAt ?? now)) - r.startedAt);
}
