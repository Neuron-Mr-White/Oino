/**
 * Session reducer (app spec §3.4, WP-A2): host session snapshot / replay /
 * live events → `SessionState`. Pure over a mutable draft: the store runs it
 * inside Solid's `produce` (one `batch()` per animation frame), tests run it
 * on plain objects.
 *
 * - `applySnapshot` — `reset:true` rebuilds from the snapshot; `reset:false`
 *   applies the `replay` in order.
 * - `applyEvent` — one `{session, seq, event}`. Duplicates (`seq ≤ state.seq`)
 *   are skipped; a gap that coalesced/dropped text deltas cannot explain
 *   returns `gap` (the store reopens with `sinceSeq`).
 * - `message_end` appends a **provisional** entry (`live:<seq>`, host entries
 *   are persisted without an event); `agent_end` asks the store to
 *   `reconcile` the tail with `get_entries{since}` so the state equals a
 *   fresh snapshot (real ids for fork / tree / edit).
 */
import type { SessionEventDto, SessionSnapshot } from '../host/client';
import { hostEventBanner } from '@unipi/protocol';
import { settleByMessage, settleByQueue } from './outbox';
import {
  LIVE_PREFIX,
  isProvisional,
  type AgentMessage,
  type BannerKind,
  type CommandInfo,
  type LiveTool,
  type Part,
  type RunPhase,
  type SessionEntry,
  type SessionState,
  type TopicValue,
} from './types';

/** What the store must do after an apply (side effects stay out of the reducer). */
export type ApplyResult = {
  /** A seq gap: reopen with `sinceSeq = state.seq`. The event was not applied. */
  gap?: boolean;
  /** The run ended: fetch `get_entries{since}` and `reconcileEntries`. */
  reconcile?: boolean;
  /** `commands_changed`: refetch `get_commands`. */
  commandsChanged?: boolean;
  /** `lagged` (core dropped events): reopen without `sinceSeq` (full snapshot). */
  resync?: boolean;
  /** `taken_over`: stop reconnecting (a reopen would take it back). */
  takenOver?: boolean;
  /** `session_replaced`: the runtime now serves this session id. */
  replacedBy?: string;
  /** `session_info_changed` / snapshot `sessionName`: sync live title to the hub card (D-03). */
  nameChanged?: string;
  /** ui-protocol registry traffic for the U9 registry (`extension_ui_request`, `_dismiss`, `oino_ui_patch`). */
  ui?: Array<Record<string, unknown>>;
  /** `woken` with a leaf past ours (the wake appended): fetch `get_entries{since}` and append. */
  catchUp?: string;
  /** [chat-tree] `session_tree` (a tree jump, any client): the leaf the branch moved to; reload its tail page unless it is ours already. */
  branchMoved?: string | null;
};

/** Extension UI methods that wait for the user (needs-you). */
const DIALOG_METHODS = new Set(['select', 'confirm', 'input', 'editor']);

export function initialState(hostId: string, sessionId: string): SessionState {
  return {
    hostId,
    sessionId,
    seq: 0,
    entries: [],
    byId: {},
    leafId: null,
    entriesRev: 0,
    liveTools: {},
    run: { phase: 'idle', running: false },
    queue: { steering: [], followUp: [] },
    topics: {},
    commands: [],
    info: {},
    dialogs: [],
    conn: 'connecting',
    outbox: [],
  };
}

const isObject = (v: unknown): v is Record<string, unknown> => !!v && typeof v === 'object' && !Array.isArray(v);
const str = (v: unknown): string | undefined => (typeof v === 'string' ? v : undefined);

function indexEntries(entries: readonly SessionEntry[]): Record<string, number> {
  const byId: Record<string, number> = {};
  for (let i = 0; i < entries.length; i++) byId[entries[i]!.id] = i;
  return byId;
}

function topicsOf(list: readonly unknown[] | undefined): Record<string, TopicValue> {
  const out: Record<string, TopicValue> = {};
  for (const raw of list ?? []) {
    if (!isObject(raw) || typeof raw.topic !== 'string') continue;
    out[raw.topic] = { data: raw.data, sticky: raw.sticky !== false, publisher: str(raw.publisher) ?? '' };
  }
  return out;
}

