/**
 * One live chat (UNI-113): a connection to one pi (host + pid) and the
 * state its bridge messages build (docs/m5/PROTOCOL.md §3).
 */
import { batch, createSignal } from 'solid-js';
import { graphErrorText, isProgress, isRun, type GraphError, type LhGraphOutput, type LhProgress, type LhRun, type TranscriptItem, type WorkTranscript } from './progress';
import { isLhModeState, type LhModeChoice, type LhModeState } from './lhMode';
import { isDreamStatus, type DreamAction, type DreamDetail, type DreamStatus } from './dream';
import { createStore, produce, reconcile } from 'solid-js/store';
import { chatTransport, type ChatConnection } from './transport';
import type { Entry, ImageRef, LiveTools, Part, Streaming } from './model';
import { friendlyBridgeError } from './bridgeError';

/** UNI-251: where a live `custom` entry belongs. Bridges before UNI-251 send
 * custom entries (a `/command` echo) only at the turn's end, AFTER the user
 * message that followed them in the session; place such a late arrival back
 * in timestamp order (scanning a short tail) so the echo sits right before
 * its harness message and folds into the user's bubble. Anything else, or
 * entries without timestamps, append. */
export function customInsertIndex(entries: readonly Entry[], entry: Entry): number {
  const n = entries.length;
  if (entry.type !== 'custom') return n;
  const at = Date.parse(String((entry as { timestamp?: unknown }).timestamp ?? ''));
  if (!Number.isFinite(at)) return n;
  let k = n;
  for (let steps = 0; k > 0 && steps < 40; steps++) {
    const prev = Date.parse(String((entries[k - 1] as { timestamp?: unknown }).timestamp ?? ''));
    if (!Number.isFinite(prev) || prev <= at) break;
    k--;
  }
  return k;
}

export type ModelInfo = { provider: string; id: string; name?: string; reasoning: boolean };
export type RunState = {
  running: boolean;
  model?: ModelInfo;
  thinking?: string;
  thinkingLevels?: string[];
  context?: { tokens: number | null; window: number; percent: number | null };
  cost?: number;
  /** UNI-162: a short label (e.g. "2 bg tasks will resume agent · subagent running") pushed while `running`
   *  is false but background work is still pending — the pi-side footer's "waiting on …" line. */
  waiting?: string;
};
/** One `@` file suggestion from pi: `value` replaces the typed `@query`. */
export type FileItem = { value: string; label: string; path: string; dir: boolean };
/** One `paths_stat` answer (UNI-204): `path` as asked, `resolved` absolute on the PC. */
export type PathStatItem = { path: string; resolved: string; kind: 'file' | 'dir' | 'missing' };
/** One row of a `sessions` reply: a session the phone can resume. */
export type SessionsItem = { path: string; id: string; name?: string; cwd: string; firstMessage: string; modified: number; messageCount: number; current: boolean };
/** One row of a `tree` reply: a session entry, preview only. */
export type TreeNode = { id: string; parentId: string | null; kind: string; preview: string; timestamp?: string; label?: string; onPath: boolean; current: boolean };
/** UNI-290: a `tree_go` to a user message moved pi's leaf to just before it — the message's text
 *  (and images, as the usual inline/`mediaRef` image refs) for the composer, like /tree in the TUI.
 *  Older bridges ack without it. */
export type TreeGoRestore = { text: string; images: ImageRef[] };
/** Pi is busy: the phone can show a "Stop the current run and …?" confirm, then retry with `force: true`. */
export class BusyError extends Error {}

/** UNI-275: one session-search result (bridge `search`). */
export type SearchHit = { id: string; role: string; snippet: string; marks: Array<[number, number]>; score: number; timestamp?: string };
export type ArgCompletion = { value: string; label: string; description?: string };
export type CommandInfo = { name: string; description?: string; source: 'extension' | 'prompt' | 'skill' | 'builtin' };
/** One queue row (UNI-159). `source:"tui"` items are pi's own queue (read-only); `source:"phone"` + `mode:"after"` items
 *  are the bridge's own queue (editable/removable/reorderable/promotable). */
export type Queued = { id: string; text: string; mode: 'steer' | 'followUp' | 'after'; source: 'phone' | 'tui'; editable: boolean };
/** A /unipi:btw page this phone asked: question, streamed/streamed-done answer.
 *  `id` is the phone's own `btw{ref}`; `bridgeId` is btw's page id (from
 *  `btw_delta`/`btw_end`/`btw_list`), used to reconcile after a reconnect. */
export type BtwPage = {
  id: string;
  bridgeId?: string;
  question: string;
  answer: string;
  error?: string;
  done: boolean;
  /** Model reasoning streamed so far (shown while running). */
  thinking?: string;
  /** One line per tool call ("… Reading x" → "✓ Read x"). */
  toolLines?: string[];
  usage?: { input: number; output: number; totalTokens: number };
};
/** One `btw_list` page. Older bridges send only question/answer/error. */
export type BtwListPage = {
  id?: string;
  ref?: string;
  question: string;
  answer: string;
  error?: string;
  done?: boolean;
  running?: boolean;
  toolLines?: string[];
  thinking?: string;
};

/** A listed page that is still being answered. Older bridges have no flag:
 *  an empty answer and no error there means "not finished yet". */
export const btwListRunning = (p: BtwListPage): boolean => (p.running ?? p.done === false) || (p.running === undefined && p.done === undefined && !p.answer && !p.error);

/** The phone's own page a `btw_delta`/`btw_end` belongs to: by `ref`, else by btw's page id. */
function findBtwPage(state: ChatState, msg: Msg): BtwPage | undefined {
  const ref = msg.ref == null ? '' : String(msg.ref);
  const id = msg.id == null ? '' : String(msg.id);
  return (ref ? state.btwPages.find((p) => p.id === ref) : undefined) ?? (id ? state.btwPages.find((p) => p.bridgeId === id) : undefined);
}

/**
 * Folds a `btw_list` reply into this chat's own pages and returns the rest
 * (pages asked elsewhere: the TUI, another phone, an earlier app run).
 * A listed page that IS one of ours (same ref/id; for older bridges, same
 * question) updates it in place: finished → answer/error, running → the
 * progress so far. So a page left on "Thinking…" by a dropped connection
 * turns into its answer and never shows twice (UNI-219).
 * `complete` (newer bridges): the list has every running page, so an own
 * page still waiting that isn't in it was lost (pi restarted / session
 * changed) and ends with an error instead of spinning forever.
 */
export function reconcileBtwPages(own: BtwPage[], listed: BtwListPage[], complete: boolean): BtwListPage[] {
  const rest: BtwListPage[] = [];
  const claimed = new Set<BtwPage>();
  // Newest first, so an id-less (older bridge) match claims the latest page with that question.
  for (const p of [...listed].reverse()) {
    let mine = (p.ref ? own.find((o) => o.id === p.ref) : undefined) ?? (p.id ? own.find((o) => o.bridgeId === p.id) : undefined);
    // Older bridges: no ids — match by question (newest first, each once).
    if (!mine && !p.id && !p.ref) {
      const running = btwListRunning(p);
      mine = [...own].reverse().find((o) => !claimed.has(o) && o.question === p.question && (running ? !o.done : o.done ? o.answer === p.answer : true));
    }
    if (!mine) {
      rest.unshift(p);
      continue;
    }
    claimed.add(mine);
    if (p.id && !p.id.startsWith('pending-')) mine.bridgeId = p.id;
    if (mine.done) continue;
    if (btwListRunning(p)) {
      if ((p.answer ?? '').length >= mine.answer.length) mine.answer = p.answer ?? '';
      if (p.thinking && p.thinking.length >= (mine.thinking ?? '').length) mine.thinking = p.thinking;
      if (p.toolLines?.length) mine.toolLines = [...p.toolLines];
    } else {
      mine.answer = p.answer ?? '';
      mine.error = p.error || undefined;
      if (p.toolLines?.length) mine.toolLines = [...p.toolLines];
      mine.done = true;
    }
  }
  if (complete) {
    for (const o of own) {
      if (!o.done && !claimed.has(o)) {
        o.done = true;
        o.error = o.error ?? 'The answer was lost (pi restarted or the session changed). Ask again.';
      }
    }
  }
  return rest;
}
export type Dialog = {
  id: number;
  kind: 'select' | 'confirm' | 'input' | 'editor' | 'ask_user' | 'custom';
  title?: string;
  options?: string[];
  message?: string;
  placeholder?: string;
  prefill?: string;
  questions?: AskQuestion[];
  deadline?: number;
};
export type AskOption = { label: string; description?: string; value?: string; action?: string; prefill?: string };
export type AskQuestion = { question: string; header?: string; options: AskOption[]; multi_select?: boolean; other?: boolean };
export type SessionInfo = { file?: string; id: string; name?: string; cwd: string };

