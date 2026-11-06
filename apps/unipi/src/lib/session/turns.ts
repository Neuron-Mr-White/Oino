/**
 * Turn model (app spec §3.5, WP-A2): the current branch → turn units for the
 * timeline (A6), with ZCode's render units in unipi wording.
 *
 * - A **turn** = one user message + everything after it until the next user
 *   message that is not a steer. `turnId` = that user entry id (ui-protocol
 *   `turnEntryId`). Entries before the first user message form a `preamble`
 *   (`pre:<firstId>`); a compaction between turns is its own `compaction`
 *   marker unit (`cmp:<id>`, Z-35).
 * - A turn has **segments**: an accepted steer (a user message that arrives
 *   while the run continues: after a tool result or a `toolUse` stop) opens a
 *   new segment inside the same turn (Z-54).
 * - Per segment: `work` (thinking, tools, intermediate prose, steps, cards)
 *   folds under "Worked for 1m 3s · read 4 files, edited 2" (Z-21); `answer`
 *   is the latest assistant prose. The fold is forced open while running and
 *   on stopped / error (Z-56: "Stopped", no duration).
 * - Consecutive tools of one class group (Z-38): shell → Execute, reads /
 *   searches → Explored, writes / edits → Changed.
 *
 * `createTurnsIndex()` recomputes only the tail turn for stream events
 * (settled turns keep their identity, so Solid does no work for them) and
 * rebuilds the whole index on snapshot / reset / reconcile / compaction /
 * tree navigation (an `entriesRev` change), reusing unchanged turn objects.
 */
import { STEP_TYPES, formatDuration, isHiddenCommandText, textOf, toolCategory, type Part as ModelPart, type ToolCategory } from '../chat/model';
import { eventVisible } from '../timeline/eventKinds';
import { humanProviderError } from '../model/humanError';
import type { AgentMessage, LiveTool, RunInfo, SessionEntry, Streaming } from './types';
import { branchOf } from './reducer';
import { agentRunOf, agentsSummary, completionOf, isAgentTool, isStepEntry, stepAgentId, stepDataOf, type AgentRun } from './agentRuns';

/** Joins separate text blocks of a toolResult with `\n` when needed so adjacent blocks never glue (`…pngSaved: /…`, F-08). */
function toolOutputText(content: string | ModelPart[] | undefined): string {
  if (typeof content === 'string') return content;
  if (!Array.isArray(content)) return '';
  let out = '';
  for (const p of content) {
    if (p.type !== 'text' || typeof (p as { text?: unknown }).text !== 'string') continue;
    const chunk = (p as { text: string }).text;
    if (!chunk) continue;
    if (out && !out.endsWith('\n') && !chunk.startsWith('\n')) out += '\n';
    out += chunk;
  }
  return out;
}

/** A tool call joined with its result or live execution (same shape as `chat/model` `ToolRun`). */
export type ToolRun = {
  id: string;
  name: string;
  args: Record<string, unknown>;
  state: 'preparing' | 'running' | 'ok' | 'error' | 'stopped';
  output?: string;
  /** Live output tail while running. */
  live?: string;
  details?: unknown;
  durationMs?: number;
  /** The toolResult entry id (lazy detail fetch, `get_entries{ids}`). */
  resultId?: string;
};

export type WorkItem =
  | { kind: 'thinking'; key: string; text: string; live: boolean }
  | { kind: 'tool'; key: string; tool: ToolRun }
  | { kind: 'prose'; key: string; text: string; live: boolean }
  | { kind: 'step'; key: string; entry: SessionEntry }
  /** One subagent run ([ui-subagents]): its `run_subagent` call (absent for orphan child steps) + the child's steps. */
  | { kind: 'agent'; key: string; run: AgentRun; tool?: ToolRun }
  | { kind: 'card'; key: string; entry: SessionEntry }
  | { kind: 'compaction'; key: string; entry: SessionEntry }
  | { kind: 'error'; key: string; text: string };

/** Z-38 tool group classes. */
export type ToolGroupKind = 'execute' | 'explore' | 'changes';
/** `work` with consecutive same-class tools merged (≥ 2 tools; thinking between them stays inside, in order). */
export type WorkNode =
  | Exclude<WorkItem, { kind: 'tool' }>
  | { kind: 'tool'; key: string; tool: ToolRun }
  | { kind: 'group'; key: string; group: ToolGroupKind; items: WorkItem[]; tools: ToolRun[] }
  /** Parallel subagent runs ([ui-subagents]): ≥ 2 consecutive `agent` items under one "Running 3 subagents" container. */
  | { kind: 'agents'; key: string; items: WorkItem[]; runs: AgentRun[] };