/** `RpcSessionState` → run / name / model / thinking. */
function applyInfo(s: SessionState, info: Record<string, unknown>): void {
  s.info = info;
  const streaming = info.isStreaming === true || info.running === true;
  const compacting = info.isCompacting === true;
  // Oino's `get_state` also says what an idle session waits on (`waitingOn`) or still runs
  // quietly (`backgroundOn`), so a chat opened mid-wait reads "Working" from the first frame.
  const strings = (v: unknown): string[] => (Array.isArray(v) ? v.filter((w): w is string => typeof w === 'string' && !!w) : []);
  const waitingOn = streaming || compacting ? [] : strings(info.waitingOn);
  const backgroundOn = streaming || compacting || waitingOn.length ? [] : strings(info.backgroundOn);
  s.run = {
    phase: compacting ? 'compacting' : streaming ? 'running' : waitingOn.length ? 'waiting' : 'idle',
    running: streaming,
    ...(streaming ? { startedAt: s.run.startedAt } : {}),
    ...(waitingOn.length ? { waitingOn } : {}),
    ...(backgroundOn.length ? { backgroundOn } : {}),
  };
  if (!streaming) delete s.run.startedAt;
  const name = str(info.sessionName);
  if (name !== undefined) s.name = name;
  if (isObject(info.model)) s.model = info.model as SessionState['model'];
  const thinking = str(info.thinkingLevel) ?? str(info.thinking);
  if (thinking !== undefined) s.thinkingLevel = thinking;
}

/**
 * The first frame of a stream. `reset:true` replaces everything the host
 * owns (outbox, conn and an open banner survive only when it is not a reset).
 */
export function applySnapshot(s: SessionState, snap: SessionSnapshot, now = Date.now()): ApplyResult {
  if (!snap.reset) {
    const merged: ApplyResult = {};
    for (const dto of snap.replay ?? []) {
      const r = applyEvent(s, dto, now);
      if (r.gap) return { ...merged, gap: true };
      mergeResult(merged, r);
    }
    s.conn = 'live';
    delete s.connError;
    return merged;
  }
  // Persisted entries are immutable: an unchanged one keeps its object (and so its proxy), so
  // the turns index keeps settled turns' identity across a reset.
  const prev = new Map<string, SessionEntry>();
  for (const e of s.entries) if (!isProvisional(e.id)) prev.set(e.id, e);
  const entries = ((snap.entries ?? []) as SessionEntry[])
    .filter((e) => isObject(e) && typeof e.id === 'string')
    .map((e) => {
      const old = prev.get(e.id);
      return old && old.type === e.type && old.timestamp === e.timestamp && old.parentId === e.parentId ? old : e;
    });
  s.seq = snap.seq;
  s.entries = entries;
  s.byId = indexEntries(entries);
  s.leafId = snap.leaf_id ?? entries.at(-1)?.id ?? null;
  s.entriesRev += 1;
  const page = snap.page;
  if (page) s.history = { total: page.total, hasMore: page.has_more, ...(page.cursor ? { cursor: page.cursor } : {}) };
  else delete s.history;
  if (snap.waking) s.waking = true;
  else delete s.waking;
  delete s.streaming;
  s.liveTools = {};
  s.queue = { steering: [], followUp: [] };
  s.topics = topicsOf(snap.topics as unknown[] | undefined);
  s.commands = (Array.isArray(snap.commands) ? snap.commands : []).filter(isObject) as CommandInfo[];
  s.dialogs = [];
  s.run = { phase: 'idle', running: false };
  applyInfo(s, isObject(snap.state) ? snap.state : {});
  delete s.banner;
  s.conn = 'live';
  delete s.connError;
  // A user message the host already holds settles its outbox bubble.
  for (const e of entries.slice(-8)) if (e.message?.role === 'user') settleByMessage(s, e.message);
  const ui = isObject(snap.ui) ? [{ type: 'oino_ui_snapshot', ...snap.ui }] : undefined;
  const out: ApplyResult = {};
  if (ui) out.ui = ui;
  if (s.name !== undefined) out.nameChanged = s.name;
  return out;
}

function mergeResult(into: ApplyResult, r: ApplyResult): void {
  if (r.reconcile) into.reconcile = true;
  if (r.commandsChanged) into.commandsChanged = true;
  if (r.resync) into.resync = true;
  if (r.takenOver) into.takenOver = true;
  if (r.replacedBy) into.replacedBy = r.replacedBy;
  if (r.nameChanged !== undefined) into.nameChanged = r.nameChanged;
  if (r.gap) into.gap = true;
  if (r.ui) (into.ui ??= []).push(...r.ui);
  if (r.catchUp) into.catchUp = r.catchUp;
  if (r.branchMoved !== undefined) into.branchMoved = r.branchMoved;
}