/** One model entry of a Fusion lead/sidekick list. */
export type FusionModelInfo = { key: string; name: string };
/** Fusion's preset + active selection (UNI-160 §1), absent when Fusion isn't installed. */
export type FusionPresetInfo = {
  leads: FusionModelInfo[];
  sidekicks: FusionModelInfo[];
  /** The terminal picker's list (recent + curated leads/sidekicks). */
  curated?: FusionModelInfo[];
  default: { lead?: string; sidekick?: string };
  effort: Record<string, string>;
  active:
    | { kind: 'single'; model: string }
    | { kind: 'fusion'; lead: string; sidekick: string; leadEffort?: string; sidekickEffort?: string }
    | undefined;
};
/** Fusion's live lead/sidekick status — undefined when Fusion is off. */
export type FusionStatusInfo = {
  leadName: string;
  leadKey?: string;
  sidekickKey?: string;
  leadEffort: string;
  sidekickName: string;
  sidekickEffort: string;
  savedUsd?: number;
  busy?: boolean;
  leadToolCalls?: number;
  sidekickToolCalls?: number;
};
/** One row of the Running section (UNI-160 §4). */
export type WorkItem = {
  id: string;
  kind: 'bg' | 'subagent' | 'sidekick';
  title: string;
  dot: 'running' | 'stopped' | 'failed' | 'done';
  detail?: string;
  startedAt: number;
  endedAt?: number;
  canStop: boolean;
  canRerun: boolean;
  canBackground: boolean;
};
/** Context/cost/tps snapshot (UNI-160 §3). */
export type StatsInfo = {
  context?: { tokens: number | null; window: number; percent: number | null };
  tokensIn: number;
  tokensOut: number;
  cacheHit: number;
  cost: number;
  tps: number;
};
/** One /unipi:info group, phone-shaped (UNI-160 §5). */
export type InfoGroupInfo = { id: string; label: string; stats: Array<{ label: string; value: string }> };

export type ConnState = 'connecting' | 'live' | 'closed' | 'error';

/**
 * A phone message on its way (UNI-143). `acked`: the bridge handed it to pi.
 * `accepted`: pi took it (its `input` event named our ref) — the bubble stays,
 * without "Sending…", until the user entry shows up.
 */
export type Pending = { ref: string; text: string; mode: 'prompt' | 'steer' | 'followUp'; at: number; acked?: boolean; accepted?: boolean };

/** A handed-over message whose entry never came (template, extension input handler…) goes after this. */
export const PENDING_ACKED_MS = 20_000;
/** No ack at all (link dropped mid-send): reported as not delivered. */
export const PENDING_LOST_MS = 30_000;

/** A message typed while the chat connection isn't live (UNI-159): kept in
 *  localStorage (per host+pid) and sent in order once `hello` arrives. Shown
 *  as a bubble marked "Waiting for connection", never "Sending…". */
export type Outboxed = { id: string; text: string; images?: Array<{ mime: string; data: string } | { mime: string; path: string }>; at: number };


const userText = (content: unknown): string =>
  typeof content === 'string'
    ? content
    : Array.isArray(content)
      ? content.filter((p: { type: string }) => p.type === 'text').map((p: { text: string }) => p.text).join('')
      : '';

const isExtensionCommand = (state: ChatState, text: string) => {
  const name = /^\/(\S+)/.exec(text)?.[1];
  return !!name && state.commands.some((c) => c.name === name && c.source === 'extension');
};

/** Drops pending bubbles that are settled by time (see the constants). Returns the ones lost before pi got them. */
export function sweepPending(state: ChatState, now: number): Pending[] {
  const lost: Pending[] = [];
  for (let i = state.pending.length - 1; i >= 0; i--) {
    const p = state.pending[i]!;
    const age = now - p.at;
    if (p.acked ? age > PENDING_ACKED_MS : age > PENDING_LOST_MS) {
      if (!p.acked) lost.unshift(p);
      state.pending.splice(i, 1);
    }
  }
  return lost;
}

export type ChatState = {
  conn: ConnState;
  connMessage?: string;
  session?: SessionInfo;
  run: RunState;
  entries: Entry[];
  truncated: boolean;
  streaming?: Streaming;
  tools: LiveTools;
  commands: CommandInfo[];
  models: ModelInfo[];
  dialogs: Dialog[];
  queue: Queued[];
  /** Sent from the phone, not yet echoed by pi (optimistic bubbles). */
  pending: Pending[];
  /** Last error from pi for a phone request. */
  error?: { message: string; at: number };
  /** Notices from pi (ctx.ui.notify). */
  notice?: { text: string; level: string; at: number };
  /** UNI-202/211: an abort just pulled pi's own queued steer/followUp
   *  messages back (`restored`) — ChatView folds `texts` into the composer
   *  draft once, then this clears itself (a fresh object each time so a
   *  second abort with the exact same texts still re-fires the effect). */
  restored?: { texts: string[]; at: number };
  /** Set when a run starts (for the running clock). */
  runStartedAt?: number;
  /** UNI-221: when `run.waiting` (wake-capable pending work) was first seen — the idle "Working…" clock. */
  waitingSince?: number;
  /** Older history can be paged in (scroll up). */
  hasOlder: boolean;
  /** A history page is in flight. */
  loadingOlder: boolean;
  /** Entry ids where a paged-in older chunk meets the newer list (row groups never span one). */
  seams: string[];
  /** Side-question (btw) pages for this chat's sheet, newest last — mirrors the bridge's `btw_list`. */
  btwPages: BtwPage[];
  /** Messages waiting for a live connection (UNI-159), persisted in localStorage per host+pid. */
  outbox: Outboxed[];
  /** Fusion's preset (UNI-160 §1) — undefined when Fusion isn't installed on this pi. */
  fusion?: FusionPresetInfo;
  /** Fusion's live status — undefined when Fusion is off (or not installed). */
  fusionStatus?: FusionStatusInfo;
  /** The Running section's items (UNI-160 §4). */
  work: WorkItem[];
  /** Context/cost/tps snapshot (UNI-160 §3), while `watch{stats:true}` is open. */
  stats?: StatsInfo;
  /** /unipi:info groups (UNI-160 §5), while `watch{info:true}` is open. */
  infoGroups: InfoGroupInfo[];
  /** UNI-222: the long-horizon progress snapshot (hello.lhProgress / `lh_progress` pushes); undefined = none published. */
  lhProgress?: LhProgress;
  /** UNI-222: `false` once this pi's bridge answered `lh_progress_get` with "unknown type" (an older unipi). */
  lhProgressSupported?: boolean;
  /** UNI-269: the working mode (hello.lhMode / `lh_mode` pushes); undefined = an older bridge or no long-horizon. */
  lhMode?: LhModeState;
  /** UNI-118: Dream status (hello.dream / `dream_status` pushes); undefined = no dream module on this pi. */
  dream?: DreamStatus;
  /** UNI-118: `false` once this pi's bridge answered `dream_get` with "unknown type" (an older unipi). */
  dreamSupported?: boolean;
};

const initial = (): ChatState => ({
  conn: 'connecting',
  run: { running: false },
  entries: [],
  truncated: false,
  tools: {},
  commands: [],
  models: [],
  dialogs: [],
  queue: [],
  pending: [],
  hasOlder: false,
  loadingOlder: false,
  seams: [],
  btwPages: [],
  outbox: [],
  work: [],
  infoGroups: [],
});

type Msg = { t: string; [key: string]: unknown };