export type FoldState = 'running' | 'done' | 'stopped' | 'error';
export type Fold = {
  state: FoldState;
  /** "Working" (the UI appends the live clock from `startedAt`), "Worked for 1m 3s", "Stopped". */
  label: string;
  /** unipi summary wording: "read 4 files, edited 2" (+ " · 1 failed"). */
  detail: string;
  durationMs?: number;
  startedAt?: number;
  /** Open regardless of the user's toggle (running / stopped / error). */
  forcedOpen: boolean;
};

export type Segment = {
  /** The user / steer entry id that opened it (segment 0 = the turn id). */
  id: string;
  index: number;
  /** The steering message (segments ≥ 1). */
  steer?: SessionEntry;
  work: WorkItem[];
  nodes: WorkNode[];
  /** [chat-events] Event cards before the segment's first work item (memory recall, nudges) — shown above the fold. */
  before?: WorkNode[];
  /** Cards appended after the assistant's final answer (model change, saved memories, …) — shown below it, never folded away. */
  after?: WorkNode[];
  answer?: Extract<WorkItem, { kind: 'prose' }>;
  /**
   * [ask-user] The `ask_user` call that ends this segment: rendered as a first-class card
   * after the segment's answer prose, never inside the fold. Work after it opens the next
   * segment (`ask:<toolCallId>`), so the card sits where the question was asked.
   */
  ask?: ToolRun;
  /** Absent when there is no work to fold. */
  fold?: Fold;
  live: boolean;
};

export type TurnSource = 'user' | 'cron' | 'harness';
export type TurnCommand = { name: string; args: string; entryId?: string; hidden: boolean };

export type Turn = {
  turnId: string;
  kind: 'turn' | 'preamble' | 'compaction';
  /** The user message entry (kind `turn`). */
  user?: SessionEntry;
  /** The compaction entry (kind `compaction`). */
  compaction?: SessionEntry;
  /** Every entry of the turn on the branch, in order (stable refs). */
  entries: SessionEntry[];
  segments: Segment[];
  live: boolean;
  startedAt?: number;
  endedAt?: number;
  durationMs?: number;
  costUsd?: number;
  model?: string;
  stopped: boolean;
  error?: string;
  source: TurnSource;
  /** `/command …` echo (Z-35: `/compact` → `hidden`, the compaction marker shows instead). */
  command?: TurnCommand;
};

/** What the index reads (a `SessionState` satisfies it). */
export type TurnsInput = {
  entries: readonly SessionEntry[];
  leafId: string | null;
  entriesRev: number;
  streaming?: Streaming;
  liveTools: Readonly<Record<string, LiveTool>>;
  run: Pick<RunInfo, 'running' | 'startedAt'>;
};

const TERMINAL = new Set(['stop', 'aborted', 'error', 'length']);
const ECHO_TYPES = new Set(['oino.command-echo', 'unipi-command-echo']);
/** State-only `custom` entries that never produce transcript cards (`oino-view` `CustomEntry` rule). */
const INTERNAL_CUSTOM_TYPES = new Set([
  'oino.tool-dispatch',
  'oino.command-echo',
  'unipi-command-echo',
  'unipi:auto-name',
  'unipi:skills-judged',
  'unipi:plan-mode',
  'unipi:permission-mode',
  'long-horizon-decision',
  'unipi-cleared-input',
  'unipi-progress',
]);

/** Z-38 class of a tool (undefined = never groups). */
export function toolGroupOf(name: string): ToolGroupKind | undefined {
  const c: ToolCategory = toolCategory(name);
  if (c === 'run') return 'execute';
  if (c === 'read' || c === 'search') return 'explore';
  if (c === 'edit') return 'changes';
  return undefined;
}

const VERB: Record<ToolCategory, [verb: string, one: string, many: string]> = {
  read: ['read', 'file', 'files'],
  search: ['searched', 'time', 'times'],
  edit: ['edited', 'file', 'files'],
  run: ['ran', 'command', 'commands'],
  web: ['searched the web', 'time', 'times'],
  ask: ['asked you', 'time', 'times'],
  agent: ['ran', 'agent', 'agents'],
  memory: ['used memory', 'time', 'times'],
  board: ['updated tasks', 'time', 'times'],
  other: ['used', 'tool', 'tools'],
};

/** The fold's summary words (unipi `processSummary` wording): "read 4 files, edited 2", " · 1 failed". */
export function foldDetail(tools: readonly ToolRun[], steps = 0): string {
  if (!tools.length) return steps ? `${steps} delegated ${steps === 1 ? 'step' : 'steps'}` : '';
  const order: ToolCategory[] = [];
  const count = new Map<ToolCategory, number>();
  for (const t of tools) {
    const c = toolCategory(t.name);
    if (!count.has(c)) order.push(c);
    count.set(c, (count.get(c) ?? 0) + 1);
  }
  const words = order.slice(0, 3).map((c, i) => {
    const n = count.get(c)!;
    const [verb, one, many] = VERB[c];
    return i === 0 ? `${verb} ${n} ${n === 1 ? one : many}` : `${verb} ${n}`;
  });
  const failed = tools.filter((t) => t.state === 'error').length;
  return words.join(', ') + (order.length > 3 ? ', …' : '') + (failed ? ` · ${failed} failed` : '');
}

