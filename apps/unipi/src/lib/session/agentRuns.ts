/**
 * Subagent runs of a turn ([ui-subagents]): the `subagent-step` / `sidekick-step`
 * entries a child writes into its parent's transcript, joined to the
 * `run_subagent` call that started it, so the timeline renders **one card per
 * run** at its call (DeepSeek Harness: delegation is its own activity, counted
 * apart from the parent's tools; one live detail line per running call)
 * instead of one top-level row per child tool, interleaved in arrival order.
 *
 * Pure over entry JSON; `turns.ts` builds the runs, the timeline's
 * `AgentCard` renders them.
 */
import type { SessionEntry } from './types';

export type AgentStatus = 'running' | 'done' | 'failed' | 'cancelled' | 'background';

/** One subagent run (a `run_subagent` call, or child steps whose call is not in this turn). */
export type AgentRun = {
  /** The child's agent id (`details.id` / step `agentId`); absent before the first progress. */
  agentId?: string;
  title: string;
  profile?: string;
  /** "Explore subagent" (`details.profileLabel`). */
  profileLabel?: string;
  /** `provider/id` of the child's model. */
  model?: string;
  status: AgentStatus;
  startedAt?: number;
  durationMs?: number;
  /** Child tool calls so far (details, else the tool steps seen). */
  toolCalls: number;
  /** Distinct files the child read / edited (from its steps). */
  files: number;
  /** Child step entries in order (every one the turn holds; the card caps what it renders). */
  steps: SessionEntry[];
  /** The child's running tool (live `details.current`). */
  current?: { name: string; arg: string };
  /** Final report excerpt (done) / error (failed). */
  report?: string;
  error?: string;
  /** The `run_subagent` call id (absent for orphan step runs). */
  callId?: string;
  /** Delegation depth of this card (0 = the parent's own call). */
  depth: number;
};

/** Tool names that start a subagent (the parent's delegation calls). */
export const AGENT_TOOLS: ReadonlySet<string> = new Set(['run_subagent', 'subagent', 'spawn_subagent']);
export const isAgentTool = (name: string): boolean => AGENT_TOOLS.has(name);

const STEP_TYPES: ReadonlySet<string> = new Set(['subagent-step', 'sidekick-step']);
const COMPLETION_TYPES: ReadonlySet<string> = new Set(['subagent-completion', 'sidekick-completion']);

const str = (v: unknown): string | undefined => (typeof v === 'string' && v.trim() ? v : undefined);
const num = (v: unknown): number | undefined => (typeof v === 'number' && Number.isFinite(v) ? v : undefined);
const obj = (v: unknown): Record<string, unknown> | undefined => (v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : undefined);

export type StepData = {
  kind?: string;
  name?: string;
  arg?: string;
  text?: string;
  output?: string;
  isError?: boolean;
  durationMs?: number;
  args?: Record<string, unknown>;
  agentId?: string;
  id?: string;
  title?: string;
  label?: string;
  profile?: string;
};

export const stepDataOf = (e: SessionEntry): StepData => (obj(e.data) ?? {}) as StepData;
export const isStepEntry = (e: SessionEntry): boolean => e.type === 'custom' && STEP_TYPES.has(String(e.customType ?? ''));
export const stepAgentId = (e: SessionEntry): string | undefined => {
  const d = stepDataOf(e);
  return str(d.agentId) ?? str(d.id);
};

/** The `details` of a completion card (`subagent-completion` custom message). */
export function completionOf(e: SessionEntry): { agentId: string; details: Record<string, unknown> } | undefined {
  if (!COMPLETION_TYPES.has(String(e.customType ?? ''))) return undefined;
  const details = obj(e.message?.details) ?? obj((e as { details?: unknown }).details);
  const agentId = str(details?.id) ?? str(details?.agentId);
  return details && agentId ? { agentId, details } : undefined;
}

const STATUS: Record<string, AgentStatus> = { completed: 'done', done: 'done', failed: 'failed', error: 'failed', cancelled: 'cancelled', aborted: 'cancelled', running: 'running' };

/** Report text without the `--- subagent id · status · …` footer / cancelled head. */
export function reportExcerpt(text: string | undefined): string | undefined {
  if (!text) return undefined;
  const body = text.split('\n\n--- subagent ')[0]!.replace(/^Subagent \S+ was cancelled[^\n]*Partial output:\n\n/, '').trim();
  return body || undefined;
}

const FILE_TOOLS = /^(read|edit|write|multi_edit|apply_patch|read_file|write_file|edit_file|view)$/i;

/** Counters from the steps: tool steps and distinct read / edited paths. */
export function stepCounters(steps: readonly SessionEntry[]): { tools: number; files: number } {
  let tools = 0;
  const files = new Set<string>();
  for (const s of steps) {
    const d = stepDataOf(s);
    if (d.kind === 'text') continue;
    tools++;
    if (d.name && FILE_TOOLS.test(d.name)) {
      const p = str(obj(d.args)?.path) ?? str(obj(d.args)?.file_path) ?? str(d.arg);
      if (p) files.add(p);
    }
  }
  return { tools, files: files.size };
}

export type RunInputs = {
  callId?: string;
  /** `run_subagent` args. */
  args?: Record<string, unknown>;
  /** Tool state of the call. */
  state?: 'preparing' | 'running' | 'ok' | 'error' | 'stopped';
  /** Result (`ok`/`error`) or live partial (`running`) details. */
  details?: unknown;
  output?: string;
  /** When the call started (its assistant message). */
  callAt?: number;
  steps: SessionEntry[];
  completion?: Record<string, unknown>;
  /** The parent's turn is running (a backgrounded child is then still "running"). */
  turnLive: boolean;
  depth?: number;
};