/** Applies one bridge message to the state (pure over the store setter). */
export function applyMessage(state: ChatState, msg: Msg): void {
  switch (msg.t) {
    case 'hello': {
      state.session = msg.session as SessionInfo;
      state.run = msg.state as RunState;
      state.entries = (msg.entries as Entry[]) ?? [];
      state.truncated = !!msg.truncated;
      state.hasOlder = !!msg.truncated;
      state.loadingOlder = false;
      state.seams = [];
      const s = msg.streaming as { id: string; content: Part[] } | undefined;
      state.streaming = s ? { id: s.id, parts: s.content ?? [] } : undefined;
      state.tools = {};
      for (const t of (msg.tools as Array<{ callId: string; name: string; args: unknown; text?: string }>) ?? []) {
        state.tools[t.callId] = { name: t.name, args: t.args, text: t.text };
      }
      state.commands = (msg.commands as CommandInfo[]) ?? [];
      state.models = (msg.models as ModelInfo[]) ?? [];
      state.dialogs = (msg.dialogs as Dialog[]) ?? [];
      state.queue = (msg.queue as Queued[]) ?? [];
      state.fusion = msg.fusion as FusionPresetInfo | undefined;
      state.work = (msg.work as WorkItem[]) ?? [];
      state.lhProgress = isProgress(msg.lhProgress) ? msg.lhProgress : undefined;
      state.lhMode = isLhModeState(msg.lhMode) ? msg.lhMode : undefined;
      state.dream = isDreamStatus(msg.dream) ? msg.dream : undefined;
      if (state.dream) state.dreamSupported = true;
      // pi already has every message it acked or accepted (entries or queue).
      {
        const seen = new Set(state.entries.flatMap((e) => (e.type === 'message' && (e as { message: { role: string } }).message.role === 'user' ? [userText((e as { message: { content: unknown } }).message.content).trim()] : [])));
        state.pending = state.pending.filter((p) => !p.acked && !p.accepted && !seen.has(p.text.trim()));
      }
      state.runStartedAt = state.run.running ? (state.runStartedAt ?? Date.now()) : undefined;
      state.waitingSince = state.run.waiting ? (state.waitingSince ?? Date.now()) : undefined;
      return;
    }
    // The model list changed after hello: a provider registered late
    // (pi-omniroute-bridge), so a phone that connected early only had part of it.
    case 'models': {
      if (Array.isArray(msg.models)) state.models = msg.models as ModelInfo[];
      return;
    }
    case 'state': {
      const { t: _t, fusion, ...rest } = msg as typeof msg & { fusion?: unknown };
      // A state push may carry a fresh fusion preset (after set_fusion,
      // UNI-176); lift it onto state.fusion instead of state.run.
      if (fusion !== undefined) state.fusion = fusion as typeof state.fusion;
      const wasRunning = state.run.running;
      const wasWaiting = state.run.waiting;
      Object.assign(state.run, rest);
      // UNI-221: the bridge omits `waiting` once pending work clears (and on
      // agent_start) — a run-state push without it means "not waiting".
      if ('running' in rest && !('waiting' in rest)) state.run.waiting = undefined;
      if (state.run.waiting && !wasWaiting) state.waitingSince = Date.now();
      if (!state.run.waiting) state.waitingSince = undefined;
      if (state.run.running && !wasRunning) state.runStartedAt = Date.now();
      if (!state.run.running) {
        state.runStartedAt = undefined;
        state.streaming = undefined;
        state.tools = {};
      }
      return;
    }
    case 'session': {
      const { t: _t, reason: _r, ...info } = msg;
      state.session = { ...(state.session ?? { id: '', cwd: '' }), ...(info as Partial<SessionInfo>) } as SessionInfo;
      return;
    }
    case 'entry': {
      const entry = msg.entry as Entry;
      if (!entry || typeof entry !== 'object') return;
      const i = state.entries.findIndex((e) => e.id === entry.id);
      if (i >= 0) state.entries[i] = entry;
      else {
        const at = customInsertIndex(state.entries, entry);
        if (at < state.entries.length) state.entries.splice(at, 0, entry);
        else state.entries.push(entry);
      }
      // A delivered user message replaces its optimistic bubble.
      if (entry.type === 'message') {
        const m = (entry as { message: { role: string; content: unknown } }).message;
        if (m.role === 'user') {
          const text = userText(m.content).trim();
          let p = state.pending.findIndex((x) => x.text.trim() === text);
          // pi may have rewritten it (prompt template, /skill:, an input
          // handler): the oldest message pi took is the one delivered.
          if (p < 0) p = state.pending.findIndex((x) => x.accepted || (x.acked && x.mode === 'prompt'));
          if (p >= 0) state.pending.splice(p, 1);
        }
      }
      return;
    }
    case 'history': {
      state.loadingOlder = false;
      const page = (msg.entries as Entry[]) ?? [];
      // Only a page that ends where our list starts (a fresh hello may have replaced it).
      if (state.entries[0]?.id !== msg.before) return;
      const known = new Set(state.entries.map((e) => e.id));
      const older = page.filter((e) => !known.has(e.id));
      if (older.length) state.seams.push(String(msg.before));
      state.entries.unshift(...older);
      state.hasOlder = !!msg.more && page.length > 0;
      return;
    }
    case 'msg_start':
      state.streaming = { id: String(msg.id), parts: [] };
      return;
    case 'delta': {
      const s = state.streaming;
      if (!s || s.id !== msg.id) return;
      const index = Number(msg.index) || 0;
      const kind = msg.kind as 'text' | 'thinking' | 'toolcall';
      const text = String(msg.text ?? '');
      while (s.parts.length <= index) s.parts.push({ type: 'text', text: '' });
      const part = s.parts[index]!;
      if (kind === 'text') {
        if (part.type !== 'text') s.parts[index] = { type: 'text', text };
        else (part as { text: string }).text += text;
      } else if (kind === 'thinking') {
        if (part.type !== 'thinking') s.parts[index] = { type: 'thinking', thinking: text };
        else (part as { thinking: string }).thinking += text;
      } else {
        if (part.type !== 'toolCall') s.parts[index] = { type: 'toolCall', id: `pending-${index}`, name: '', arguments: text };
        else (part as { arguments: unknown }).arguments = String((part as { arguments: unknown }).arguments ?? '') + text;
      }
      return;
    }
    case 'msg_end':
      if (state.streaming?.id === msg.id) state.streaming = undefined;
      return;
    case 'tool_start':
      state.tools[String(msg.callId)] = { name: String(msg.name), args: msg.args };
      return;
    case 'tool_update': {
      const t = state.tools[String(msg.callId)];
      if (t) t.text = String(msg.text ?? '');
      return;
    }
    case 'tool_end': {
      const t = state.tools[String(msg.callId)];
      if (t) {
        t.done = true;
        t.isError = !!msg.isError;
      }
      return;
    }
    case 'input': {
      // Something was submitted (TUI or phone). A phone prompt pi took names
      // its ref: the bubble stays (no "Sending…") until the entry lands;
      // a steer / follow-up now lives in pi's queue (the queue strip).
      const i = msg.ref ? state.pending.findIndex((p) => p.ref === msg.ref) : -1;
      if (i >= 0) {
        if (state.pending[i]!.mode === 'prompt' && msg.mode === 'prompt') state.pending[i]!.accepted = true;
        else state.pending.splice(i, 1);
      }
      return;
    }
    case 'ack': {
      const i = state.pending.findIndex((p) => p.ref === msg.ref);
      if (i < 0) return;
      // An extension command ran (no chat message follows). Older bridges
      // don't say so: decide from the command list.
      if (msg.as === 'command' || isExtensionCommand(state, state.pending[i]!.text)) state.pending.splice(i, 1);
      else state.pending[i]!.acked = true;
      return;
    }
    case 'queue':
      state.queue = (msg.items as Queued[]) ?? [];
      return;
    case 'restored':
      state.restored = { texts: (msg.texts as string[]) ?? [], at: Date.now() };
      return;
    case 'btw_delta': {
      // Matched by `ref` (the phone's own request id, set by askBtw before
      // sending), else by btw's page id (learned from an earlier delta or a
      // `btw_list` after a reconnect). A run this phone doesn't know (a
      // `btw_list{watch}` stream of another phone's question) is ignored —
      // the sheet lists it from `btw_list` instead.
      const page = findBtwPage(state, msg);
      if (!page || page.done) return;
      const id = msg.id == null ? '' : String(msg.id);
      if (id && !id.startsWith('pending-')) page.bridgeId = id;
      const text = String(msg.text ?? '');
      if (msg.kind === 'text') page.answer += text;
      else if (msg.kind === 'thinking') page.thinking = (page.thinking ?? '') + text;
      else if (msg.kind === 'tool') {
        const lines = page.toolLines ?? (page.toolLines = []);
        const index = typeof msg.index === 'number' ? msg.index : -1;
        if (index >= 0 && index <= lines.length) lines[index] = text;
        // Older bridges: no index — a finished line replaces the last running one.
        else if (index < 0 && !text.startsWith('…') && lines.length && lines[lines.length - 1]!.startsWith('…')) lines[lines.length - 1] = text;
        else lines.push(text);
      }
      return;
    }
    case 'btw_end': {
      const page = findBtwPage(state, msg);
      if (!page) return;
      const id = msg.id == null ? '' : String(msg.id);
      if (id && !id.startsWith('pending-')) page.bridgeId = id;
      page.answer = String(msg.answer || page.answer || '');
      page.error = msg.error ? String(msg.error) : undefined;
      page.usage = msg.usage as BtwPage['usage'];
      page.done = true;
      return;
    }
    case 'dialog': {
      const { t: _t, ...d } = msg;
      const dialog = d as unknown as Dialog;
      if (!state.dialogs.some((x) => x.id === dialog.id)) state.dialogs.push(dialog);
      return;
    }
    case 'dialog_end': {
      const i = state.dialogs.findIndex((d) => d.id === msg.id);
      if (i >= 0) state.dialogs.splice(i, 1);
      return;
    }
    case 'notify':
      state.notice = { text: String(msg.text ?? ''), level: String(msg.level ?? 'info'), at: Date.now() };
      return;
    case 'fusion':
      state.fusionStatus = msg.status as FusionStatusInfo | undefined;
      return;
    case 'work':
      state.work = (msg.items as WorkItem[]) ?? [];
      return;
    case 'stats':
      state.stats = msg.stats as StatsInfo;
      return;
    case 'info':
      state.infoGroups = (msg.groups as InfoGroupInfo[]) ?? [];
      return;
    case 'lh_progress':
      state.lhProgress = isProgress(msg.progress) ? msg.progress : undefined;
      state.lhProgressSupported = true;
      return;
    case 'lh_mode':
      state.lhMode = isLhModeState(msg.mode) ? msg.mode : undefined;
      return;
    case 'dream_status':
      state.dream = isDreamStatus(msg.status) ? msg.status : undefined;
      state.dreamSupported = true;
      return;
    case 'error': {
      state.error = { message: friendlyBridgeError(msg.message as string | undefined), at: Date.now() };
      const ref = msg.ref as string | undefined;
      if (ref) {
        const i = state.pending.findIndex((p) => p.ref === ref);
        if (i >= 0) state.pending.splice(i, 1);
        // `btw{}` refused (btw not installed, bad request): end that page.
        const page = state.btwPages.find((p) => p.id === ref && !p.done);
        if (page) {
          page.error = friendlyBridgeError(msg.message as string | undefined);
          page.done = true;
        }
      }
      return;
    }
    default:
      return;
  }
}