/** Merges consecutive same-class tools (≥ 2) into groups; thinking between two of them joins the group. */
export function groupWork(work: readonly WorkItem[]): WorkNode[] {
  const out: WorkNode[] = [];
  let i = 0;
  while (i < work.length) {
    const item = work[i]!;
    if (item.kind === 'agent') {
      // Consecutive runs (thinking between them stays inside, in order) share one container.
      const items: WorkItem[] = [item];
      const runs: AgentRun[] = [item.run];
      let j = i + 1;
      let end = i + 1;
      while (j < work.length) {
        const next = work[j]!;
        if (next.kind === 'thinking') {
          j++;
          continue;
        }
        if (next.kind !== 'agent') break;
        for (let k = end; k <= j; k++) items.push(work[k]!);
        runs.push(next.run);
        j++;
        end = j;
      }
      if (runs.length >= 2) {
        out.push({ kind: 'agents', key: `a-${item.key}`, items, runs });
        i = end;
      } else {
        out.push(item);
        i++;
      }
      continue;
    }
    const cls = item.kind === 'tool' ? toolGroupOf(item.tool.name) : undefined;
    if (!cls) {
      out.push(item);
      i++;
      continue;
    }
    const items: WorkItem[] = [item];
    const tools: ToolRun[] = [(item as Extract<WorkItem, { kind: 'tool' }>).tool];
    let j = i + 1;
    let end = i + 1;
    while (j < work.length) {
      const next = work[j]!;
      if (next.kind === 'thinking') {
        j++;
        continue;
      }
      if (next.kind === 'tool' && toolGroupOf(next.tool.name) === cls) {
        for (let k = end; k <= j; k++) items.push(work[k]!);
        tools.push(next.tool);
        j++;
        end = j;
        continue;
      }
      break;
    }
    if (tools.length >= 2) {
      out.push({ kind: 'group', key: `g-${item.key}`, group: cls, items, tools });
      i = end;
    } else {
      out.push(item);
      i++;
    }
  }
  return out;
}

const tsOf = (e: SessionEntry): number | undefined => {
  const m = e.message?.timestamp;
  if (typeof m === 'number' && Number.isFinite(m)) return m;
  const t = e.timestamp ? Date.parse(e.timestamp) : NaN;
  return Number.isFinite(t) ? t : undefined;
};

/**
 * When an entry's work ENDED. An assistant message's own `timestamp` is when it started
 * streaming; the entry is appended when it ends. So a turn whose last step is one long stream
 * ("marathon": 330 s of text) read "Done in 0s". The entry time counts only for an assistant
 * message and only within 6 h of its start (a copied / migrated entry can carry a later time).
 */
const doneOf = (e: SessionEntry, at: number | undefined): number | undefined => {
  if (at === undefined || e.message?.role !== 'assistant' || !e.timestamp) return at;
  const t = Date.parse(e.timestamp);
  return Number.isFinite(t) && t > at && t - at < 6 * 3_600_000 ? t : at;
};

const isUser = (e: SessionEntry | undefined): boolean => e?.type === 'message' && e.message?.role === 'user';

/** A user message delivered while the run continued (after a tool result or a `toolUse` stop). */
function isSteer(prevMessage: AgentMessage | undefined): boolean {
  if (!prevMessage) return false;
  if (prevMessage.role === 'toolResult') return true;
  return prevMessage.role === 'assistant' && prevMessage.stopReason === 'toolUse';
}

/** A compaction between turns (the run before it had ended) is its own marker unit. */
function isMarkerCompaction(prevMessage: AgentMessage | undefined): boolean {
  if (!prevMessage) return true;
  if (prevMessage.role === 'assistant') return TERMINAL.has(String(prevMessage.stopReason ?? ''));
  return prevMessage.role === 'user';
}

const stoppedOf = (m: AgentMessage): boolean => m.stopReason === 'aborted' || (m.stopReason === 'error' && /\baborted\b/i.test(m.errorMessage ?? ''));

function parseArgs(args: unknown): Record<string, unknown> {
  if (args && typeof args === 'object' && !Array.isArray(args)) return args as Record<string, unknown>;
  if (typeof args === 'string') {
    try {
      const v = JSON.parse(args);
      if (v && typeof v === 'object') return v as Record<string, unknown>;
    } catch {
      // partial JSON while streaming
    }
  }
  return {};
}