/** Applies a batch in order; stops at the first gap. */
export function applyEvents(s: SessionState, items: readonly SessionEventDto[], now = Date.now()): ApplyResult {
  const out: ApplyResult = {};
  for (const dto of items) {
    const r = applyEvent(s, dto, now);
    mergeResult(out, r);
    if (r.gap) break;
  }
  return out;
}

/** Text/thinking deltas the host coalesces (and may drop for a slow client) — a gap before one is benign. */
function isLossyDelta(event: Record<string, unknown>): boolean {
  if (event.type !== 'message_update') return false;
  const kind = (event.assistantMessageEvent as { type?: unknown } | undefined)?.type;
  return kind === 'text_delta' || kind === 'thinking_delta';
}

function benignGap(s: SessionState, event: Record<string, unknown>): boolean {
  if (isLossyDelta(event)) return true;
  // A dropped delta followed by the next update / the end of the same message: the full
  // `message` the event carries repairs the text.
  return !!s.streaming && (event.type === 'message_update' || event.type === 'message_end');
}

const isoOf = (ts: unknown, now: number): string => new Date(typeof ts === 'number' && Number.isFinite(ts) ? ts : now).toISOString();

/** A settled message → the entry pi would persist for it. */
function entryForMessage(message: AgentMessage, id: string, parentId: string | null, now: number): SessionEntry {
  if (message.role === 'custom') {
    const entry: SessionEntry = { type: 'custom_message', id, parentId, timestamp: isoOf(message.timestamp, now), customType: str(message.customType) ?? '', content: message.content };
    if (message.display !== undefined) entry.display = message.display;
    if (message.details !== undefined) entry.details = message.details;
    return entry;
  }
  return { type: 'message', id, parentId, timestamp: isoOf(message.timestamp, now), message };
}

function appendEntry(s: SessionState, entry: SessionEntry): void {
  const at = s.byId[entry.id];
  if (at !== undefined) {
    // Not an append: the turns index must rebuild (it reuses unchanged turns).
    s.entries[at] = entry;
    s.entriesRev += 1;
  } else {
    s.byId[entry.id] = s.entries.length;
    s.entries.push(entry);
  }
  s.leafId = entry.id;
}

function resultText(result: unknown): string | undefined {
  if (!isObject(result)) return typeof result === 'string' ? result : undefined;
  const content = result.content;
  if (typeof content === 'string') return content;
  if (!Array.isArray(content)) return undefined;
  return content
    .filter((p): p is { type: string; text: string } => isObject(p) && p.type === 'text' && typeof p.text === 'string')
    .map((p) => p.text)
    .join('');
}

/** Applies one assistant stream event to the streaming message's parts (when no full `message.content` came). */
function applyDelta(message: AgentMessage, ev: Record<string, unknown>): void {
  const kind = str(ev.type);
  if (!kind) return;
  const index = typeof ev.contentIndex === 'number' ? ev.contentIndex : -1;
  const parts = (Array.isArray(message.content) ? message.content : (message.content = [])) as Part[];
  const ensure = (fresh: () => Part): Part | undefined => {
    if (index < 0) return undefined;
    while (parts.length <= index) parts.push(fresh());
    return parts[index];
  };
  switch (kind) {
    case 'text_start':
    case 'text_delta': {
      const part = ensure(() => ({ type: 'text', text: '' }));
      if (part && kind === 'text_delta') part.text = String(part.text ?? '') + String(ev.delta ?? '');
      return;
    }
    case 'thinking_start':
    case 'thinking_delta': {
      const part = ensure(() => ({ type: 'thinking', thinking: '' }));
      if (part && kind === 'thinking_delta') part.thinking = String(part.thinking ?? '') + String(ev.delta ?? '');
      return;
    }
    case 'toolcall_start':
    case 'toolcall_delta': {
      const part = ensure(() => ({ type: 'toolCall', id: '', name: '', arguments: {} }));
      if (!part) return;
      const call = isObject(ev.toolCall) ? ev.toolCall : undefined;
      if (call) {
        if (typeof call.id === 'string') part.id = call.id;
        if (typeof call.name === 'string') part.name = call.name;
      }
      if (kind === 'toolcall_delta') {
        part.partialJson = String(part.partialJson ?? '') + String(ev.delta ?? '');
        try {
          const parsed = JSON.parse(String(part.partialJson));
          if (isObject(parsed)) part.arguments = parsed;
        } catch {
          // partial JSON while streaming
        }
      }
      return;
    }
    case 'toolcall_end': {
      const call = isObject(ev.toolCall) ? ev.toolCall : undefined;
      if (call && index >= 0) {
        while (parts.length <= index) parts.push({ type: 'toolCall', id: '', name: '', arguments: {} });
        parts[index] = { type: 'toolCall', ...call };
      }
      return;
    }
    default:
      return;
  }
}