let refSeq = 0;
const nextRef = () => `m${Date.now().toString(36)}${(++refSeq).toString(36)}`;

/** localStorage key for one chat's outbox (UNI-159): per host+pid, survives app restarts. */
const outboxKey = (hostId: string, pid: number) => `unipi.chat.outbox.${hostId}.${pid}`;

function loadOutbox(hostId: string, pid: number): Outboxed[] {
  try {
    const raw = localStorage.getItem(outboxKey(hostId, pid));
    const parsed = raw ? (JSON.parse(raw) as unknown) : [];
    return Array.isArray(parsed) ? (parsed as Outboxed[]) : [];
  } catch {
    return [];
  }
}

function saveOutbox(hostId: string, pid: number, items: Outboxed[]): void {
  try {
    localStorage.setItem(outboxKey(hostId, pid), JSON.stringify(items));
  } catch {
    // storage full/unavailable: the outbox just won't survive a restart.
  }
}

export type Chat = ReturnType<typeof createChat>;

/** A live chat with one pi. Call `close()` when the view goes away. */
export function createChat(hostId: string, pid: number) {
  const [state, setState] = createStore<ChatState>({ ...initial(), outbox: loadOutbox(hostId, pid) });
  const [generation, setGeneration] = createSignal(0);
  let conn: ChatConnection | undefined;
  let closed = false;
  let retry: ReturnType<typeof setTimeout> | undefined;
  let attempts = 0;
  /** A `reconnect` arrived: the stream's close is a session swap, not a failure. */
  let swapping = false;
  /** In-flight `@` lookups by ref (replies are not chat state). */
  const fileWaiters = new Map<string, (items: FileItem[]) => void>();
  const argWaiters = new Map<string, (items: ArgCompletion[]) => void>();
  const searchWaiters = new Map<string, (hits: SearchHit[] | null) => void>();
  /** UNI-275: navigateTo's promise, resolved when the paged-in history reaches the entry. */
  let navWaiter: { id: string; done: (ok: boolean) => void } | undefined;
  /** In-flight `paths_stat{}` lookups by ref (UNI-204); an older pi answers `error` → null. */
  const statWaiters = new Map<string, (items: PathStatItem[] | null) => void>();
  /** In-flight `sessions{}` / `tree{}` lookups by ref. */
  const sessionsWaiters = new Map<string, (items: SessionsItem[], more: boolean) => void>();
  const treeWaiters = new Map<string, (nodes: TreeNode[] | Error) => void>();
  /** In-flight `media{mediaRef}` fetches, keyed by mediaRef (not `ref`: chunks don't carry one). Accumulates base64 chunks until `done`. */
  const mediaWaiters = new Map<string, { mime: string; chunks: string[]; resolve: (r: { mime: string; dataUrl: string }) => void; reject: (e: Error) => void }>();
  /** In-flight `ack`/`error` waiters for session-navigation requests (session_new, tree_go, …). */
  const opWaiters = new Map<string, { resolve: (ack?: Msg) => void; reject: (e: Error) => void; raw?: boolean }>();
  /** In-flight `btw_list{}` lookups by ref. */
  const btwListWaiters = new Map<string, (pages: BtwListPage[], complete: boolean) => void>();
  /** In-flight `work_log{id}` lookups by ref. */
  const workLogWaiters = new Map<string, (page: { text: string; more: boolean } | { error: string }) => void>();
  /** In-flight `lh_graph{op:"output"|"history"}` / `work_transcript` lookups by ref (UNI-282): the reply type they wait for, and the raw reply (or error). */
  const replyWaiters = new Map<string, { t: string; done: (msg: Msg) => void }>();
  /** In-flight `lh_progress_get` refs (UNI-222). */
  const lhProgressRefs = new Set<string>();
  /** In-flight `dream_get` refs and `dream_detail` waiters (UNI-118). */
  const dreamRefs = new Set<string>();
  const dreamDetailWaiters = new Map<string, (d: DreamDetail | { error: string }) => void>();

  const connect = () => {
    if (closed) return;
    setState('conn', 'connecting');
    conn?.close();
    conn = chatTransport().open(
      hostId,
      pid,
      (line) => {
        let msg: Msg;
        try {
          msg = JSON.parse(line) as Msg;
        } catch {
          return;
        }
        // UNI-275: session search (older pi: "unknown type" → null = unsupported).
        if ((msg.t === 'search' || msg.t === 'error') && searchWaiters.has(String(msg.ref))) {
          const done = searchWaiters.get(String(msg.ref))!;
          searchWaiters.delete(String(msg.ref));
          done(msg.t === 'search' && Array.isArray(msg.hits) ? (msg.hits as SearchHit[]) : null);
          return;
        }
        // UNI-276: a command's argument suggestions (older pi: "unknown type" → none).
        if ((msg.t === 'arg_completions' || msg.t === 'error') && argWaiters.has(String(msg.ref))) {
          const done = argWaiters.get(String(msg.ref))!;
          argWaiters.delete(String(msg.ref));
          done(msg.t === 'arg_completions' && Array.isArray(msg.items) ? (msg.items as ArgCompletion[]) : []);
          return;
        }
        // A pi before `files` support answers "unknown type": no toast, empty list.
        if (msg.t === 'error' && fileWaiters.has(String(msg.ref))) {
          fileWaiters.get(String(msg.ref))!([]);
          fileWaiters.delete(String(msg.ref));
          return;
        }
        if (msg.t === 'files') {
          const done = fileWaiters.get(String(msg.ref));
          if (done) {
            fileWaiters.delete(String(msg.ref));
            done(Array.isArray(msg.items) ? (msg.items as FileItem[]) : []);
          }
          return;
        }
        if (msg.t === 'error' && sessionsWaiters.has(String(msg.ref))) {
          sessionsWaiters.get(String(msg.ref))!([], false);
          sessionsWaiters.delete(String(msg.ref));
          return;
        }
        if ((msg.t === 'paths_stat' || msg.t === 'error') && statWaiters.has(String(msg.ref))) {
          const done = statWaiters.get(String(msg.ref))!;
          statWaiters.delete(String(msg.ref));
          done(msg.t === 'paths_stat' && Array.isArray(msg.items) ? (msg.items as PathStatItem[]) : null);
          return;
        }
        if (msg.t === 'sessions') {
          const done = sessionsWaiters.get(String(msg.ref));
          if (done) {
            sessionsWaiters.delete(String(msg.ref));
            done(Array.isArray(msg.items) ? (msg.items as SessionsItem[]) : [], !!msg.more);
          }
          return;
        }
        if (msg.t === 'error' && treeWaiters.has(String(msg.ref))) {
          treeWaiters.get(String(msg.ref))!(new Error(String((msg as { message?: string }).message ?? 'The session tree failed')));
          treeWaiters.delete(String(msg.ref));
          return;
        }
        if (msg.t === 'tree') {
          const done = treeWaiters.get(String(msg.ref));
          if (done) {
            treeWaiters.delete(String(msg.ref));
            done(Array.isArray(msg.nodes) ? (msg.nodes as TreeNode[]) : []);
          }
          return;
        }
        if (msg.t === 'media_chunk') {
          const mediaRef = String((msg as { mediaRef?: unknown }).mediaRef ?? '');
          const waiter = mediaWaiters.get(mediaRef);
          if (!waiter) return;
          waiter.chunks.push(String((msg as { data?: unknown }).data ?? ''));
          if ((msg as { done?: unknown }).done) {
            mediaWaiters.delete(mediaRef);
            waiter.resolve({ mime: waiter.mime, dataUrl: `data:${waiter.mime};base64,${waiter.chunks.join('')}` });
          }
          return;
        }
        if (msg.t === 'media_error') {
          const mediaRef = String((msg as { mediaRef?: unknown }).mediaRef ?? '');
          const waiter = mediaWaiters.get(mediaRef);
          if (waiter) {
            mediaWaiters.delete(mediaRef);
            waiter.reject(new Error(String((msg as { message?: unknown }).message ?? 'Image unavailable')));
          }
          return;
        }
        if (msg.t === 'btw_list' && btwListWaiters.has(String(msg.ref))) {
          const done = btwListWaiters.get(String(msg.ref))!;
          btwListWaiters.delete(String(msg.ref));
          const pages = Array.isArray(msg.pages) ? (msg.pages as BtwListPage[]) : [];
          // Newer bridges id every page and list every running one.
          done(pages, msg.complete === true);
          return;
        }
        if (msg.t === 'error' && btwListWaiters.has(String(msg.ref))) {
          btwListWaiters.get(String(msg.ref))!([], false);
          btwListWaiters.delete(String(msg.ref));
          return;
        }
        // UNI-222: an older bridge answers `lh_progress_get` with "unknown type" — no toast, just unsupported.
        if (msg.t === 'error' && lhProgressRefs.has(String(msg.ref))) {
          lhProgressRefs.delete(String(msg.ref));
          setState('lhProgressSupported', false);
          return;
        }
        if (msg.t === 'lh_progress' && msg.ref) lhProgressRefs.delete(String(msg.ref));
        // UNI-118: same for `dream_get` on an older bridge.
        if (msg.t === 'error' && dreamRefs.has(String(msg.ref))) {
          dreamRefs.delete(String(msg.ref));
          setState('dreamSupported', false);
          return;
        }
        if (msg.t === 'dream_status' && msg.ref) dreamRefs.delete(String(msg.ref));
        if ((msg.t === 'dream_detail' || msg.t === 'error') && dreamDetailWaiters.has(String(msg.ref))) {
          const done = dreamDetailWaiters.get(String(msg.ref))!;
          dreamDetailWaiters.delete(String(msg.ref));
          done(msg.t === 'dream_detail' ? (msg.detail as DreamDetail) : { error: friendlyBridgeError(msg.message as string | undefined) });
          return;
        }
        // UNI-282: agent-graph detail requests (output / history / transcript): reply or error.
        if (replyWaiters.has(String(msg.ref)) && (msg.t === replyWaiters.get(String(msg.ref))!.t || msg.t === 'error')) {
          const w = replyWaiters.get(String(msg.ref))!;
          replyWaiters.delete(String(msg.ref));
          w.done(msg);
          return;
        }
        if (msg.t === 'work_log' && workLogWaiters.has(String(msg.ref))) {
          const done = workLogWaiters.get(String(msg.ref))!;
          workLogWaiters.delete(String(msg.ref));
          done({ text: String(msg.text ?? ''), more: !!msg.more });
          return;
        }
        if (msg.t === 'error' && workLogWaiters.has(String(msg.ref))) {
          const done = workLogWaiters.get(String(msg.ref))!;
          workLogWaiters.delete(String(msg.ref));
          done({ error: friendlyBridgeError(msg.message as string | undefined) });
          return;
        }
        // pi is replacing the session (new / resume / fork / reload): its
        // bridge restarts; reconnect for the new session's hello.
        if (msg.t === 'reconnect') {
          // The switch we asked for (session_new / resume / fork / tree_go)
          // happened: its ack can't arrive (the old bridge is going away).
          for (const w of opWaiters.values()) w.resolve();
          opWaiters.clear();
          swapping = true;
          if (retry) clearTimeout(retry);
          retry = setTimeout(() => {
            swapping = false;
            attempts = 0;
            connect();
          }, 400);
          return;
        }
        if ((msg.t === 'ack' || msg.t === 'error') && opWaiters.has(String(msg.ref))) {
          const w = opWaiters.get(String(msg.ref))!;
          opWaiters.delete(String(msg.ref));
          if (msg.t === 'ack') w.resolve(msg);
          else w.reject(msg.code === 'busy' ? new BusyError(String(msg.message ?? 'pi is busy')) : new Error(w.raw ? String(msg.message ?? 'Something went wrong') : friendlyBridgeError(msg.message as string | undefined)));
          return;
        }
        if (msg.t === 'hello') {
          attempts = 0;
          setGeneration((g) => g + 1);
          batch(() => {
            setState('conn', 'live');
            setState('connMessage', undefined);
            setState(produce((s) => applyMessage(s, msg)));
            setState('entries', reconcile([...state.entries], { key: 'id' }));
          });
          flushOutbox();
          // A side question still waiting: its stream went to the old
          // connection. Re-list (and watch) to pick it back up.
          if (state.btwPages.some((p) => !p.done)) void syncBtw();
          return;
        }
        setState(produce((s) => applyMessage(s, msg)));
        // UNI-275: a navigate-to page landed — reached, or page further back.
        if (msg.t === 'history' && navWaiter) {
          const w = navWaiter;
          if (state.entries.some((e) => e.id === w.id)) {
            navWaiter = undefined;
            w.done(true);
          } else if (!state.hasOlder || !state.entries[0]) {
            navWaiter = undefined;
            w.done(false);
          } else {
            setState('loadingOlder', true);
            void sendRaw({ t: 'history', before: state.entries[0].id, until: w.id, ref: nextRef() });
          }
        }
      },
      (st, message, code) => {
        if (st === 'open') return;
        // The close that follows a `reconnect` is expected: stay "connecting".
        if (swapping) return;
        setState('conn', st === 'closed' ? 'closed' : 'error');
        setState('connMessage', message);
        // pi gone (not found / session ended) → stay closed; transport trouble → retry.
        const final = code === 'E_NOT_FOUND' || /session ended|is gone/i.test(message ?? '');
        if (!final && !closed && attempts < 6) {
          attempts++;
          retry = setTimeout(connect, Math.min(8000, 600 * 2 ** attempts));
        }
      },
    );
  };
  connect();

  // Time-settled pending bubbles (UNI-143): a message pi took but never
  // saved as typed goes quietly; one that never reached pi says so.
  const sweep = setInterval(() => {
    if (!state.pending.length) return;
    let lost: Pending[] = [];
    setState(produce((s) => (lost = sweepPending(s, Date.now()))));
    if (lost.length) setState('error', { message: lost.length === 1 ? "Your message didn't reach pi. Send it again." : `${lost.length} messages didn't reach pi.`, at: Date.now() });
  }, 2000);

  const sendRaw = (msg: Record<string, unknown>) => {
    const line = JSON.stringify(msg);
    return (conn ? conn.send(line) : Promise.reject(new Error('not connected'))).catch((error: unknown) => {
      setState('error', { message: String((error as { message?: string })?.message ?? error), at: Date.now() });
    });
  };

  /** Asks `btw_list{watch:true}` and folds it into `state.btwPages`; resolves
   *  with the pages that aren't this chat's own. Older bridges (no ids, no
   *  watch) are re-polled every 3 s while an own page is still waiting. */
  let btwPoll: ReturnType<typeof setTimeout> | undefined;
  function syncBtw(): Promise<BtwListPage[]> {
    if (state.conn !== 'live') return Promise.resolve([]);
    const ref = nextRef();
    return new Promise((resolve) => {
      btwListWaiters.set(ref, (pages, complete) => {
        let rest: BtwListPage[] = [];
        setState('btwPages', produce((own) => (rest = reconcileBtwPages(own, pages, complete))));
        if (btwPoll) clearTimeout(btwPoll);
        btwPoll = undefined;
        if (!complete && state.btwPages.some((p) => !p.done) && !closed) btwPoll = setTimeout(() => void syncBtw(), 3000);
        resolve(rest);
      });
      void sendRaw({ t: 'btw_list', watch: true, ref });
      setTimeout(() => {
        if (btwListWaiters.delete(ref)) resolve([]);
      }, 8000);
    });
  }

  /** Outbox messages (UNI-159): sent in order on every reconnect/hello. A
   *  message leaves the outbox as soon as it's handed to the bridge (acked
   *  or not — it becomes a normal `pending` bubble from there, same as any
   *  other send). */
  function flushOutbox() {
    const items = [...state.outbox];
    if (!items.length) return;
    setState('outbox', []);
    saveOutbox(hostId, pid, []);
    for (const item of items) doSend(item.text, 'auto', item.images);
  }

  /** Shared by `send()` and `flushOutbox()`: builds the pending bubble (except
   *  `after`, which shows in the queue strip instead — see the `queue` state)
   *  and sends the wire prompt. */
  function doSend(text: string, mode: 'auto' | 'steer' | 'followUp' | 'now' | 'after', images?: Outboxed['images']) {
    const ref = nextRef();
    if (mode === 'after') {
      void sendRaw({ t: 'prompt', text, mode, images, ref });
      return;
    }
    // 'now' delivers as a fresh prompt once pi goes idle: same bubble as any
    // other prompt (no "Sending…" beyond the usual ack wait).
    const effective: Pending['mode'] = mode === 'now' ? 'prompt' : !state.run.running ? 'prompt' : mode === 'followUp' ? 'followUp' : 'steer';
    setState('pending', (p) => [...p, { ref, text, mode: effective, at: Date.now() }]);
    void sendRaw({ t: 'prompt', text, mode, images, ref });
  }

  /** A request answered by `ack`/`error` (session_new, tree_go, …): resolves on `ack`, rejects on `error` (BusyError for `code:"busy"`). */
  /** Sends `msg` and resolves with the reply of type `t` (or the `error`), `Timed out` after 8 s. */
  const request = (msg: Record<string, unknown>, t: string): Promise<Msg> => {
    if (state.conn !== 'live') return Promise.resolve({ t: 'error', message: 'Not connected' });
    const ref = nextRef();
    return new Promise<Msg>((resolve) => {
      replyWaiters.set(ref, { t, done: resolve });
      void sendRaw({ ...msg, ref });
      setTimeout(() => {
        if (replyWaiters.delete(ref)) resolve({ t: 'error', message: 'Timed out' });
      }, 8000);
    });
  };

  const runOp = (msg: Record<string, unknown>): Promise<void> => runOpAck(msg).then(() => undefined);
  /** `runOp` whose rejection keeps the bridge's raw message (callers phrase it themselves). */
  const runOpRaw = (msg: Record<string, unknown>): Promise<void> => runOpAck(msg, true).then(() => undefined);
  /** `runOp` that hands back the `ack` line (undefined when a `reconnect` stood in for it). */
  const runOpAck = (msg: Record<string, unknown>, raw = false): Promise<Msg | undefined> => {
    if (state.conn !== 'live') return Promise.reject(new Error('Not connected'));
    const ref = nextRef();
    return new Promise<Msg | undefined>((resolve, reject) => {
      opWaiters.set(ref, { resolve, reject, raw });
      void sendRaw({ ...msg, ref });
      setTimeout(() => {
        if (opWaiters.delete(ref)) reject(new Error('Timed out'));
      }, 15000);
    });
  };

  return {
    state,
    generation,
    hostId,
    pid,
    /** Send a prompt. `mode`: `auto` (prompt when idle, steer when busy; default) \| `steer`
     * (next tool boundary) \| `followUp` (pi's own queued follow-up) \| `now` (abort the run,
     * wait for idle, then send as a prompt) \| `after` (wait in the bridge's own queue,
     * delivered on the next agent_end — shows in the queue strip, not a pending bubble).
     * `images`: inline base64 (small) or a host path (blob-channel upload; see `lib/chat/attachments.ts`). */
    send(text: string, mode: 'auto' | 'steer' | 'followUp' | 'now' | 'after' = 'auto', images?: Array<{ mime: string; data: string } | { mime: string; path: string }>) {
      // Not live: keep it in the outbox (persisted) instead of dropping it —
      // sent in order as soon as the next `hello` arrives.
      if (state.conn !== 'live') {
        const item: Outboxed = { id: nextRef(), text, images, at: Date.now() };
        setState('outbox', (o) => {
          const next = [...o, item];
          saveOutbox(hostId, pid, next);
          return next;
        });
        return;
      }
      doSend(text, mode, images);
    },
    /** Removes one message still waiting in the outbox (not yet sent: the connection wasn't live). */
    removeOutboxed(id: string) {
      setState('outbox', (o) => {
        const next = o.filter((x) => x.id !== id);
        saveOutbox(hostId, pid, next);
        return next;
      });
    },
    abort: () => void sendRaw({ t: 'abort', ref: nextRef() }),
    answer: (id: number, value: unknown) => {
      setState('dialogs', (d) => d.filter((x) => x.id !== id));
      void sendRaw({ t: 'answer', id, value, ref: nextRef() });
    },
    setModel: (provider: string, model: string) => void sendRaw({ t: 'set_model', provider, model, ref: nextRef() }),
    /** set_model as a request (UNI-212): resolves on ack, rejects with pi's error. */
    selectModel: (provider: string, model: string): Promise<void> => runOp({ t: 'set_model', provider, model }),
    /** set_thinking as a request (UNI-212). */
    selectThinking: (level: string): Promise<void> => runOp({ t: 'set_thinking', level }),
    setThinking: (level: string) => void sendRaw({ t: 'set_thinking', level, ref: nextRef() }),
    compact: (instructions?: string) => void sendRaw({ t: 'compact', instructions, ref: nextRef() }),
    /** UNI-275: similarity search over the whole session (not only what's paged in). null = this pi can't search. */
    search(query: string): Promise<SearchHit[] | null> {
      if (state.conn !== 'live') return Promise.resolve(null);
      const ref = nextRef();
      return new Promise((resolve) => {
        searchWaiters.set(ref, resolve);
        void sendRaw({ t: 'search', query, ref });
        setTimeout(() => {
          if (searchWaiters.delete(ref)) resolve(null);
        }, 10000);
      });
    },
    /** UNI-275: make sure entry `id` is in the transcript, paging older history in up to it
     * (the whole chat stays loaded — nothing is cut). Resolves false when it can't be reached. */
    navigateTo(id: string): Promise<boolean> {
      if (state.entries.some((e) => e.id === id)) return Promise.resolve(true);
      const first = state.entries[0];
      if (!first || !state.hasOlder || state.conn !== 'live') return Promise.resolve(false);
      navWaiter?.done(false);
      return new Promise<boolean>((resolve) => {
        const timer = setTimeout(() => {
          if (navWaiter?.id === id) navWaiter = undefined;
          resolve(false);
        }, 20000);
        navWaiter = { id, done: (ok) => (clearTimeout(timer), resolve(ok)) };
        setState('loadingOlder', true);
        void sendRaw({ t: 'history', before: first.id, until: id, ref: nextRef() });
      });
    },
    /** UNI-276: `/command <prefix>` argument suggestions (the command's own completer). */
    argCompletions(command: string, prefix: string): Promise<ArgCompletion[]> {
      if (state.conn !== 'live') return Promise.resolve([]);
      const ref = nextRef();
      return new Promise<ArgCompletion[]>((resolve) => {
        argWaiters.set(ref, resolve);
        void sendRaw({ t: 'arg_completions', command, prefix, ref });
        setTimeout(() => {
          if (argWaiters.delete(ref)) resolve([]);
        }, 4000);
      });
    },
    /** `@` suggestions for the text after `@`. An older pi (no `files` support) answers with an error: empty list after a timeout. */
    files(query: string): Promise<FileItem[]> {
      if (state.conn !== 'live') return Promise.resolve([]);
      const ref = nextRef();
      return new Promise<FileItem[]>((resolve) => {
        fileWaiters.set(ref, resolve);
        void sendRaw({ t: 'files', query, ref });
        setTimeout(() => {
          if (fileWaiters.delete(ref)) resolve([]);
        }, 5000);
      });
    },
    /** UNI-204: which chat-text paths exist on the PC (`paths_stat`). `null` = can't tell (offline,
     * timed out, or a pi without `paths_stat`) — the caller keeps those paths plain and may retry. */
    statPaths(paths: string[]): Promise<PathStatItem[] | null> {
      if (state.conn !== 'live' || paths.length === 0) return Promise.resolve(paths.length ? null : []);
      const ref = nextRef();
      return new Promise((resolve) => {
        statWaiters.set(ref, resolve);
        void sendRaw({ t: 'paths_stat', paths, ref });
        setTimeout(() => {
          if (statWaiters.delete(ref)) resolve(null);
        }, 8000);
      });
    },
    /** Sessions to resume: this project's (`cwd`) or every project's (`all`), optionally filtered. */
    sessions(scope: 'cwd' | 'all', query?: string): Promise<{ items: SessionsItem[]; more: boolean }> {
      if (state.conn !== 'live') return Promise.resolve({ items: [], more: false });
      const ref = nextRef();
      return new Promise((resolve) => {
        sessionsWaiters.set(ref, (items, more) => resolve({ items, more }));
        void sendRaw({ t: 'sessions', scope, query, ref });
        setTimeout(() => {
          if (sessionsWaiters.delete(ref)) resolve({ items: [], more: false });
        }, 8000);
      });
    },
    /** Fetches the full bytes behind an image `mediaRef` (see `imagesOf`'s
     * `ImageRef.mediaRef`): resolves a `data:` URL once every chunk arrived. */
    media(mediaRef: string, mime: string): Promise<{ mime: string; dataUrl: string }> {
      if (state.conn !== 'live') return Promise.reject(new Error('Not connected'));
      if (mediaWaiters.has(mediaRef)) return Promise.reject(new Error('Already fetching that image'));
      return new Promise((resolve, reject) => {
        mediaWaiters.set(mediaRef, { mime, chunks: [], resolve, reject });
        void sendRaw({ t: 'media', mediaRef });
        setTimeout(() => {
          if (mediaWaiters.delete(mediaRef)) reject(new Error('Timed out'));
        }, 15000);
      });
    },
    /** The whole session tree, every branch, previews only. */
    tree(): Promise<TreeNode[]> {
      if (state.conn !== 'live') return Promise.resolve([]);
      const ref = nextRef();
      return new Promise((resolve, reject) => {
        treeWaiters.set(ref, (got) => (got instanceof Error ? reject(got) : resolve(got)));
        void sendRaw({ t: 'tree', ref });
        setTimeout(() => {
          if (treeWaiters.delete(ref)) reject(new Error("The session tree didn't arrive (timed out)"));
        }, 15000);
      });
    },
    /** Starts a brand-new session. Rejects with BusyError while pi is running unless `force`. */
    sessionNew(force?: boolean): Promise<void> {
      return runOp({ t: 'session_new', force });
    },
    /** Resumes a session file. Rejects with BusyError while pi is running unless `force`. */
    sessionResume(path: string, force?: boolean): Promise<void> {
      return runOp({ t: 'session_resume', path, force });
    },
    /** Forks a new session file from a session entry. Rejects with BusyError while pi is running unless `force`. */
    sessionFork(entryId: string, force?: boolean): Promise<void> {
      return runOp({ t: 'session_fork', entryId, force });
    },
    /** Navigates to another point in the session tree. Rejects with BusyError while pi is running unless `force`.
     *  Resolves with the target's text/images when it was a user message (UNI-290; older bridges: undefined). */
    treeGo(id: string, summarize?: boolean, force?: boolean): Promise<TreeGoRestore | undefined> {
      return runOpAck({ t: 'tree_go', id, summarize, force }).then((ack) => treeGoRestoreOf(ack));
    },
    /** Sets the session's display name. */
    renameSession(name: string): Promise<void> {
      return runOp({ t: 'session_rename', name });
    },
    /** UNI-308: `/reload` — re-reads extensions, skills, prompts and context files (pi's own
     * `ctx.reload()`, same as the terminal's built-in). Rejects with BusyError while pi is
     * running unless `force`; an older bridge without this op rejects with the friendly
     * "this pi is older than the app" message (see `runOp`/`friendlyBridgeError`). */
    reload(force?: boolean): Promise<void> {
      return runOp({ t: 'reload', force });
    },
    /** Registers a path (seen as a tappable token in chat text) with the
     * bridge's allow-list so `blob_get{path}` can serve it; rejects when
     * the path is outside the session's cwd or doesn't exist on the host. */
    shareFile(path: string): Promise<void> {
      return runOp({ t: 'file_share', path });
    },
    /** Edits a bridge-queued ("after it ends") message's text. TUI items are never editable. */
    queueEdit(id: string, text: string) {
      void sendRaw({ t: 'queue_edit', id, text, ref: nextRef() });
    },
    /** Removes a bridge-queued message. */
    queueRemove(id: string) {
      void sendRaw({ t: 'queue_remove', id, ref: nextRef() });
    },
    /** Promotes a bridge-queued message to run now: `to:"steer"` (next tool boundary) or `to:"now"` (abort, wait, send as a prompt). */
    queuePromote(id: string, to: 'steer' | 'now') {
      void sendRaw({ t: 'queue_promote', id, to, ref: nextRef() });
    },
    /** Reorders a bridge-queued message to `index` (phone items only). */
    queueMove(id: string, index: number) {
      void sendRaw({ t: 'queue_move', id, index, ref: nextRef() });
    },
    /** Asks a side question (btw): streams into `state.btwPages` (matched by this call's own ref). Resolves once the answer (or an error) lands. */
    askBtw(question: string): { id: string; done: Promise<void> } {
      const ref = nextRef();
      if (state.conn !== 'live') {
        setState('btwPages', (p) => [...p, { id: ref, question, answer: '', done: true, error: 'Not connected to pi. Ask again once it reconnects.' }]);
        return { id: ref, done: Promise.resolve() };
      }
      setState('btwPages', (p) => [...p, { id: ref, question, answer: '', done: false }]);
      void sendRaw({ t: 'btw', question, ref });
      const done = new Promise<void>((resolve) => {
        const check = () => {
          const page = state.btwPages.find((p) => p.id === ref);
          if (page?.done) {
            resolve();
            return;
          }
          if (state.conn !== 'live') {
            resolve();
            return;
          }
          setTimeout(check, 200);
        };
        setTimeout(check, 200);
      });
      return { id: ref, done };
    },
    /** Switches to a single model (UNI-160 §1), with its own effort. Turning Fusion
     * off first (when it's on) is the caller's job — the phone UI asks "Turn Fusion
     * off?" before calling this; the wire message itself just applies. */
    setFusionSingle(model: string, effort: string): Promise<void> {
      return runOp({ t: 'set_fusion', single: model, effort });
    },
    /** Turns Fusion on (or edits the active pair/efforts). */
    setFusionPair(lead: string, sidekick: string, leadEffort: string, sidekickEffort: string): Promise<void> {
      return runOp({ t: 'set_fusion', lead, sidekick, leadEffort, sidekickEffort });
    },
    /** Starts/stops the `stats`/`info` push streams (UNI-160 §3/§5). */
    watch(opts: { stats?: boolean; info?: boolean }) {
      void sendRaw({ t: 'watch', ...opts, ref: nextRef() });
    },
    /** UNI-222: asks for the current long-horizon progress snapshot (pushes keep it live afterwards). */
    refreshProgress() {
      if (state.conn !== 'live') return;
      const ref = nextRef();
      lhProgressRefs.add(ref);
      void sendRaw({ t: 'lh_progress_get', ref });
    },
    /** UNI-269: switch the working mode (no prompt); `none` = plain. Resolves on ack (an `lh_mode` push follows), rejects with pi's reason. */
    setLhMode(mode: LhModeChoice): Promise<void> {
      return runOp({ t: 'set_lh_mode', mode });
    },
    /** UNI-118: asks for the current Dream status (pushes keep it live afterwards). */
    refreshDream() {
      if (state.conn !== 'live') return;
      const ref = nextRef();
      dreamRefs.add(ref);
      void sendRaw({ t: 'dream_get', ref });
    },
    /** UNI-118: one run's report markdown + live trajectory. */
    dreamDetail(id: string): Promise<DreamDetail | { error: string }> {
      if (state.conn !== 'live') return Promise.resolve({ error: 'Not connected' });
      const ref = nextRef();
      return new Promise((resolve) => {
        dreamDetailWaiters.set(ref, resolve);
        void sendRaw({ t: 'dream_detail', id, ref });
        setTimeout(() => {
          if (dreamDetailWaiters.delete(ref)) resolve({ error: 'Timed out' });
        }, 8000);
      });
    },
    /** UNI-118: run / stop / approve / reject / dismiss — resolves on ack, rejects with pi's reason. */
    dreamAction(action: DreamAction, id?: string, proposal?: string): Promise<void> {
      return runOp({ t: 'dream_action', action, ...(id ? { id } : {}), ...(proposal ? { proposal } : {}) });
    },
    /** A page of a bg task's live log, tail-first (UNI-160 §4). */
    workLog(id: string): Promise<{ text: string; more: boolean } | { error: string }> {
      if (state.conn !== 'live') return Promise.resolve({ error: 'Not connected' });
      const ref = nextRef();
      return new Promise((resolve) => {
        workLogWaiters.set(ref, resolve);
        void sendRaw({ t: 'work_log', id, ref });
        setTimeout(() => {
          if (workLogWaiters.delete(ref)) resolve({ error: 'Timed out' });
        }, 8000);
      });
    },
    /** UNI-282: a subagent's mini-transcript + report (`id` = work id `agent-<id>` or a graph step's `operatorId`). */
    async workTranscript(id: string): Promise<WorkTranscript | GraphError> {
      const msg = await request({ t: 'work_transcript', id }, 'work_transcript');
      if (msg.t === 'error') return graphErrorText(String(msg.message ?? 'Something went wrong'), 'Opening a step’s session');
      return {
        id: String(msg.id ?? id),
        items: Array.isArray(msg.items) ? (msg.items as TranscriptItem[]).filter((i) => !!i && typeof i === 'object' && typeof (i as { kind?: unknown }).kind === 'string') : [],
        ...(typeof msg.report === 'string' ? { report: msg.report } : {}),
      };
    },
    /** UNI-282: one graph/swarm step's full result (`graphId` for a past run from `graphHistory`). */
    async graphOutput(id: string, graphId?: string): Promise<LhGraphOutput | GraphError> {
      const msg = await request({ t: 'lh_graph', op: 'output', id, ...(graphId ? { graphId } : {}) }, 'lh_graph');
      if (msg.t === 'error') return graphErrorText(String(msg.message ?? 'Something went wrong'), 'Reading a step’s result');
      const out = msg.output as LhGraphOutput | undefined;
      return out && typeof out === 'object' ? out : { error: 'No result came back.' };
    },
    /** UNI-282: up to 20 earlier graphs of this project, newest first (each a finished LhRun with items). */
    async graphHistory(): Promise<LhRun[] | GraphError> {
      const msg = await request({ t: 'lh_graph', op: 'history' }, 'lh_graph');
      if (msg.t === 'error') return graphErrorText(String(msg.message ?? 'Something went wrong'), 'Earlier runs');
      return Array.isArray(msg.runs) ? msg.runs.filter(isRun) : [];
    },
    /** UNI-282: stops the running agent graph (running steps are cancelled). Rejects with pi's reason. */
    graphStop(): Promise<void> {
      // runOp already turns a raw "unknown type" into the generic restart hint;
      // keep this op's own, more specific wording.
      return runOpRaw({ t: 'lh_graph', op: 'stop' }).catch((e: unknown) => {
        throw new Error(graphErrorText(e instanceof Error ? e.message : String(e), 'Stopping the graph from the app').error);
      });
    },
    /** Stops a running bg task or cancels a running subagent. */
    workStop(id: string): Promise<void> {
      return runOp({ t: 'work_stop', id });
    },
    /** Re-runs a finished/stopped item. */
    workRerun(id: string): Promise<void> {
      return runOp({ t: 'work_rerun', id });
    },
    /** Sends a running foreground subagent to the background. */
    workBackground(id: string): Promise<void> {
      return runOp({ t: 'work_background', id });
    },
    /** Recent btw pages for this pi session (TUI + phone share history), minus this
     *  chat's own pages — those are updated in place in `state.btwPages` (a page
     *  that lost its stream gets its answer; nothing shows twice). */
    btwList(): Promise<BtwListPage[]> {
      return syncBtw();
    },
    /** Pages in older history (scroll-up). No-op while one is loading or none is left. */
    loadOlder() {
      const first = state.entries[0];
      if (!first || !state.hasOlder || state.loadingOlder || state.conn !== 'live') return;
      setState('loadingOlder', true);
      void sendRaw({ t: 'history', before: first.id, ref: nextRef() });
      // A lost reply must not block paging forever.
      setTimeout(() => setState('loadingOlder', false), 15000);
    },
    reconnect() {
      attempts = 0;
      if (retry) clearTimeout(retry);
      connect();
    },
    clearError: () => setState('error', undefined),
    clearRestored: () => setState('restored', undefined),
    close() {
      closed = true;
      clearInterval(sweep);
      if (retry) clearTimeout(retry);
      conn?.close();
    },
  };
}

/** UNI-290: the `restore` of a `tree_go` ack, validated (anything malformed → undefined / dropped). */
export function treeGoRestoreOf(ack: { [key: string]: unknown } | undefined): TreeGoRestore | undefined {
  const r = ack?.restore as { text?: unknown; images?: unknown } | undefined;
  if (!r || typeof r !== 'object' || typeof r.text !== 'string') return undefined;
  const images = (Array.isArray(r.images) ? r.images : [])
    .map((raw) => raw as { mime?: unknown; data?: unknown; mediaRef?: unknown; bytes?: unknown } | null)
    .filter((i): i is { mime?: unknown; data?: unknown; mediaRef?: unknown; bytes?: unknown } => !!i && (typeof i.data === 'string' || typeof i.mediaRef === 'string'))
    .map((i) => ({
      mime: typeof i.mime === 'string' ? i.mime : 'image/*',
      ...(typeof i.data === 'string' ? { data: i.data } : {}),
      ...(typeof i.mediaRef === 'string' ? { mediaRef: i.mediaRef } : {}),
      ...(typeof i.bytes === 'number' ? { bytes: i.bytes } : {}),
    }));
  if (!r.text && !images.length) return undefined;
  return { text: r.text, images };
}