function commandOf(text: string, entryId?: string): TurnCommand | undefined {
  const m = /^\/(\S+)(?:\s+([\s\S]*))?$/.exec(text.trim());
  if (!m) return undefined;
  const name = m[1]!;
  return { name, args: (m[2] ?? '').trim(), ...(entryId ? { entryId } : {}), hidden: name === 'compact' || isHiddenCommandText(text) };
}

type Start = { at: number; kind: Turn['kind'] };

/** Turn starts in `branch[from..]`; `branch[from]` itself is a start when `firstKind` is given. */
function scanStarts(branch: readonly SessionEntry[], from: number, firstKind?: Turn['kind']): Start[] {
  const starts: Start[] = [];
  let prev: AgentMessage | undefined;
  for (let i = from; i < branch.length; i++) {
    const e = branch[i]!;
    if (i === from) {
      const kind = firstKind ?? (isUser(e) ? 'turn' : e.type === 'compaction' ? 'compaction' : 'preamble');
      starts.push({ at: i, kind });
      if (e.type === 'message' && e.message) prev = e.message;
      continue;
    }
    if (e.type === 'message' && e.message) {
      if (e.message.role === 'user' && !isSteer(prev)) starts.push({ at: i, kind: 'turn' });
      prev = e.message;
    } else if (e.type === 'compaction' && isMarkerCompaction(prev)) {
      starts.push({ at: i, kind: 'compaction' });
    }
  }
  return starts;
}

type Live = { streaming?: Streaming; liveTools: Readonly<Record<string, LiveTool>>; running: boolean; runStartedAt?: number };

function echoCommand(e: SessionEntry | undefined): TurnCommand | undefined {
  if (!e || e.type !== 'custom' || !ECHO_TYPES.has(String(e.customType))) return undefined;
  const d = (e.data ?? {}) as { name?: unknown; args?: unknown; text?: unknown };
  if (typeof d.name === 'string') return { name: d.name, args: typeof d.args === 'string' ? d.args : '', entryId: e.id, hidden: d.name === 'compact' || isHiddenCommandText(`/${d.name}`) };
  if (typeof d.text === 'string') return commandOf(d.text, e.id);
  return undefined;
}