/** The streaming message takes the event's full partial `message` (host coalescing keeps the newest). */
function takeMessage(s: SessionState, message: AgentMessage, now: number): void {
  if (!s.streaming) {
    s.streaming = { message: { ...message, content: Array.isArray(message.content) ? message.content.map((p) => ({ ...p })) : [] }, startedAt: now };
    return;
  }
  const cur = s.streaming.message;
  for (const key of Object.keys(message)) if (key !== 'content') cur[key] = message[key];
  if (!Array.isArray(message.content)) return;
  const parts = (Array.isArray(cur.content) ? cur.content : (cur.content = [])) as Part[];
  const next = message.content;
  // Patch in place: unchanged parts keep their identity (Solid re-renders the changed block only).
  for (let i = 0; i < next.length; i++) {
    const want = next[i]!;
    const have = parts[i];
    if (!have || have.type !== want.type) {
      parts[i] = { ...want };
      continue;
    }
    for (const key of Object.keys(want)) if (have[key] !== want[key]) have[key] = want[key];
  }
  if (parts.length > next.length) parts.length = next.length;
}

function setBanner(s: SessionState, kind: BannerKind, event: Record<string, unknown>, now: number): void {
  const text = hostEventBanner({ type: kind, error: event.error, from: event.from ?? event.by }) ?? '';
  s.banner = { kind, text, at: now, ...(typeof event.from === 'string' ? { from: event.from } : {}) };
}

function endRun(s: SessionState, phase: RunPhase = 'idle'): void {
  s.run = { phase, running: false };
  delete s.streaming;
  s.liveTools = {};
}

