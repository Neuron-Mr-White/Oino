/**
 * Long-horizon status data (WP-A16, §3.10): the LH section of the status
 * pane and the collapsed capsule read the extension's sticky bus topics —
 * `unipi:long-horizon:state` (mode / owner) and `unipi:long-horizon:progress`
 * (goal / ralph / graph / todos line) — subscribed through the session's
 * `_oino/topics` (ui-protocol §8: "LH topics / `_oino/topics`"). Pure parsing
 * and view-shaping only; the components own the fetches and commands.
 */
import { modeLabel } from '../../components/status/lhMode';

export const LH_EXTENSION_ID = 'unipi.long-horizon';
export const LH_STATE_TOPIC = 'unipi:long-horizon:state';
export const LH_PROGRESS_TOPIC = 'unipi:long-horizon:progress';

/** `LH_STATE` (sticky): the working mode and its owner, as long-horizon publishes it. */
export type LhState = {
  mode: string;
  /** A parked owner while the session runs plain. */
  paused?: string;
  owner?: { kind: string; status: 'active' | 'parked' };
  lastStop?: { kind: string; at: number };
};

/** `LH_PROGRESS` (sticky): one compact snapshot of the active work. */
export type LhProgressTopic = {
  mode: string;
  goal?: { objective: string; status: string; turn: number; maxTurns: number };
  ralph?: { name: string; iteration: number; checked: number; total: number };
  graph?: { graphId?: string; mode: string; task?: string; counts: Record<string, number> };
  /** ralph::todo_line — "2/5 · ▶ next item". */
  todos?: string;
};

const obj = (v: unknown): Record<string, unknown> | undefined => (v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : undefined);
const str = (v: unknown): string | undefined => (typeof v === 'string' && v ? v : undefined);
const num = (v: unknown): number | undefined => (typeof v === 'number' && Number.isFinite(v) ? v : undefined);

export function parseLhState(v: unknown): LhState | undefined {
  const o = obj(v);
  const mode = str(o?.mode);
  if (!o || !mode) return undefined;
  const owner = obj(o.owner);
  const ownerKind = str(owner?.kind);
  const lastStop = obj(o.lastStop);
  const stopKind = str(lastStop?.kind);
  const stopAt = num(lastStop?.at);
  return {
    mode,
    ...(str(o.paused) ? { paused: str(o.paused) } : {}),
    ...(owner && ownerKind ? { owner: { kind: ownerKind, status: owner.status === 'parked' ? 'parked' : 'active' } } : {}),
    ...(lastStop && stopKind && stopAt !== undefined ? { lastStop: { kind: stopKind, at: stopAt } } : {}),
  };
}

export function parseLhProgress(v: unknown): LhProgressTopic | undefined {
  const o = obj(v);
  const mode = str(o?.mode);
  if (!o || !mode) return undefined;
  const goal = obj(o.goal);
  const goalObjective = str(goal?.objective);
  const ralph = obj(o.ralph);
  const ralphName = str(ralph?.name);
  const graph = obj(o.graph);
  const graphMode = str(graph?.mode);
  const counts = obj(graph?.counts) ?? {};
  const countMap: Record<string, number> = {};
  for (const [k, val] of Object.entries(counts)) {
    const n = num(val);
    if (n !== undefined) countMap[k] = n;
  }
  return {
    mode,
    ...(goal && goalObjective
      ? { goal: { objective: goalObjective, status: str(goal.status) ?? 'active', turn: num(goal.turn) ?? 0, maxTurns: num(goal.maxTurns) ?? 0 } }
      : {}),
    ...(ralph && ralphName
      ? { ralph: { name: ralphName, iteration: num(ralph.iteration) ?? 0, checked: num(ralph.checked) ?? 0, total: num(ralph.total) ?? 0 } }
      : {}),
    ...(graph && graphMode ? { graph: { ...(str(graph.graphId) ? { graphId: str(graph.graphId) } : {}), mode: graphMode, ...(str(graph.task) ? { task: str(graph.task) } : {}), counts: countMap } } : {}),
    ...(str(o.todos) ? { todos: str(o.todos) } : {}),
  };
}

// ── View shaping ──────────────────────────────────────────────────────────

export type LhPhase = 'active' | 'selected' | 'paused';