/** Builds one turn from `branch[start, end)`; `live` only for the tail turn. */
function buildTurn(branch: readonly SessionEntry[], start: Start, end: number, live: Live | undefined): Turn {
  const first = branch[start.at]!;
  const entries = branch.slice(start.at, end) as SessionEntry[];
  const results = new Map<string, { message: AgentMessage; id: string; at?: number }>();
  for (const e of entries) {
    if (e.message?.role === 'toolResult' && typeof e.message.toolCallId === 'string') {
      results.set(e.message.toolCallId, { message: e.message, id: e.id, at: tsOf(e) });
    }
  }
  const running = !!live?.running;
  // [ui-subagents] child steps by agent id (rendered inside their run's card, never as loose rows).
  const stepsByAgent = new Map<string, SessionEntry[]>();
  const completions = new Map<string, Record<string, unknown>>();
  for (const e of entries) {
    if (isStepEntry(e)) {
      const id = stepAgentId(e);
      if (id) {
        const list = stepsByAgent.get(id);
        if (list) list.push(e);
        else stepsByAgent.set(id, [e]);
      }
    } else if (e.type === 'custom_message') {
      const c = completionOf(e);
      if (c) completions.set(c.agentId, c.details);
    }
  }
  const claimed = new Set<string>();
  const agentItem = (key: string, tool: ToolRun | undefined, details: unknown, callAt: number | undefined, agentId?: string): Extract<WorkItem, { kind: 'agent' }> => {
    const detailsId = (details as { id?: unknown } | undefined)?.id;
    let id = agentId ?? (typeof detailsId === 'string' ? detailsId : undefined);
    if (!id && tool) {
      // No details yet (reopened mid-run): the first unclaimed child whose steps carry this title.
      const title = typeof tool.args.title === 'string' ? tool.args.title : undefined;
      for (const [agentId, steps] of stepsByAgent) {
        if (!claimed.has(agentId) && title && stepDataOf(steps[0]!).title === title) {
          id = agentId;
          break;
        }
      }
    }
    if (id) claimed.add(id);
    const run = agentRunOf({
      ...(tool ? { callId: tool.id, args: tool.args, state: tool.state, ...(tool.output !== undefined ? { output: tool.output } : {}) } : {}),
      details,
      ...(callAt !== undefined ? { callAt } : {}),
      steps: (id && stepsByAgent.get(id)) || [],
      ...(id && completions.get(id) ? { completion: completions.get(id)! } : {}),
      turnLive: running,
    });
    return { kind: 'agent', key, run, ...(tool ? { tool } : {}) };
  };
  const segments: Segment[] = [];
  let seg: Segment = { id: start.kind === 'compaction' ? `cmp:${first.id}` : first.id, index: 0, work: [], nodes: [], live: false };
  segments.push(seg);
  const segMeta: Array<{ startedAt?: number; endedAt?: number; stopped: boolean; error: boolean; steps: number }> = [{ startedAt: tsOf(first), stopped: false, error: false, steps: 0 }];
  // [ask-user] after an `ask_user` call the next work item opens a new segment (lazily, so a
  // question that is still waiting has no empty "Working" fold under it).
  let afterAsk: { at?: number } | undefined;
  /** The segment new work goes to. */
  const cur = (): Segment => {
    if (afterAsk) {
      const at = afterAsk.at;
      afterAsk = undefined;
      seg = { id: `ask:${seg.ask!.id}`, index: segments.length, work: [], nodes: [], live: false };
      segments.push(seg);
      segMeta.push({ startedAt: at, endedAt: at, stopped: false, error: false, steps: 0 });
    }
    return seg;
  };
  const pushTool = (key: string, tool: ToolRun, at: number | undefined) => {
    if (tool.name === ASK_TOOL) {
      const s = cur();
      s.ask = tool;
      afterAsk = { at };
      return;
    }
    cur().work.push({ kind: 'tool', key, tool });
  };
  let cost = 0;
  let model: string | undefined;
  let lastAssistant: AgentMessage | undefined;
  let endedAt: number | undefined;
  const turn: Turn = { turnId: start.kind === 'turn' ? first.id : start.kind === 'compaction' ? `cmp:${first.id}` : `pre:${first.id}`, kind: start.kind, entries, segments, live: false, stopped: false, source: 'user' };
  const pushParts = (key: string, parts: readonly ModelPart[], isLive: boolean, stopped: boolean, callAt?: number) => {
    let stepStart = callAt;
    parts.forEach((part, i) => {
      const k = `${key}-${i}`;
      if (part.type === 'thinking') {
        const text = String((part as { thinking?: unknown }).thinking ?? '');
        if (!text.trim() && !isLive) return;
        cur().work.push({ kind: 'thinking', key: k, text, live: isLive && i === parts.length - 1 });
      } else if (part.type === 'toolCall') {
        const call = part as { id?: unknown; name?: unknown; arguments?: unknown };
        const id = String(call.id ?? '');
        const result = results.get(id);
        const lt = live?.liveTools[id];
        const tool: ToolRun = { id, name: String(call.name ?? ''), args: parseArgs(call.arguments), state: 'running' };
        if (result) {
          tool.state = result.message.isError ? 'error' : 'ok';
          tool.output = toolOutputText(result.message.content as string | ModelPart[] | undefined);
          if (result.message.details !== undefined) tool.details = result.message.details;
          tool.resultId = result.id;
          const det = result.message.details as { durationMs?: unknown; duration_ms?: unknown } | undefined;
          const detMs = typeof det?.durationMs === 'number' ? det.durationMs : typeof det?.duration_ms === 'number' ? det.duration_ms : undefined;
          if (detMs !== undefined && Number.isFinite(detMs) && detMs >= 0) {
            tool.durationMs = detMs;
          } else if (stepStart !== undefined && result.at !== undefined && result.at >= stepStart && result.at - stepStart < 3_600_000) {
            tool.durationMs = result.at - stepStart;
          }
          if (result.at !== undefined) stepStart = result.at;
        } else if (lt) {
          tool.state = lt.done ? (lt.isError ? 'error' : 'ok') : 'running';
          if (lt.partial !== undefined) tool.live = lt.partial;
        } else if (isLive) tool.state = 'preparing';
        else tool.state = stopped || !running ? 'stopped' : 'running';
        if (lt?.durationMs !== undefined) tool.durationMs = lt.durationMs;
        if (isAgentTool(tool.name)) {
          const details = result ? result.message.details : lt?.details;
          cur().work.push(agentItem(k, tool, details, lt?.startedAt ?? callAt));
          return;
        }
        pushTool(k, tool, result?.at ?? callAt);
      } else if (part.type === 'text') {
        const text = String((part as { text?: unknown }).text ?? '');
        if (!text.trim()) return;
        cur().work.push({ kind: 'prose', key: k, text, live: isLive });
      }
    });
  };
  for (let i = 0; i < entries.length; i++) {
    const e = entries[i]!;
    const at = tsOf(e);
    const mRole = e.message?.role;
    const countsForDuration =
      (e.type === 'message' && (mRole === 'user' || mRole === 'assistant' || mRole === 'toolResult' || mRole === 'bashExecution')) ||
      (e.type === 'custom' && STEP_TYPES.has(String(e.customType ?? '')));
    if (at !== undefined && countsForDuration) {
      endedAt = doneOf(e, at);
      // [ask-user] the time the user takes to answer is not the segment's work.
      if (!seg.ask) segMeta[seg.index]!.endedAt = endedAt;
    }
    if (i === 0 && start.kind === 'turn') {
      turn.user = e;
      const m = e.message!;
      const text = textOf(m.content as string | ModelPart[] | undefined);
      turn.command = echoCommand(branch[start.at - 1]) ?? commandOf(text, e.id);
      if (turn.command && isHiddenCommandText(text)) turn.command.hidden = true;
      const hasProvenance = !!(m.unipiHarness || (m.details as { oinoProvenance?: unknown; unipiHarness?: unknown } | undefined)?.oinoProvenance || (m.details as { unipiHarness?: unknown } | undefined)?.unipiHarness);
      turn.source = m.source === 'cron' || m.unipiCron ? 'cron' : hasProvenance ? 'harness' : 'user';
      continue;
    }
    if (i === 0 && start.kind === 'compaction') {
      turn.compaction = e;
      continue;
    }
    if (e.type === 'message' && e.message) {
      const m = e.message;
      if (m.role === 'user') {
        afterAsk = undefined;
        seg = { id: e.id, index: segments.length, steer: e, work: [], nodes: [], live: false };
        segments.push(seg);
        segMeta.push({ startedAt: at, endedAt: at, stopped: false, error: false, steps: 0 });
      } else if (m.role === 'assistant') {
        lastAssistant = m;
        cost += m.usage?.cost?.total ?? 0;
        if (typeof m.model === 'string') model = m.model;
        const stopped = stoppedOf(m);
        pushParts(e.id, (Array.isArray(m.content) ? m.content : []) as ModelPart[], false, stopped, at);
        if (stopped) segMeta[seg.index]!.stopped = true;
        else if (m.stopReason === 'error') {
          segMeta[seg.index]!.error = true;
          if (m.errorMessage) cur().work.push({ kind: 'error', key: `${e.id}-err`, text: humanProviderError(m.errorMessage, typeof m.provider === 'string' ? m.provider : undefined, { settingsHint: true }) });
        }
      } else if (m.role === 'bashExecution' || (m.role === 'system' && eventVisible(e))) {
        // [chat-events] a user `!cmd` run, or a mid-chat system message (mode switch, agent prompt).
        cur().work.push({ kind: 'card', key: e.id, entry: e });
      }
      // toolResult: joined into its call.
    } else if (e.type === 'custom') {
      const ct = String(e.customType ?? '');
      if (ECHO_TYPES.has(ct)) {
        const cmd = echoCommand(e);
        // An echo right before the user message it opened belongs to that turn (its `command`).
        if (cmd?.hidden || (start.at + i + 1 === end && isUser(branch[end]))) continue;
      }
      if (INTERNAL_CUSTOM_TYPES.has(ct) && !eventVisible(e)) continue;
      if (STEP_TYPES.has(ct)) {
        const agentId = stepAgentId(e);
        if (agentId && claimed.has(agentId)) continue; // inside its run's card
        if (agentId) {
          // A child whose call is not in this turn (backgrounded earlier, resumed): one card at its first step.
          cur().work.push(agentItem(`agent-${agentId}`, undefined, undefined, at, agentId));
          continue;
        }
        cur().work.push({ kind: 'step', key: e.id, entry: e });
        segMeta[seg.index]!.steps++;
      } else cur().work.push({ kind: 'card', key: e.id, entry: e });
    } else if (e.type === 'custom_message') {
      // [chat-events] `display: false` hides only kinds without an app summary (memory recall, ralph… still show).
      if (!eventVisible(e)) continue;
      cur().work.push({ kind: 'card', key: e.id, entry: e });
    } else if (e.type === 'model_change' || e.type === 'branch_summary' || e.type === 'thinking_level_change') {
      // D-02: before the first prompt these are the chat's starting settings (the composer
      // chips show them), not changes — no "Model → … / Thinking → …" rows atop a new chat.
      if (start.kind === 'preamble' && e.type !== 'branch_summary') continue;
      cur().work.push({ kind: 'card', key: e.id, entry: e });
    } else if (e.type === 'compaction') {
      cur().work.push({ kind: 'compaction', key: e.id, entry: e });
    }
  }
  if (live) {
    if (live.streaming) {
      const m = live.streaming.message;
      pushParts('stream', (Array.isArray(m.content) ? m.content : []) as ModelPart[], true, false);
    }
    // Live tools whose call part never streamed (joined late / reconnect).
    const shown = new Set<string>();
    for (const s of segments) {
      if (s.ask) shown.add(s.ask.id);
      for (const w of s.work) if (w.kind === 'tool') shown.add(w.tool.id);
      else if (w.kind === 'agent' && w.tool) shown.add(w.tool.id);
    }
    for (const t of Object.values(live.liveTools)) {
      if (shown.has(t.id) || t.done || t.parentId) continue;
      if (isAgentTool(t.name)) {
        cur().work.push(agentItem(`live-${t.id}`, { id: t.id, name: t.name, args: t.args, state: 'running' }, t.details, t.startedAt));
        continue;
      }
      pushTool(`live-${t.id}`, { id: t.id, name: t.name, args: t.args, state: 'running', ...(t.partial !== undefined ? { live: t.partial } : {}) }, t.startedAt);
    }
  }
  turn.live = running;
  turn.startedAt = tsOf(first);
  if (!running) {
    turn.endedAt = endedAt;
    if (turn.startedAt !== undefined && endedAt !== undefined) turn.durationMs = Math.max(0, endedAt - turn.startedAt);
  }
  if (cost) turn.costUsd = cost;
  if (model) turn.model = model;
  if (lastAssistant) {
    turn.stopped = stoppedOf(lastAssistant);
    if (!turn.stopped && lastAssistant.stopReason === 'error' && lastAssistant.errorMessage) {
      turn.error = humanProviderError(lastAssistant.errorMessage, typeof lastAssistant.provider === 'string' ? lastAssistant.provider : undefined, { settingsHint: true });
    }
  }
  // [ask-user] a segment whose question is waiting is not "Working": the card is what's live.
  for (const s of segments) finishSegment(s, segMeta[s.index]!, running && s === seg && !askWaiting(s));
  return turn;
}

