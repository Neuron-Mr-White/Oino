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
import { INTERRUPTED_TYPE, NOTE_TYPE, describeEvent, eventVisible, isSummaryRequest } from '../timeline/eventKinds';
import { humanProviderError } from '../model/humanError';
import { modeCommandKey } from './outbox';
import type { AgentMessage, LiveTool, RunInfo, SessionEntry, Streaming } from './types';
import { branchOf } from './reducer';
import { stripTruncationNote } from './fullMessages';
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
  /** Persisted assistant entry supplying this call (full-argument hydration). */
  callEntryId?: string;
};

export type WorkItem =
  | { kind: 'thinking'; key: string; text: string; live: boolean }
  | { kind: 'tool'; key: string; tool: ToolRun }
  /** `cut`: a cached excerpt (the warm-launch snapshot): the rest arrives with the live turn (D10). */
  | { kind: 'prose'; key: string; text: string; live: boolean; cut?: boolean }
  | { kind: 'step'; key: string; entry: SessionEntry }
  /** One subagent run ([ui-subagents]): its `run_subagent` call (absent for orphan child steps) + the child's steps. */
  | { kind: 'agent'; key: string; run: AgentRun; tool?: ToolRun }
  | { kind: 'card'; key: string; entry: SessionEntry }
  | { kind: 'compaction'; key: string; entry: SessionEntry }
  /** `count` > 1: that many consecutive identical errors folded into this one (`foldErrors`); `key` is the newest's. */
  | { kind: 'error'; key: string; text: string; count?: number };

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
  /** Active model-view anchor; older turns remain scrollable. */
  renderCutoff?: SessionEntry;
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
  harnessCount?: number;
  /** `/command …` echo (Z-35: `/compact` → `hidden`, the compaction marker shows instead). */
  command?: TurnCommand;
  /** D3: a bare `/swarm <text>` echo with no user message (yet): this turn IS the typed command; the outbox key of that text. */
  echoKey?: string;
  /** The `messageId` of the request that ran this echo's command (host `oino.command-echo` data), when the host sent one. */
  echoMessageId?: string;
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
/** Core's durable "this run ended" boundary entry. */
const AGENT_END_TYPE = 'oino.agent-end';
/** The loop guard's notice: the run stopped right after it (chats from before `oino.agent-end` carry only this). */
const LOOP_NOTICE_TYPE = 'oino.loop-notice';
const ECHO_TYPES = new Set(['oino.command-echo', 'unipi-command-echo']);
/** State-only `custom` entries that never produce transcript cards (`oino-view` `CustomEntry` rule). */
const INTERNAL_CUSTOM_TYPES = new Set([
  // oino-core state entries (one per round): never rows, they only split folds.
  'oino.image-policy',
  'oino.sent-frontier',
  'oino.tool-declarations',
  'oino.search-snapshot',
  'oino.image-setting',
  'oino.virtual-model-state',
  'oino.resume-repair',
  'oino.agent-end',
  'oino.tool-dispatch',
  'oino:autonomy',
  'oino:unattended',
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

/**
 * Folds runs of directly consecutive errors with the same text (trimmed) into one item carrying
 * `count`. The folded item takes the newest error's key and text; any other item between two
 * errors keeps them apart.
 */
export function foldErrors(work: readonly WorkItem[]): WorkItem[] {
  const out: WorkItem[] = [];
  for (const item of work) {
    const prev = out.at(-1);
    if (item.kind === 'error' && prev?.kind === 'error' && prev.text.trim() === item.text.trim()) {
      out[out.length - 1] = { kind: 'error', key: item.key, text: item.text, count: (prev.count ?? 1) + 1 };
    } else out.push(item);
  }
  return out;
}

/** The key of the newest error in the whole timeline (the only one that renders as a full card), if any. */
export function newestErrorKey(turns: readonly Turn[]): string | undefined {
  for (let t = turns.length - 1; t >= 0; t--) {
    const segments = turns[t]!.segments;
    for (let s = segments.length - 1; s >= 0; s--) {
      const work = segments[s]!.work;
      for (let w = work.length - 1; w >= 0; w--) if (work[w]!.kind === 'error') return work[w]!.key;
    }
  }
  return undefined;
}

/** Merges consecutive same-class tools (≥ 2) into groups; thinking between two of them joins the group. Identical consecutive errors fold (`foldErrors`). */
export function groupWork(unfolded: readonly WorkItem[]): WorkNode[] {
  const work = foldErrors(unfolded);
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

/**
 * When a turn STARTED. A user message's own `timestamp` is when it was typed; the entry is appended
 * when the host delivers it. A queued follow-up waits (maybe minutes) between the two, and its turn
 * only begins at delivery, so "Done in" counts from the entry time — never from the queueing. A
 * normal prompt is delivered within milliseconds of being typed, so nothing changes for it.
 * (Same 6 h sanity window as `doneOf`.)
 */
const turnStartOf = (e: SessionEntry): number | undefined => {
  const at = tsOf(e);
  if (at === undefined || e.type !== 'message' || e.message?.role !== 'user' || !e.timestamp) return at;
  const t = Date.parse(e.timestamp);
  return Number.isFinite(t) && t > at && t - at < 6 * 3_600_000 ? t : at;
};

const isUser = (e: SessionEntry | undefined): boolean => e?.type === 'message' && e.message?.role === 'user';

/** Explicit non-user authors and legacy harness provenance never become user steering. */
export function messageAuthor(message: AgentMessage | undefined): string | undefined {
  if (!message) return undefined;
  const details = message.details as Record<string, unknown> | undefined;
  const raw = message.author ?? details?.author;
  if (typeof raw === 'string' && raw && raw.toLowerCase() !== 'user') return raw;
  if (message.oinoProvenance || message.unipiHarness || details?.oinoProvenance || details?.unipiHarness) return 'Host';
  return undefined;
}

/** Actual user follow-up delivered while the run continued. */
export function isSteer(prevMessage: AgentMessage | undefined, message: AgentMessage, branch: readonly SessionEntry[] = [], at = -1): boolean {
  if (messageAuthor(message)) return false;
  if (!prevMessage) return false;
  if (prevMessage.role !== 'toolResult' && !(prevMessage.role === 'assistant' && prevMessage.stopReason === 'toolUse')) return false;
  if (runEndedBefore(branch, at)) return false;
  if (prevMessage.role === 'toolResult') return !staleRun(prevMessage, message, branch, at);
  return toolCallsSettleLater(prevMessage, branch, at);
}

/** A steer is typed while its run still goes, so it lands within moments of the tool result it follows. */
const STALE_RUN_MS = 15 * 60_000;

/**
 * r4 D12: a run cut off before `oino.agent-end` / `oino.turn-interrupted` existed leaves no marker
 * at all. A user message that follows its last tool result by more than STALE_RUN_MS cannot be a
 * steer of that run (nothing was running): it is a new prompt. Only the recorded times decide, so
 * the answer is the same live, reopened and after a reconnect.
 */
function staleRun(prev: AgentMessage, message: AgentMessage, branch: readonly SessionEntry[], at: number): boolean {
  const before = typeof prev.timestamp === 'number' ? prev.timestamp : undefined;
  const typed = typeof message.timestamp === 'number' ? message.timestamp : at >= 0 && branch[at] ? tsOf(branch[at]!) : undefined;
  return before !== undefined && typed !== undefined && typed - before > STALE_RUN_MS;
}

/**
 * Core persists `oino.agent-end` when a run ends on a user / tool-result / tool-call tail (the
 * loop guard, a stop after a tool), and `oino.turn-interrupted` when resume repair finds a run
 * cut off; the guard's own `oino.loop-notice` message ends the run too. Any of them between the
 * previous message and `branch[at]` means that run is over: the user message is a new prompt,
 * never a steer of it ("Worked for 19m" spanning both prompts).
 */
function runEndedBefore(branch: readonly SessionEntry[], at: number): boolean {
  for (let i = at - 1; i >= 0; i--) {
    const e = branch[i]!;
    if (e.type === 'message') return false;
    if (e.type === 'custom' && (e.customType === AGENT_END_TYPE || e.customType === INTERRUPTED_TYPE)) return true;
    if (e.type === 'custom_message' && e.customType === LOOP_NOTICE_TYPE) return true;
  }
  return false;
}

/**
 * A user message right after an assistant `toolUse` is a steer only while that call is still
 * in flight: its result lands after the message, or the message is the newest one. A call
 * that never got a result (an `ask_user` left unanswered when its run was stopped or the host
 * restarted) is orphaned, so the owner's next prompt is a new turn. Calling it a steer glued
 * every later prompt and reply onto the old turn, which then read "Working" for good.
 */
function toolCallsSettleLater(assistant: AgentMessage, branch: readonly SessionEntry[], at: number): boolean {
  if (at < 0) return true;
  const ids = new Set<string>();
  if (Array.isArray(assistant.content)) {
    for (const part of assistant.content as ModelPart[]) {
      const id = (part as { id?: unknown }).id;
      if (part.type === 'toolCall' && typeof id === 'string') ids.add(id);
    }
  }
  // No call ids to match (older or partial transcripts): keep the plain steer rule.
  if (ids.size === 0) return true;
  let more = false;
  for (let i = at + 1; i < branch.length; i++) {
    const m = branch[i]!.message;
    if (branch[i]!.type !== 'message' || !m) continue;
    more = true;
    if (m.role === 'toolResult' && typeof m.toolCallId === 'string' && ids.has(m.toolCallId)) return true;
  }
  return !more;
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

/** `at`: the entry that opens the turn (its user message). `lead` (D4): where the turn really begins — the command echo of an extension prompt (`/swarm …`) that landed before the hooks' own entries and the user message. */
type Start = { at: number; kind: Turn['kind']; lead?: number; /** D3: a mode command's echo with no user message (yet): the typed command is the turn's only bubble. */ echoOnly?: true };

/** Where a turn's entries begin. */
const startOf = (s: Start): number => s.lead ?? s.at;

const isExtensionAuthor = (m: AgentMessage | undefined): boolean => messageAuthor(m)?.toLowerCase() === 'extension';

/**
 * D4: an extension prompt (`/goal`, `/swarm`, `/graph`, `/ralph`) records its command echo first and the user message
 * only after every `before_agent_start` hook ran, with custom / system / unipi-response / run-begun / reminder / memory /
 * skills entries (and a busy-refusal note) in between. The echo opens the turn of the next extension-authored user message —
 * the nearest one back, stopping at the previous real message (user, assistant, tool result) or compaction, and never at or
 * before `floor` (the previous turn's own first entry).
 */
function leadOf(branch: readonly SessionEntry[], at: number, floor: number): number | undefined {
  if (!isExtensionAuthor(branch[at]?.message)) return undefined;
  for (let j = at - 1; j > floor; j--) {
    const e = branch[j]!;
    if (e.type === 'custom' && ECHO_TYPES.has(String(e.customType))) return j;
    if (e.type === 'compaction' || e.type === 'compaction_state') return undefined;
    if (e.type === 'message') {
      const role = e.message?.role;
      if (role === 'user' || role === 'assistant' || role === 'toolResult' || role === 'bashExecution') return undefined;
    }
  }
  return undefined;
}

/** Turn starts in `branch[from..]`; `branch[from]` itself is a start when `firstKind` is given. `floor`: the first entry of the turn before `from` (−1: none). */
function scanStarts(branch: readonly SessionEntry[], from: number, firstKind?: Turn['kind'], floor = -1): Start[] {
  const starts: Start[] = [];
  const push = (at: number, kind: Turn['kind']) => {
    const lead = kind === 'turn' ? leadOf(branch, at, floor) : undefined;
    starts.push(lead === undefined ? { at, kind } : { at, kind, lead });
    floor = at;
  };
  let prev: AgentMessage | undefined;
  const echoes: number[] = [];
  for (let i = from; i < branch.length; i++) {
    const e = branch[i]!;
    if (e.type === 'custom' && ECHO_TYPES.has(String(e.customType)) && modeEchoKey(e)) echoes.push(i);
    if (i === from) {
      const kind = firstKind ?? (isUser(e) ? 'turn' : (e.type === 'compaction' || e.type === 'compaction_state') ? 'compaction' : 'preamble');
      push(i, kind);
      if (e.type === 'message' && e.message) prev = e.message;
      continue;
    }
    if (e.type === 'message' && e.message) {
      if (e.message.role === 'user' && !isSteer(prev, e.message, branch, i)) push(i, 'turn');
      prev = e.message;
    } else if ((e.type === 'compaction' || e.type === 'compaction_state') && isMarkerCompaction(prev)) {
      push(i, 'compaction');
    }
  }
  // D3: a `/goal` `/swarm` `/graph` `/ralph <text>` echo that no user message claimed and that nothing else follows (the
  // message lands only after the hooks ran, seconds later; or never) is a bubble of its own — the user's words stay on screen.
  if (echoes.length) {
    const claimed = new Set<number>();
    for (const st of starts) {
      claimed.add(st.at - 1);
      if (st.lead !== undefined) claimed.add(st.lead);
    }
    for (const c of echoes) {
      if (claimed.has(c)) continue;
      let later = false;
      for (let k = c + 1; k < branch.length && !later; k++) later = branch[k]!.type === 'message';
      if (!later) starts.push({ at: c, kind: 'turn', echoOnly: true });
    }
    starts.sort((a, b) => a.at - b.at);
  }
  return starts;
}

type Live = { streaming?: Streaming; liveTools: Readonly<Record<string, LiveTool>>; running: boolean; runStartedAt?: number };

/** The outbox key (`/swarm text`) of a mode command's echo; undefined for any other entry. */
function modeEchoKey(e: SessionEntry): string | undefined {
  const cmd = echoCommand(e);
  return cmd && !cmd.hidden ? modeCommandKey(cmd.name, cmd.args) : undefined;
}

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
  const from = startOf(start);
  /** Where the user message sits in `entries` (after its command echo and the hooks' entries, D4). */
  const userIdx = start.at - from;
  const entries = branch.slice(from, end) as SessionEntry[];
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
      ...(tool ? { callId: tool.id, toolName: tool.name, args: tool.args, state: tool.state, ...(tool.output !== undefined ? { output: tool.output } : {}) } : {}),
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
  const segMeta: Array<{ startedAt?: number; endedAt?: number; stopped: boolean; error: boolean; steps: number }> = [{ startedAt: turnStartOf(first), stopped: false, error: false, steps: 0 }];
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
  /** The long-horizon mode in force at this point of the branch (resolved lazily). */
  let lhMode: string | undefined;
  let model: string | undefined;
  let lastAssistant: AgentMessage | undefined;
  /** The last assistant message finished its reply (no tool calls pending). */
  let answered = false;
  /** A reply after a finished answer (woken by a background notification, a cron nudge, …)
   *  opens its own segment, so the earlier answer is not folded into "Worked". */
  const wake = (key: string, at: number | undefined) => {
    if (!answered) return;
    answered = false;
    afterAsk = undefined;
    seg = { id: `wake:${key}`, index: segments.length, work: [], nodes: [], live: false };
    segments.push(seg);
    segMeta.push({ startedAt: at, endedAt: at, stopped: false, error: false, steps: 0 });
  };
  let endedAt: number | undefined;
  // r9 P3-2: time the host was down between the last step and an Interrupted marker is not work ("Done in" skips it, like the folds do).
  let outageMs = 0;
  const turn: Turn = { turnId: start.kind === 'turn' ? first.id : start.kind === 'compaction' ? `cmp:${first.id}` : `pre:${first.id}`, kind: start.kind, entries, segments, live: false, stopped: false, source: 'user' };
  /** Index (in `entries`) of the last assistant message / of the last reference-only stub: an `ask_user` call before either has been left behind. */
  let lastAssistantAt = -1;
  let lastStubAt = -1;
  entries.forEach((e, k) => {
    if (e.message?.role === 'assistant') lastAssistantAt = k;
    else if (!e.type && e.truncated === true) lastStubAt = k;
  });
  /** The entry being read by `pushParts` (its index in `entries`). */
  let entryAt = -1;
  const pushParts = (key: string, content: unknown, isLive: boolean, stopped: boolean, callAt?: number) => {
    let stepStart = callAt;
    const parts = answerParts(content);
    parts.forEach((part, i) => {
      const k = `${key}-${i}`;
      if (part.type === 'thinking') {
        const text = stripTruncationNote(String((part as { thinking?: unknown }).thinking ?? ''));
        if (!text.trim() && !isLive) return;
        cur().work.push({ kind: 'thinking', key: k, text, live: isLive && i === parts.length - 1 });
      } else if (part.type === 'toolCall') {
        const call = part as { id?: unknown; name?: unknown; arguments?: unknown };
        const id = String(call.id ?? '');
        const result = results.get(id);
        const lt = live?.liveTools[id];
        const tool: ToolRun = { id, name: String(call.name ?? ''), args: parseArgs(call.arguments), state: 'running', ...(isLive ? {} : { callEntryId: key }) };
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
        else if (call.name === ASK_TOOL && entryAt >= 0 && lastStubAt > entryAt) tool.state = 'ok'; // its result is a reference-only stub the app is still fetching: answered
        else if (call.name === ASK_TOOL && entryAt >= 0 && lastAssistantAt > entryAt) tool.state = 'stopped'; // the agent went on without a result: the question is over, never "waiting"
        else tool.state = stopped || !running ? 'stopped' : 'running';
        if (lt?.durationMs !== undefined) tool.durationMs = lt.durationMs;
        if (isAgentTool(tool.name)) {
          const details = result ? result.message.details : lt?.details;
          cur().work.push(agentItem(k, tool, details, lt?.startedAt ?? callAt));
          return;
        }
        pushTool(k, tool, result?.at ?? callAt);
      } else if (part.type === 'text') {
        const text = stripTruncationNote(String((part as { text?: unknown }).text ?? ''));
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
    if (i === 0 && start.echoOnly) {
      turn.command = echoCommand(e);
      turn.echoKey = modeEchoKey(e);
      const echoId = (e.data as { messageId?: unknown } | undefined)?.messageId;
      if (typeof echoId === 'string' && echoId) turn.echoMessageId = echoId;
      turn.source = 'harness';
      continue;
    }
    if (i === userIdx && start.kind === 'turn') {
      turn.user = e;
      const m = e.message!;
      const text = textOf(m.content as string | ModelPart[] | undefined);
      turn.command = echoCommand(start.lead !== undefined ? branch[start.lead] : branch[start.at - 1]) ?? commandOf(text, e.id);
      if (isHiddenCommandText(text)) turn.command = turn.command ? { ...turn.command, hidden: true } : { name: 'continue', args: '', entryId: e.id, hidden: true };
      if (isSummaryRequest(m)) {
        // OIN-19: the summarizer prompt sent as a chat message is a compaction card, not a bubble.
        turn.command = { name: 'compact', args: '', entryId: e.id, hidden: true };
        cur().work.push({ kind: 'card', key: e.id, entry: e });
      }
      const hasProvenance = !!(m.unipiHarness || (m.details as { oinoProvenance?: unknown; unipiHarness?: unknown } | undefined)?.oinoProvenance || (m.details as { unipiHarness?: unknown } | undefined)?.unipiHarness);
      turn.source = m.source === 'cron' || m.unipiCron ? 'cron' : hasProvenance || messageAuthor(m) ? 'harness' : 'user';
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
        answered = false;
        const request = isSummaryRequest(m);
        // r7 P2-5: an internal resume instruction landing mid-turn is no bubble either.
        const quiet = request || isHiddenCommandText(textOf(m.content as string | ModelPart[] | undefined));
        seg = { id: e.id, index: segments.length, ...(quiet ? {} : { steer: e }), work: [], nodes: [], live: false };
        segments.push(seg);
        segMeta.push({ startedAt: at, endedAt: at, stopped: false, error: false, steps: 0 });
        if (request) seg.work.push({ kind: 'card', key: e.id, entry: e });
      } else if (m.role === 'assistant') {
        wake(e.id, at);
        answered = m.stopReason === 'stop' || m.stopReason === 'length';
        lastAssistant = m;
        cost += m.usage?.cost?.total ?? 0;
        if (typeof m.model === 'string') model = m.model;
        const stopped = stoppedOf(m);
        entryAt = i;
        pushParts(e.id, m.content, false, stopped, at);
        entryAt = -1;
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
        if (cmd?.hidden || (start.lead !== undefined && i === 0) || (from + i + 1 === end && isUser(branch[end]))) continue;
      }
      if (ct === LH_DECISION) {
        // OIN-19: a decision repeating the branch's current mode (every prompt of a sticky
        // `/graph`, each retry's re-run gate) is not a switch: one "Mode · Graph" per change.
        const mode = lhModeOf(e);
        const prev = lhMode ?? lhModeBefore(branch, from);
        lhMode = mode;
        if (mode === prev) continue;
      }
      if (INTERNAL_CUSTOM_TYPES.has(ct) && !eventVisible(e)) continue;
      if (ct === NOTE_TYPE || ct === INTERRUPTED_TYPE) {
        // The owner's note (and perf T13's "Interrupted — Continue" marker) is its own row, in order — not a user message (no turn, no outbox settle) and never
        // inside a "Worked" fold: it opens a segment of its own (its card is lifted above that segment's fold).
        if (!eventVisible(e)) continue;
        if (ct === INTERRUPTED_TYPE && at !== undefined && endedAt !== undefined && at > endedAt) outageMs += at - endedAt;
        if (seg.work.length || seg.steer) {
          afterAsk = undefined;
          seg = { id: `${ct === NOTE_TYPE ? 'note' : 'interrupted'}:${e.id}`, index: segments.length, work: [], nodes: [], live: false };
          segments.push(seg);
          segMeta.push({ startedAt: at, endedAt: at, stopped: false, error: false, steps: 0 });
        }
        seg.work.push({ kind: 'card', key: e.id, entry: e });
        continue;
      }
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
    } else if ((e.type === 'compaction' || e.type === 'compaction_state')) {
      cur().work.push({ kind: 'compaction', key: e.id, entry: e });
    }
  }
  if (live) {
    if (live.streaming) {
      const m = live.streaming.message;
      wake('stream', undefined);
      pushParts('stream', m.content, true, false);
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
  turn.startedAt = turnStartOf(first);
  if (!running) {
    turn.endedAt = endedAt;
    if (turn.startedAt !== undefined && endedAt !== undefined) turn.durationMs = Math.max(0, endedAt - turn.startedAt - outageMs);
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
  mergeFolds(segments, segMeta.map((m) => m.steps));
  return turn;
}

/**
 * OIN-45: an assistant message's parts as the timeline sees them. A bare-string `content` is one
 * text part (it used to vanish), and adjacent text blocks are ONE part, so the message's closing
 * text is a single answer instead of its first block hiding in the fold.
 */
function answerParts(content: unknown): ModelPart[] {
  if (typeof content === 'string') return content.trim() ? [{ type: 'text', text: content }] : [];
  if (!Array.isArray(content)) return [];
  const out: ModelPart[] = [];
  for (const part of content as ModelPart[]) {
    const prev = out.at(-1) as { type?: string; text?: unknown } | undefined;
    if (part.type === 'text' && prev?.type === 'text') {
      out[out.length - 1] = { type: 'text', text: `${String(prev.text ?? '')}\n\n${String((part as { text?: unknown }).text ?? '')}` };
    } else out.push(part);
  }
  return out;
}

const FOLD_RANK: Record<FoldState, number> = { done: 0, error: 1, stopped: 2, running: 3 };

/**
 * OIN-96: consecutive "Worked" rows with no visible prose between them are ONE row.
 *
 * 1. A segment whose fold has no visible content after it (no answer prose, no `ask_user`
 *    card, no trailing cards) and whose successor opens with no steer bubble and no event cards
 *    is absorbed into that successor: consecutive folds (a swarm supervisor waking many times)
 *    become ONE fold — total duration, tool counts and failures summed, every step inside in
 *    order. The successor's answer / ask / after stay outside the fold, so the final text never
 *    hides.
 * 2. When the earlier segment ends on its answer plus work that trails it (`after`: the row
 *    `afterRuns` draws as its own "Worked"), and nothing visible follows that work, the trailing
 *    run moves to the front of the successor's fold: the answer stays above ONE row, not two.
 *
 * Mutates `segments` (the turn's own array); `steps[i]` = delegated steps of segment i.
 */
const detailOf = (work: readonly WorkItem[], steps: number): string =>
  [agentsSummary(work.flatMap((w) => (w.kind === 'agent' ? [w.run] : []))), foldDetail(work.flatMap((w) => (w.kind === 'tool' ? [w.tool] : [])), steps)].filter(Boolean).join(' · ');

/** The items a run of `WorkNode`s was grouped from (inverse of `groupWork`). */
const itemsOf = (nodes: readonly WorkNode[]): WorkItem[] => nodes.flatMap((n): WorkItem[] => (n.kind === 'group' || n.kind === 'agents' ? n.items : [n]));

/**
 * Rule 2 of `mergeFolds`: the work trailing `a`'s answer (its last `afterRuns` run, when nothing
 * visible follows it) moves to the front of `s`'s fold. Returns the delegated steps that moved
 * (`undefined`: nothing moved).
 */
function hoistTrailingRun(a: Segment, s: Segment): number | undefined {
  const runs = afterRuns(a.after!);
  const last = runs.at(-1);
  if (last?.kind !== 'work') return undefined;
  const moved = itemsOf(last.nodes);
  const rest = runs.slice(0, -1).flatMap((r) => (r.kind === 'plain' ? [r.node] : r.nodes));
  s.work = [...moved, ...s.work];
  s.nodes = groupWork(s.work);
  if (rest.length) a.after = rest;
  else delete a.after;
  return moved.filter((w) => w.kind === 'step').length;
}

function mergeFolds(segments: Segment[], steps: number[]): void {
  const out: Segment[] = [];
  const outSteps: number[] = [];
  for (const s of segments) {
    let stepsOf = steps[s.index] ?? 0;
    const a = out.at(-1);
    const joinable = !!s.fold && !s.steer && !s.before?.length;
    if (a && joinable && a.answer && !a.ask && a.after?.length) {
      const movedSteps = hoistTrailingRun(a, s);
      if (movedSteps !== undefined) {
        const aSteps = Math.max(0, outSteps[out.length - 1]! - movedSteps);
        outSteps[out.length - 1] = aSteps;
        stepsOf += movedSteps;
        if (a.fold) a.fold = { ...a.fold, detail: detailOf(a.work, aSteps) };
        s.fold = { ...s.fold!, detail: detailOf(s.work, stepsOf) };
      }
    }
    if (a?.fold && !a.answer && !a.ask && !a.after?.length && s.fold && !s.steer && !s.before?.length) {
      const work = [...a.work, ...s.work];
      const state = [a.fold, s.fold].reduce<FoldState>((acc, f) => (FOLD_RANK[f.state] > FOLD_RANK[acc] ? f.state : acc), 'done');
      const durations = [a.fold.durationMs, s.fold.durationMs];
      const durationMs = state === 'done' || state === 'error' ? (durations.some((d) => d !== undefined) ? durations.reduce<number>((n, d) => n + (d ?? 0), 0) : undefined) : undefined;
      const totalSteps = outSteps[out.length - 1]! + stepsOf;
      const priorMs = a.fold.durationMs ?? 0;
      const startedAt = s.fold.startedAt !== undefined ? s.fold.startedAt - priorMs : a.fold.startedAt;
      out[out.length - 1] = {
        ...a,
        work,
        nodes: groupWork(work),
        ...(s.answer ? { answer: s.answer } : {}),
        ...(s.ask ? { ask: s.ask } : {}),
        ...(s.after ? { after: s.after } : {}),
        live: s.live,
        fold: {
          state,
          label: state === 'running' ? 'Working' : state === 'stopped' ? 'Stopped' : durationMs === undefined ? 'Worked' : `Worked for ${formatDuration(durationMs)}`,
          detail: detailOf(work, totalSteps),
          ...(durationMs !== undefined ? { durationMs } : {}),
          ...(startedAt !== undefined ? { startedAt } : {}),
          forcedOpen: state !== 'done',
        },
      };
      outSteps[out.length - 1] = totalSteps;
    } else {
      out.push(s);
      outSteps.push(stepsOf);
    }
  }
  segments.splice(0, segments.length, ...out);
}

/** One work run of a segment's trailing nodes, or a plain row next to it (see `afterRuns`). */
export type AfterRun =
  | { kind: 'work'; key: string; nodes: WorkNode[]; detail: string }
  | { kind: 'plain'; key: string; node: WorkNode };

/**
 * The work trailing a segment's answer, as the rows under it: ONE "Worked" fold per run of work.
 * Event cards that sit BETWEEN two pieces of work (a background-task result, a memory-save
 * card) go inside that fold's step list; cards before the first / after the last piece of work
 * (and compaction dividers, which end the run) stay plain rows next to it. So between two
 * pieces of real prose there is never more than one fold.
 */
export function afterRuns(nodes: readonly WorkNode[]): AfterRun[] {
  const out: AfterRun[] = [];
  /** Drawn cards seen since the last piece of work: absorbed if more work follows, else plain. */
  let held: Array<Extract<WorkNode, { kind: 'card' }>> = [];
  const flush = () => {
    for (const card of held) out.push({ kind: 'plain', key: card.key, node: card });
    held = [];
  };
  const toolsOf = (list: readonly WorkNode[]) => list.flatMap((n): ToolRun[] => n.kind === 'tool' ? [n.tool] : n.kind === 'group' ? n.tools : []);
  for (const node of nodes) {
    if (node.kind === 'card') {
      // A card that draws nothing (core state entry) must not split a work run.
      if (rendersCard(node)) held.push(node);
      continue;
    }
    if (node.kind === 'compaction') {
      flush();
      out.push({ kind: 'plain', key: node.key, node });
      continue;
    }
    const prev = out.at(-1);
    if (prev?.kind === 'work') {
      prev.nodes.push(...held, node);
      held = [];
      prev.detail = foldDetail(toolsOf(prev.nodes));
    } else {
      flush();
      out.push({ kind: 'work', key: node.key, nodes: [node], detail: foldDetail(toolsOf([node])) });
    }
  }
  flush();
  return out;
}

/** Whether `CardRow` draws this card (same early branches + `describeEvent`). */
function rendersCard(node: Extract<WorkNode, { kind: 'card' }>): boolean {
  const e = node.entry;
  if (e.type === 'model_change' || e.type === 'thinking_level_change') return true;
  if (isCwdChange(e)) return true;
  return describeEvent(e) !== null;
}

const LH_DECISION = 'long-horizon-decision';
const lhModeOf = (e: SessionEntry): string => String((e.data as { mode?: unknown } | undefined)?.mode ?? 'none');

/** The mode of the last long-horizon decision before `branch[at]` (`none` = the plain default). */
function lhModeBefore(branch: readonly SessionEntry[], at: number): string {
  for (let i = at - 1; i >= 0; i--) {
    const e = branch[i]!;
    if (e.type === 'custom' && e.customType === LH_DECISION) return lhModeOf(e);
  }
  return 'none';
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
    if (live && item.kind !== 'card' && item.kind !== 'compaction') break;
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
  return withRenderCutoff(starts.map((s, i) => buildTurn(branch, s, endOf(starts, i, branch.length), i === starts.length - 1 ? liveOf(input) : undefined)), branch);
}

/** Select only a branch-local, existing inclusive anchor. Never slice human history. */
function withRenderCutoff(turns: Turn[], branch: readonly SessionEntry[]): Turn[] {
  const anchorOf = (e: SessionEntry) => e.type === 'compaction_state' ? (e.anchorMessage as { id?: string } | undefined)?.id : typeof e.firstKeptEntryId === 'string' ? e.firstKeptEntryId : undefined;
  const state = [...branch].reverse().find((e) => (e.type === 'compaction_state' || e.type === 'compaction') && branch.some((m) => m.id === anchorOf(e)));
  const anchor = state && anchorOf(state);
  return turns.map((turn) => {
    const cutoff = state && turn.entries.some((e) => e.id === anchor) ? state : undefined;
    if (turn.renderCutoff === cutoff) return turn;
    return { ...turn, renderCutoff: cutoff };
  });
}

/** Where turn `i` ends: where the next one begins (its command echo, when it has one). */
const endOf = (starts: readonly Start[], i: number, length: number): number => (starts[i + 1] ? startOf(starts[i + 1]!) : length);

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
  /** How the last update ran (tests / perf). `stream`: only streamed text grew — the tail turn was patched, not rebuilt. */
  readonly lastMode: 'full' | 'tail' | 'stream' | 'none';
};

/**
 * perf-webview (spec §4.7): what the tail turn was last built from, for the stream fast path.
 * Values are copied out (the store patches the streaming message and live tools in place).
 */
type LiveShape = {
  running: boolean;
  startedAt?: number;
  /** `answerParts` of the streaming message: type + text for prose/thinking, a fingerprint otherwise. */
  parts: Array<{ type: string; text: string; sig: unknown }>;
  tools: Array<{ ref: LiveTool; id: string; name: string; done?: boolean; isError?: boolean; partial?: string; details?: unknown; durationMs?: number; parentId?: string }>;
};

const isStreamText = (type: string) => type === 'text' || type === 'thinking';

function liveShape(input: TurnsInput): LiveShape | undefined {
  const m = input.streaming?.message;
  if (!m) return undefined;
  return {
    running: input.run.running,
    startedAt: input.run.startedAt,
    parts: answerParts(m.content).map((p) => {
      const part = p as { type: string; text?: unknown; thinking?: unknown; id?: unknown; name?: unknown; arguments?: unknown; partialJson?: unknown };
      if (isStreamText(part.type)) return { type: part.type, text: String((part.type === 'text' ? part.text : part.thinking) ?? ''), sig: undefined };
      return { type: part.type, text: '', sig: part.type === 'toolCall' ? `${String(part.id)}|${String(part.name)}|${String(part.partialJson ?? '')}|${JSON.stringify(part.arguments ?? null)}` : p };
    }),
    tools: Object.values(input.liveTools).map((t) => ({ ref: t, id: t.id, name: t.name, done: t.done, isError: t.isError, partial: t.partial, details: t.details, durationMs: t.durationMs, parentId: t.parentId })),
  };
}

/** Same turn structure: only the text of streamed prose / thinking parts may differ (an empty part stays empty). */
function sameShape(a: LiveShape, b: LiveShape): boolean {
  if (a.running !== b.running || a.startedAt !== b.startedAt || a.parts.length !== b.parts.length || a.tools.length !== b.tools.length) return false;
  for (let i = 0; i < a.parts.length; i++) {
    const x = a.parts[i]!;
    const y = b.parts[i]!;
    if (x.type !== y.type || x.sig !== y.sig) return false;
    if (isStreamText(x.type) && x.type === 'text' && !x.text.trim() !== !y.text.trim()) return false;
  }
  for (let i = 0; i < a.tools.length; i++) {
    const x = a.tools[i]!;
    const y = b.tools[i]!;
    if (x.ref !== y.ref || x.id !== y.id || x.name !== y.name || x.done !== y.done || x.isError !== y.isError || x.partial !== y.partial || x.details !== y.details || x.durationMs !== y.durationMs || x.parentId !== y.parentId) return false;
  }
  return true;
}

/**
 * The tail turn with the streamed parts' new text, sharing every untouched object: only the
 * `stream-<i>` items, the arrays / groups / segments holding them and the turn itself are new.
 * Undefined when an item cannot be found (the caller rebuilds the turn instead).
 */
function patchStreamText(turn: Turn, shape: LiveShape): Turn | undefined {
  const text = new Map<string, string>();
  shape.parts.forEach((p, i) => isStreamText(p.type) && text.set(`stream-${i}`, p.text));
  let found = 0;
  const item = <T extends WorkItem>(w: T): T => {
    if ((w.kind !== 'prose' && w.kind !== 'thinking') || !text.has(w.key)) return w;
    found++;
    const next = text.get(w.key)!;
    return (w.text === next ? w : { ...w, text: next }) as T;
  };
  const items = <T extends WorkItem>(list: T[]): T[] => {
    let out: T[] | undefined;
    list.forEach((w, i) => {
      const n = item(w);
      if (n !== w) (out ??= list.slice())[i] = n;
    });
    return out ?? list;
  };
  const nodes = (list: WorkNode[] | undefined): WorkNode[] | undefined => {
    if (!list) return list;
    let out: WorkNode[] | undefined;
    list.forEach((n, i) => {
      let m: WorkNode = n;
      if (n.kind === 'group' || n.kind === 'agents') {
        const inner = items(n.items);
        if (inner !== n.items) m = { ...n, items: inner } as WorkNode;
      } else m = item(n as WorkItem) as WorkNode;
      if (m !== n) (out ??= list.slice())[i] = m;
    });
    return out ?? list;
  };
  const before = found;
  const segments = turn.segments.map((s) => {
    const work = items(s.work);
    const answer = s.answer ? item(s.answer) : undefined;
    const ns = nodes(s.nodes)!;
    const b = nodes(s.before);
    const a = nodes(s.after);
    if (work === s.work && answer === s.answer && ns === s.nodes && b === s.before && a === s.after) return s;
    return { ...s, work, nodes: ns, ...(answer ? { answer } : {}), ...(b ? { before: b } : {}), ...(a ? { after: a } : {}) };
  });
  // Every streamed text part must have been located at least once (work or answer).
  for (const key of text.keys()) {
    const seen = turn.segments.some((s) => s.answer?.key === key || s.work.some((w) => w.key === key));
    if (!seen) return undefined;
  }
  if (found === before) return turn;
  return segments.every((s, i) => s === turn.segments[i]) ? turn : { ...turn, segments };
}

export function createTurnsIndex(): TurnsIndex {
  let rev = -1;
  let source: readonly SessionEntry[] = [];
  let sourceLen = 0;
  let leaf: string | null = null;
  let branch: SessionEntry[] = [];
  let starts: Start[] = [];
  let turns: Turn[] = [];
  let mode: TurnsIndex['lastMode'] = 'none';
  /** What the current tail turn was built from (stream fast path). */
  let shape: LiveShape | undefined;

  const full = (input: TurnsInput) => {
    mode = 'full';
    branch = branchOf(input.entries, input.leafId).slice();
    starts = scanStarts(branch, 0);
    const prev = new Map(turns.map((t) => [t.turnId, t]));
    const live = liveOf(input);
    turns = starts.map((s, i) => {
      const end = endOf(starts, i, branch.length);
      const tail = i === starts.length - 1;
      const old = prev.get(turnIdOf(branch, s));
      if (!tail && old && !old.live && sameEntries(old.entries, branch, startOf(s), end)) return old;
      return buildTurn(branch, s, end, tail ? live : undefined);
    });
  };

  const tail = (input: TurnsInput) => {
    mode = 'tail';
    for (let i = sourceLen; i < input.entries.length; i++) branch.push(input.entries[i]!);
    // A bare echo turn (D3) is re-derived with the rest: its message may have just landed and claim it.
    if (starts[starts.length - 1]?.echoOnly) starts = starts.slice(0, -1);
    const lastStart = starts[starts.length - 1];
    const from = lastStart?.at ?? 0;
    const fresh = scanStarts(branch, from, lastStart?.kind, starts[starts.length - 2]?.at ?? -1);
    starts = starts.slice(0, Math.max(0, starts.length - 1)).concat(fresh);
    const keep = starts.length - fresh.length;
    const live = liveOf(input);
    const next = turns.slice(0, keep);
    for (let i = keep; i < starts.length; i++) {
      const end = endOf(starts, i, branch.length);
      const isTail = i === starts.length - 1;
      const old = turns[i];
      // The previous tail, settled and unchanged, keeps its identity once a new turn follows it.
      if (!isTail && old && !old.live && old.turnId === turnIdOf(branch, starts[i]!) && sameEntries(old.entries, branch, startOf(starts[i]!), end)) next.push(old);
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
      const next = appended && input.entries.length === sourceLen ? liveShape(input) : undefined;
      const last = turns[turns.length - 1];
      const patched = next && shape && last?.live && sameShape(shape, next) ? patchStreamText(last, next) : undefined;
      if (patched) {
        mode = 'stream';
        turns = turns.slice(0, -1).concat(patched);
      } else if (appended) tail(input);
      else full(input);
      shape = next ?? liveShape(input);
      rev = input.entriesRev;
      source = input.entries;
      sourceLen = input.entries.length;
      leaf = input.leafId;
      turns = withRenderCutoff(turns, branch);
      return turns;
    },
    get lastMode() {
      return mode;
    },
  };
}

/** Collapse adjacent standalone attributed notifications without hiding turn content. */
export function collapseHarnessTurns(turns: readonly Turn[]): Turn[] {
  const result: Turn[] = [];
  for (const turn of turns) {
    const prev = result.at(-1);
    const author = messageAuthor(turn.user?.message);
    const plain = (t: Turn) => !t.live && !t.command && t.segments.every((s) => !s.steer && !s.work.length && !s.answer);
    if (author && prev && plain(turn) && plain(prev) && author === messageAuthor(prev.user?.message) &&
        textOf(turn.user?.message?.content as string | ModelPart[] | undefined) === textOf(prev.user?.message?.content as string | ModelPart[] | undefined)) {
      result[result.length - 1] = { ...prev, harnessCount: (prev.harnessCount ?? 1) + 1 };
    } else result.push(turn);
  }
  return result;
}