/** What the LH section and the capsule's LH chip show. */
export type LhSummary = {
  mode: string;
  label: string;
  phase: LhPhase;
  goal?: LhProgressTopic['goal'];
  ralph?: LhProgressTopic['ralph'];
  graph?: LhProgressTopic['graph'];
  todosLine?: string;
  /** 0..100 for the section's bar, when there is anything to measure. */
  percent?: number;
  /** The capsule chip's count ("3/7", "#4", "45%"). */
  countText?: string;
};

/**
 * The LH view from both topics. `state` (owner/parked) decides the phase;
 * `progress` carries the numbers of the mode's current work. Without a
 * topic (an older host) there is nothing to show — undefined.
 */
export function lhSummary(state: LhState | undefined, progress: LhProgressTopic | undefined): LhSummary | undefined {
  const mode = state && state.mode !== 'none' ? state.mode : undefined;
  const paused = state?.paused && state.paused !== 'none' ? state.paused : undefined;
  const current = mode ?? paused;
  // [graph-mode] an explicit `none` state (/regular) is Plain even while a stale progress value
  // of the last mode is still sticky; progress stands in only when no state was published.
  if (!current) return !state && progress && progress.mode !== 'none' ? { mode: progress.mode, label: modeLabel(progress.mode), phase: 'active', ...progressParts(progress) } : undefined;
  const active = state?.owner?.status === 'active';
  const phase: LhPhase = mode ? (active ? 'active' : 'selected') : 'paused';
  // The topics are published together; a stale progress of another mode paints nothing.
  const parts = progress?.mode === current ? progressParts(progress) : {};
  return { mode: current, label: modeLabel(current), phase, ...parts };
}

function progressParts(progress: LhProgressTopic | undefined): Pick<LhSummary, 'goal' | 'ralph' | 'graph' | 'todosLine' | 'percent' | 'countText'> {
  if (!progress) return {};
  const out: Pick<LhSummary, 'goal' | 'ralph' | 'graph' | 'todosLine' | 'percent' | 'countText'> = {};
  if (progress.goal) {
    out.goal = progress.goal;
    const turn = progress.goal.turn;
    out.countText = turn > 0 ? `t${turn}` : undefined;
  }
  if (progress.ralph) {
    out.ralph = progress.ralph;
    const r = progress.ralph;
    if (r.total > 0) {
      out.percent = Math.round((r.checked / r.total) * 100);
      out.countText = `${r.checked}/${r.total}`;
    } else out.countText = `#${r.iteration}`;
  }
  if (progress.graph) {
    out.graph = progress.graph;
    const c = graphSettled(progress.graph.counts);
    if (c.total > 0) {
      const done = Math.max(0, c.settled - c.failed);
      out.percent = Math.round((done / c.total) * 100);
      out.countText = `${done}/${c.total}`;
    }
  }
  if (progress.todos) out.todosLine = progress.todos;
  return out;
}

/** Settled (completed + failed + stopped + cancelled + superseded) of the graph counts. */
export function graphSettled(counts: Record<string, number>): { settled: number; total: number; running: number; failed: number } {
  const get = (k: string) => counts[k] ?? 0;
  const terminal = get('completed') + get('failed') + get('stopped') + get('cancelled') + get('superseded');
  const live = get('running') + get('queued') + get('blocked');
  return { settled: terminal, total: terminal + live, running: get('running'), failed: get('failed') };
}

/** One compact line of the graph counts: "3/7 settled · 1 running". */
export function graphCountLine(counts: Record<string, number>): string {
  const c = graphSettled(counts);
  const parts = [`${c.settled}/${c.total} settled`];
  if (c.running) parts.push(`${c.running} running`);
  if (c.failed) parts.push(`${c.failed} failed`);
  return parts.join(' · ');
}

/**
 * The `/<mode> <sub>` prompt that drives the section's actions (long-horizon's
 * own mode commands; core runs extension commands without a model turn).
 * `stop` parks/stops the owner, `resume` picks a parked one up again,
 * `history` prints the graph/swarm run history in the chat.
 */
export function lhModeCommand(mode: string, sub: 'stop' | 'resume' | 'history'): string {
  return `/${mode} ${sub}`;
}