/** The `unipi.ask-user` tool: its calls render as inline question cards (never folded). */
export const ASK_TOOL = 'ask_user';

/** The segment ends on a question the user has not answered yet. */
function askWaiting(s: Segment): boolean {
  return !!s.ask && (s.ask.state === 'running' || s.ask.state === 'preparing');
}

/** `customType` of the marker a "Move to a workspace…" appends (core `oino.cwd_change`). */
export const CWD_CHANGE_TYPE = 'oino.cwd_change';

/** The chat moved to a workspace here (shown as a marker row after the answer, never folded away). */
export function isCwdChange(e: SessionEntry): boolean {
  return e.type === 'custom_message' && e.customType === CWD_CHANGE_TYPE;
}

/** The folder a `oino.cwd_change` entry moved the chat to. */
export function cwdChangeTarget(e: SessionEntry): string | undefined {
  const details = (e.details ?? (e.message as { details?: unknown } | undefined)?.details) as { cwd?: unknown } | undefined;
  return typeof details?.cwd === 'string' && details.cwd ? details.cwd : undefined;
}

function finishSegment(s: Segment, meta: { startedAt?: number; endedAt?: number; stopped: boolean; error: boolean; steps: number }, live: boolean): void {
  s.live = live;
  // [chat-events] Event cards that open the segment (recall, nudges, mode switch) sit above the
  // "Worked" fold, not inside it: the fold is closed once the run is done, which hid them.
  let lead = 0;
  while (lead < s.work.length && s.work[lead]!.kind === 'card') lead++;
  if (lead) {
    s.before = groupWork(s.work.slice(0, lead));
    s.work = s.work.slice(lead);
  }
  // Extract the final answer prose even when trailing metadata cards or a compaction
  // were appended after the assistant finished speaking; they render after the answer
  // (never buried inside the closed "Worked for …" fold).
  for (let i = s.work.length - 1; i >= 0; i--) {
    const item = s.work[i]!;
    if (item.kind === 'prose') {
      s.answer = item;
      const tail = s.work.slice(i + 1);
      if (tail.length) s.after = groupWork(tail);
      s.work = s.work.slice(0, i);
      break;
    }
    if (item.kind !== 'card' && item.kind !== 'compaction') break;
  }
  // Even if the segment ended without prose (e.g. tool-only turn followed by auto-compaction),
  // lift trailing compaction + event cards out of the closed fold so compaction is always visible.
  if (!s.answer && !s.after) {
    let tailFrom = s.work.length;
    while (tailFrom > 0 && (s.work[tailFrom - 1]!.kind === 'card' || s.work[tailFrom - 1]!.kind === 'compaction')) {
      tailFrom--;
    }
    if (tailFrom < s.work.length && s.work.slice(tailFrom).some((w) => w.kind === 'compaction')) {
      s.after = groupWork(s.work.slice(tailFrom));
      s.work = s.work.slice(0, tailFrom);
    }
  }
  s.nodes = groupWork(s.work);
  if (!s.work.length && !live) return;
  const tools = s.work.flatMap((w) => (w.kind === 'tool' ? [w.tool] : []));
  const state: FoldState = live ? 'running' : meta.stopped ? 'stopped' : meta.error ? 'error' : 'done';
  const durationMs = (state === 'done' || state === 'error') && meta.startedAt !== undefined && meta.endedAt !== undefined ? Math.max(0, meta.endedAt - meta.startedAt) : undefined;
  s.fold = {
    state,
    label: state === 'running' ? 'Working' : state === 'stopped' ? 'Stopped' : durationMs === undefined ? 'Worked' : `Worked for ${formatDuration(durationMs)}`,
    detail: [agentsSummary(s.work.flatMap((w) => (w.kind === 'agent' ? [w.run] : []))), foldDetail(tools, meta.steps)].filter(Boolean).join(' · '),
    ...(durationMs !== undefined ? { durationMs } : {}),
    ...(meta.startedAt !== undefined ? { startedAt: meta.startedAt } : {}),
    forcedOpen: state !== 'done',
  };
}