/** Applies one numbered event. */
export function applyEvent(s: SessionState, dto: SessionEventDto, now = Date.now()): ApplyResult {
  const event = (dto.event ?? {}) as Record<string, unknown>;
  if (dto.seq <= s.seq) return {};
  if (dto.seq > s.seq + 1 && s.seq > 0 && !benignGap(s, event)) return { gap: true };
  s.seq = dto.seq;
  const hostKind = str(event.kind);
  if (hostKind && !event.type) {
    switch (hostKind) {
      case 'crashed':
        setBanner(s, 'crashed', event, now);
        endRun(s);
        return {};
      case 'taken_over':
        setBanner(s, 'taken_over', event, now);
        endRun(s);
        return { takenOver: true };
      case 'reloaded':
        setBanner(s, 'reloaded', event, now);
        return {};
      case 'woken':
        return applyWoken(s, event);
      case 'wake_failed':
        delete s.waking;
        setBanner(s, 'wake_failed', event, now);
        return {};
      default:
        return {};
    }
  }
  switch (event.type) {
    case 'agent_start':
      s.run = { phase: 'running', running: true, startedAt: s.run.startedAt ?? now };
      return {};
    case 'agent_end':
      endRun(s, event.willRetry === true ? 'retrying' : 'idle');
      return { reconcile: true };
    case 'agent_settled':
      if (s.run.running) endRun(s);
      return { reconcile: true };
    case 'message_start': {
      const message = event.message as AgentMessage | undefined;
      if (!isObject(message)) return {};
      if (message.role === 'assistant') {
        delete s.streaming;
        takeMessage(s, message, now);
      } else if (message.role === 'user') {
        settleByMessage(s, message);
      }
      return {};
    }
    case 'message_update': {
      const message = event.message as AgentMessage | undefined;
      const full = isObject(message) && Array.isArray(message.content);
      if (full) takeMessage(s, message!, now);
      else {
        if (!s.streaming) s.streaming = { message: { role: 'assistant', content: [] }, startedAt: now };
        if (isObject(event.assistantMessageEvent)) applyDelta(s.streaming.message, event.assistantMessageEvent);
      }
      return {};
    }
    case 'message_end': {
      const message = event.message as AgentMessage | undefined;
      if (!isObject(message) || typeof message.role !== 'string') return {};
      if (message.role === 'assistant') delete s.streaming;
      if (message.role === 'user') settleByMessage(s, message);
      if (message.role === 'toolResult' && typeof message.toolCallId === 'string') delete s.liveTools[message.toolCallId];
      appendEntry(s, entryForMessage(message, `${LIVE_PREFIX}${dto.seq}`, s.leafId, now));
      return {};
    }
    case 'tool_execution_start': {
      const id = str(event.toolCallId);
      if (!id) return {};
      const tool: LiveTool = { id, name: str(event.toolName) ?? '', args: isObject(event.args) ? event.args : {}, startedAt: now };
      const parent = str(event.parentToolCallId);
      if (parent) tool.parentId = parent;
      s.liveTools[id] = tool;
      return {};
    }
    case 'tool_execution_update': {
      const id = str(event.toolCallId);
      if (!id) return {};
      const tool = (s.liveTools[id] ??= { id, name: str(event.toolName) ?? '', args: isObject(event.args) ? event.args : {}, startedAt: now });
      const text = resultText(event.partialResult);
      if (text !== undefined) tool.partial = text;
      if (isObject(event.partialResult) && event.partialResult.details !== undefined) tool.details = event.partialResult.details;
      return {};
    }
    case 'tool_execution_end': {
      const id = str(event.toolCallId);
      if (!id) return {};
      // Its toolResult entry may already be there (pi orders end before the result message).
      const tool = (s.liveTools[id] ??= { id, name: str(event.toolName) ?? '', args: isObject(event.args) ? event.args : {}, startedAt: now });
      tool.done = true;
      tool.isError = event.isError === true;
      if (event.result !== undefined) tool.result = event.result;
      if (typeof event.durationMs === 'number') tool.durationMs = event.durationMs;
      return {};
    }
    case 'queue_update': {
      const steering = Array.isArray(event.steering) ? event.steering.filter((t): t is string => typeof t === 'string') : [];
      const followUp = Array.isArray(event.followUp) ? event.followUp.filter((t): t is string => typeof t === 'string') : [];
      s.queue = { steering, followUp };
      settleByQueue(s, [...steering, ...followUp]);
      return {};
    }
    case 'compaction_start':
      s.run = { ...s.run, phase: 'compacting' };
      return {};
    case 'compaction_end':
      s.run = { ...s.run, phase: s.run.running ? 'running' : 'idle' };
      return {};
    case 'auto_retry_start':
      s.run = {
        ...s.run,
        phase: 'retrying',
        retry: { attempt: Number(event.attempt ?? 0), maxAttempts: Number(event.maxAttempts ?? 0), delayMs: Number(event.delayMs ?? 0), error: str(event.errorMessage) ?? '' },
      };
      return {};
    case 'auto_retry_end': {
      const { retry: _retry, ...rest } = s.run;
      s.run = { ...rest, phase: rest.running ? 'running' : 'idle' };
      return {};
    }
    case 'entry_appended': {
      const entry = event.entry as SessionEntry | undefined;
      if (isObject(entry) && typeof entry.id === 'string') appendEntry(s, entry);
      return {};
    }
    case 'session_info_changed': {
      const name = str(event.name);
      if (name === undefined) delete s.name;
      else s.name = name;
      return { nameChanged: name ?? '' };
    }
    case 'thinking_level_changed':
      if (typeof event.level === 'string') s.thinkingLevel = event.level;
      return {};
    case 'model_changed':
      if (isObject(event.model)) s.model = event.model as SessionState['model'];
      return {};
    case 'run_state': {
      const phase = str(event.state) as RunPhase | undefined;
      if (!phase) return {};
      s.run = { ...s.run, phase };
      if (Array.isArray(event.waitingOn)) s.run.waitingOn = event.waitingOn.filter((w): w is string => typeof w === 'string');
      else delete s.run.waitingOn;
      if (Array.isArray(event.backgroundOn)) s.run.backgroundOn = event.backgroundOn.filter((w): w is string => typeof w === 'string');
      else delete s.run.backgroundOn;
      return {};
    }
    case 'commands_changed':
      return { commandsChanged: true };
    case 'lagged':
      return { resync: true };
    case 'session_tree':
      // [chat-tree] The host's notice that `_oino/navigate_tree` moved the leaf.
      return { branchMoved: str(event.leafId) ?? null };
    case 'session_replaced': {
      const id = str(event.sessionId);
      return id ? { replacedBy: id } : {};
    }
    case 'session_crashed':
      setBanner(s, 'crashed', event, now);
      endRun(s);
      return {};
    case 'topic': {
      const topic = str(event.topic);
      if (topic) s.topics[topic] = { data: event.data, sticky: event.sticky !== false, publisher: str(event.publisher) ?? '' };
      return {};
    }
    case 'extension_ui_request': {
      const id = str(event.id);
      if (id && DIALOG_METHODS.has(str(event.method) ?? '') && !s.dialogs.includes(id)) s.dialogs.push(id);
      return { ui: [event] };
    }
    case 'extension_ui_dismiss': {
      const id = str(event.id);
      if (id) s.dialogs = s.dialogs.filter((d) => d !== id);
      return { ui: [event] };
    }
    case 'oino_ui_patch':
      return { ui: [event] };
    default:
      return {};
  }
}