/** Builds an `AgentRun` from its call / steps / completion. */
export function agentRunOf(i: RunInputs): AgentRun {
  const d = obj(i.details) ?? {};
  const c = i.completion;
  const first = i.steps[0] ? stepDataOf(i.steps[0]) : undefined;
  const title = str(d.title) ?? str(i.args?.title) ?? str(c?.title) ?? str(first?.title) ?? str(first?.label) ?? str(i.args?.description) ?? 'Subagent';
  const profile = str(d.profile) ?? str(i.args?.profile) ?? str(i.args?.agent) ?? str(i.args?.subagent_type) ?? str(c?.profile) ?? str(first?.profile);
  const counters = stepCounters(i.steps);
  let status: AgentStatus;
  const phase = str(d.phase);
  if (c && str(c.status)) status = STATUS[str(c.status)!] ?? 'done';
  else if (i.state === 'running' || i.state === 'preparing') status = 'running';
  else if (i.state === 'stopped') status = 'cancelled';
  else if (phase === 'started' || phase === 'moved') status = i.turnLive ? 'running' : 'background';
  else if (phase === 'done' && str(d.status)) status = STATUS[str(d.status)!] ?? (i.state === 'error' ? 'failed' : 'done');
  else if (i.state === 'error') status = 'failed';
  else if (i.state === 'ok') status = 'done';
  else status = i.turnLive ? 'running' : 'background';
  const run: AgentRun = {
    title,
    status,
    toolCalls: Math.max(num(c?.toolCalls) ?? num(d.toolCalls) ?? 0, counters.tools),
    files: counters.files,
    steps: i.steps,
    depth: i.depth ?? 0,
  };
  const agentId = str(d.id) ?? str(c?.id) ?? (i.steps[0] ? stepAgentId(i.steps[0]) : undefined);
  if (agentId) run.agentId = agentId;
  if (profile) run.profile = profile;
  const profileLabel = str(d.profileLabel);
  if (profileLabel) run.profileLabel = profileLabel;
  const model = str(d.model) ?? str(c?.model);
  if (model) run.model = model;
  const startedAt = num(d.startedAt) ?? i.callAt;
  if (startedAt !== undefined) run.startedAt = startedAt;
  const durationMs = num(c?.durationMs) ?? num(d.durationMs);
  if (durationMs !== undefined && status !== 'running') run.durationMs = durationMs;
  if (i.callId) run.callId = i.callId;
  const cur = obj(d.current);
  if (status === 'running' && cur && str(cur.name)) run.current = { name: str(cur.name)!, arg: str(cur.arg) ?? '' };
  const error = str(c?.error) ?? str(d.error);
  if (status === 'failed' && error) run.error = error;
  if (status !== 'running' && status !== 'background') {
    const report = reportExcerpt(str(c?.report)) ?? (phase === 'done' || !phase ? reportExcerpt(i.output) : undefined);
    if (report) run.report = report;
    if (status === 'failed' && !run.error && i.output) run.error = i.output.split('\n').find((l) => l.trim())?.trim();
  }
  return run;
}

/** A nested run (the child's own `run_subagent` step) as a one-level card, from the step alone. */
export function nestedRunOf(step: SessionEntry, depth: number): AgentRun {
  const d = stepDataOf(step);
  const args = obj(d.args) ?? {};
  return agentRunOf({
    args,
    state: d.isError ? 'error' : 'ok',
    output: d.output,
    steps: [],
    turnLive: false,
    depth,
    ...(d.durationMs !== undefined ? { details: { durationMs: d.durationMs, phase: 'done', status: d.isError ? 'failed' : 'completed' } } : {}),
  });
}

/** The live one-line activity of a run: its running tool, else its latest step. */
export function liveLineOf(run: AgentRun): { name?: string; text: string } | undefined {
  if (run.current) return { name: run.current.name, text: oneLine(run.current.arg) };
  for (let i = run.steps.length - 1; i >= 0; i--) {
    const d = stepDataOf(run.steps[i]!);
    if (d.kind === 'text') {
      const t = oneLine(d.text ?? '');
      if (t) return { text: t };
      continue;
    }
    return { name: d.name ?? 'tool', text: oneLine(d.arg ?? '') };
  }
  return undefined;
}

/** Whitespace-collapsed, clipped at 160 chars (DeepSeek `normalizeLiveToolText`). */
export function oneLine(text: string, max = 160): string {
  const t = text.replace(/\s+/g, ' ').trim();
  return t.length > max ? `${t.slice(0, max - 1).trimEnd()}…` : t;
}

/** "12 tools · 3 files". */
export function runCounters(run: AgentRun): string {
  const parts: string[] = [];
  if (run.toolCalls) parts.push(`${run.toolCalls} ${run.toolCalls === 1 ? 'tool' : 'tools'}`);
  if (run.files) parts.push(`${run.files} ${run.files === 1 ? 'file' : 'files'}`);
  return parts.join(' · ');
}

/** "3 subagents · 2 running · 1 done" (+ "· 1 failed" / "· 1 cancelled" / "· 1 in background"). */
export function agentsSummary(runs: readonly AgentRun[]): string {
  if (!runs.length) return '';
  const n = runs.length;
  const count = (s: AgentStatus) => runs.filter((r) => r.status === s).length;
  const parts = [`${n} ${n === 1 ? 'subagent' : 'subagents'}`];
  const order: Array<[AgentStatus, string]> = [
    ['running', 'running'],
    ['done', 'done'],
    ['failed', 'failed'],
    ['cancelled', 'cancelled'],
    ['background', 'in background'],
  ];
  for (const [s, label] of order) {
    const k = count(s);
    if (k && !(n === 1 && s === 'done')) parts.push(`${k} ${label}`);
  }
  return parts.join(' · ');
}