const liveOf = (input: TurnsInput): Live => ({ streaming: input.streaming, liveTools: input.liveTools, running: input.run.running, runStartedAt: input.run.startedAt });

/** All turns of `branch` (pure; the index below is the incremental version). */
export function buildTurns(input: TurnsInput): Turn[] {
  const branch = branchOf(input.entries, input.leafId);
  const starts = scanStarts(branch, 0);
  return starts.map((s, i) => buildTurn(branch, s, starts[i + 1]?.at ?? branch.length, i === starts.length - 1 ? liveOf(input) : undefined));
}

function turnIdOf(branch: readonly SessionEntry[], s: Start): string {
  const id = branch[s.at]!.id;
  return s.kind === 'turn' ? id : s.kind === 'compaction' ? `cmp:${id}` : `pre:${id}`;
}

function sameEntries(a: readonly SessionEntry[], branch: readonly SessionEntry[], from: number, to: number): boolean {
  if (a.length !== to - from) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== branch[from + i]) return false;
  return true;
}

export type TurnsIndex = {
  /** Turns for the new input: settled turns keep identity; only the tail is rebuilt for appends / live changes. */
  update(input: TurnsInput): Turn[];
  /** How the last update ran (tests / perf). */
  readonly lastMode: 'full' | 'tail' | 'none';
};