/**
 * perf-open `{kind:"woken"}`: the runtime behind a file-read snapshot is up — its real
 * state, commands, ui registry and topics replace the provisional ones. A leaf past ours
 * (the wake appended, e.g. a repair) asks the store to catch up.
 */
function applyWoken(s: SessionState, event: Record<string, unknown>): ApplyResult {
  delete s.waking;
  if (isObject(event.state)) applyInfo(s, event.state);
  if (Array.isArray(event.commands)) s.commands = event.commands.filter(isObject) as CommandInfo[];
  if (Array.isArray(event.topics)) s.topics = { ...s.topics, ...topicsOf(event.topics) };
  const out: ApplyResult = {};
  if (isObject(event.ui)) out.ui = [{ type: 'oino_ui_snapshot', ...event.ui }];
  if (s.name !== undefined) out.nameChanged = s.name;
  const leaf = str(event.leafId);
  const last = [...s.entries].reverse().find((e) => !isProvisional(e.id));
  if (leaf && last && leaf !== s.leafId && s.byId[leaf] === undefined) out.catchUp = last.id;
  return out;
}

/**
 * Prepends an older page (`get_entries{before}` reply) to the loaded history; entries
 * already present are skipped. The page's `cursor`/`hasMore` become the history's.
 */
export function prependEntries(
  s: SessionState,
  page: { entries?: readonly SessionEntry[]; total?: number; hasMore?: boolean; cursor?: string | null },
): void {
  const older = (page.entries ?? []).filter((e) => isObject(e) && typeof e.id === 'string' && s.byId[e.id] === undefined);
  const prev = s.history;
  s.history = {
    total: typeof page.total === 'number' ? page.total : (prev?.total ?? s.entries.length + older.length),
    hasMore: page.hasMore === true && older.length > 0,
    ...(page.cursor ? { cursor: page.cursor } : {}),
  };
  if (!older.length) return;
  s.entries = [...older, ...s.entries];
  s.byId = indexEntries(s.entries);
  s.entriesRev += 1;
}

/**
 * [chat-tree] The branch moved (a tree jump: "Return here", Edit, Retry, the tree view):
 * the loaded entries become the new branch's tail page (`get_entries{limit}`), its paging
 * state follows. Entries both branches share keep their object, so the turns index keeps
 * those turns' identity. Bumps `branchRev` (the tree view reloads on it).
 */
export function replaceBranch(
  s: SessionState,
  page: { entries?: readonly SessionEntry[]; leafId?: string | null; total?: number; hasMore?: boolean; cursor?: string | null },
): void {
  const prev = new Map<string, SessionEntry>();
  for (const e of s.entries) if (!isProvisional(e.id)) prev.set(e.id, e);
  const entries = (page.entries ?? [])
    .filter((e) => isObject(e) && typeof e.id === 'string')
    .map((e) => {
      const old = prev.get(e.id);
      return old && old.type === e.type && old.timestamp === e.timestamp && old.parentId === e.parentId ? old : e;
    });
  s.entries = entries;
  s.byId = indexEntries(entries);
  s.leafId = page.leafId === undefined ? (entries.at(-1)?.id ?? null) : page.leafId;
  s.entriesRev += 1;
  s.branchRev = (s.branchRev ?? 0) + 1;
  const hasMore = page.hasMore === true && entries.length > 0;
  s.history = { total: typeof page.total === 'number' ? page.total : entries.length, hasMore, ...(hasMore && page.cursor ? { cursor: page.cursor } : {}) };
}