export function createTurnsIndex(): TurnsIndex {
  let rev = -1;
  let source: readonly SessionEntry[] = [];
  let sourceLen = 0;
  let leaf: string | null = null;
  let branch: SessionEntry[] = [];
  let starts: Start[] = [];
  let turns: Turn[] = [];
  let mode: TurnsIndex['lastMode'] = 'none';

  const full = (input: TurnsInput) => {
    mode = 'full';
    branch = branchOf(input.entries, input.leafId).slice();
    starts = scanStarts(branch, 0);
    const prev = new Map(turns.map((t) => [t.turnId, t]));
    const live = liveOf(input);
    turns = starts.map((s, i) => {
      const end = starts[i + 1]?.at ?? branch.length;
      const tail = i === starts.length - 1;
      const old = prev.get(turnIdOf(branch, s));
      if (!tail && old && !old.live && sameEntries(old.entries, branch, s.at, end)) return old;
      return buildTurn(branch, s, end, tail ? live : undefined);
    });
  };

  const tail = (input: TurnsInput) => {
    mode = 'tail';
    for (let i = sourceLen; i < input.entries.length; i++) branch.push(input.entries[i]!);
    const lastStart = starts[starts.length - 1];
    const from = lastStart?.at ?? 0;
    const fresh = scanStarts(branch, from, lastStart?.kind);
    starts = starts.slice(0, Math.max(0, starts.length - 1)).concat(fresh);
    const keep = starts.length - fresh.length;
    const live = liveOf(input);
    const next = turns.slice(0, keep);
    for (let i = keep; i < starts.length; i++) {
      const end = starts[i + 1]?.at ?? branch.length;
      const isTail = i === starts.length - 1;
      const old = turns[i];
      // The previous tail, settled and unchanged, keeps its identity once a new turn follows it.
      if (!isTail && old && !old.live && old.turnId === turnIdOf(branch, starts[i]!) && sameEntries(old.entries, branch, starts[i]!.at, end)) next.push(old);
      else next.push(buildTurn(branch, starts[i]!, end, isTail ? live : undefined));
    }
    turns = next;
  };

  return {
    update(input) {
      const appended =
        input.entriesRev === rev &&
        input.entries === source &&
        input.entries.length >= sourceLen &&
        (sourceLen === 0 || input.entries[sourceLen - 1] === branch[branch.length - 1]) &&
        (input.entries.length === sourceLen || input.entries[sourceLen]?.parentId === leaf || leaf === null);
      if (appended) tail(input);
      else full(input);
      rev = input.entriesRev;
      source = input.entries;
      sourceLen = input.entries.length;
      leaf = input.leafId;
      return turns;
    },
    get lastMode() {
      return mode;
    },
  };
}