/**
 * perf-app: drops loaded history older than the newest `keep` entries (at least `keep`
 * stay unless that turn alone is longer than `keep`), cut at the user message that starts
 * the turn the cut falls in (turn-aligned), so a chat read back to its start does not hold
 * every page forever. Only paged chats (`history` present) trim — the dropped
 * part loads again through `get_entries{before}` (the cut entry becomes the
 * cursor). Returns whether anything was dropped.
 */
export function trimHistory(s: SessionState, keep: number): boolean {
  const h = s.history;
  if (!h || h.loading || s.entries.length <= keep) return false;
  const isUser = (e: SessionEntry | undefined) => e?.type === 'message' && e.message?.role === 'user';
  // render-loop: the cut moves BACK to the user message that starts the turn it falls in,
  // so at least `keep` entries stay. It used to move forward, and a chat whose last user
  // message is near the end (an open-ended turn after one giant subagent turn) kept just a
  // few entries — a short page, which the timeline paged straight back in, past
  // LOADED_MAX, trimmed again… a flicker loop that starved the app.
  // Bounded: when that turn alone is longer than `keep`, the cut moves forward as before
  // (the timeline's paging guards keep that from looping).
  const target = s.entries.length - keep;
  let cut = target;
  while (cut > 0 && !isUser(s.entries[cut])) cut--;
  if (cut <= 0 || target - cut > keep) {
    cut = target;
    while (cut < s.entries.length && !isUser(s.entries[cut])) cut++;
  }
  // No user message to cut at (one giant turn): keep everything.
  if (cut <= 0 || cut >= s.entries.length) return false;
  const first = s.entries[cut]!;
  s.entries = s.entries.slice(cut);
  s.byId = indexEntries(s.entries);
  s.history = { total: h.total, hasMore: true, cursor: first.id };
  s.entriesRev += 1;
  return true;
}

/** The last host-confirmed entry before the first provisional one (`get_entries{since}`); undefined = from the start. */
export function reconcileSince(s: SessionState): { since?: string; needed: boolean } {
  const first = s.entries.findIndex((e) => isProvisional(e.id));
  if (first < 0) return { needed: false };
  const prev = first > 0 ? s.entries[first - 1]!.id : undefined;
  return prev ? { since: prev, needed: true } : { needed: true };
}

/**
 * Replaces the provisional tail with the host's entries (`get_entries{since}` reply):
 * everything after `since` is the host's truth.
 */
export function reconcileEntries(s: SessionState, since: string | undefined, fetched: readonly SessionEntry[], leafId: string | null | undefined): void {
  const keep = since === undefined ? 0 : (s.byId[since] ?? -1) + 1;
  if (since !== undefined && keep === 0) return; // `since` vanished (a reset raced us): the snapshot wins.
  const next = s.entries.slice(0, keep);
  for (const e of fetched) if (isObject(e) && typeof e.id === 'string') next.push(e);
  s.entries = next;
  s.byId = indexEntries(next);
  s.leafId = leafId ?? next.at(-1)?.id ?? null;
  s.entriesRev += 1;
}

/**
 * The current branch: every entry when there is no fork in the list, else the
 * chain from `leafId` to the root (snapshots may carry every branch).
 */
export function branchOf(entries: readonly SessionEntry[], leafId: string | null): SessionEntry[] {
  if (!leafId || entries.length < 2) return entries as SessionEntry[];
  const seen = new Set<string>();
  let forked = false;
  for (const e of entries) {
    const p = e.parentId;
    if (!p) continue;
    if (seen.has(p)) {
      forked = true;
      break;
    }
    seen.add(p);
  }
  if (!forked) return entries as SessionEntry[];
  const byId = new Map(entries.map((e) => [e.id, e]));
  const chain: SessionEntry[] = [];
  let cur = byId.get(leafId);
  const guard = new Set<string>();
  while (cur && !guard.has(cur.id)) {
    guard.add(cur.id);
    chain.push(cur);
    cur = cur.parentId ? byId.get(cur.parentId) : undefined;
  }
  return chain.reverse();
}
