/**
 * Session data layer types (app spec §3.3–3.5, WP-A2).
 *
 * Entries are oino session entries (pi's format, which oino keeps):
 * `{type, id, parentId, timestamp, …}`; messages carry `message` (the
 * `AgentMessage` JSON). Event payloads are oino `ClientEvent` JSON narrowed by
 * `type` (host-only events by `kind`), never zod-parsed on the hot path
 * (ui-protocol D-U14).
 */
import type { AppError } from '@unipi/protocol';

/** A content block of an `AgentMessage` (text, thinking, toolCall, image, …). */
export type Part = { type: string; [key: string]: unknown };

export type Usage = { input?: number; output?: number; cacheRead?: number; cacheWrite?: number; totalTokens?: number; cost?: { total?: number } };

/** An `AgentMessage` (user, assistant, toolResult, bashExecution, custom, …). */
export type AgentMessage = {
  role: string;
  content?: string | Part[];
  timestamp?: number;
  stopReason?: string;
  errorMessage?: string;
  model?: string;
  provider?: string;
  usage?: Usage;
  toolCallId?: string;
  toolName?: string;
  isError?: boolean;
  details?: unknown;
  [key: string]: unknown;
};

/** One session entry. Provisional live entries (not yet confirmed by the host) have ids `live:<n>`. */
export type SessionEntry = {
  type: string;
  id: string;
  parentId?: string | null;
  timestamp?: string;
  message?: AgentMessage;
  customType?: string;
  data?: unknown;
  [key: string]: unknown;
};

/** The id prefix of entries the reducer made from `message_end` before the host confirmed them. */
export const LIVE_PREFIX = 'live:';
export const isProvisional = (id: string): boolean => id.startsWith(LIVE_PREFIX);

/** The assistant message being streamed (`message_start` … `message_end`). */
export type Streaming = { message: AgentMessage; startedAt: number };

/** A tool execution seen on the stream (`tool_execution_start|update|end`). */
export type LiveTool = {
  id: string;
  name: string;
  args: Record<string, unknown>;
  /** Text of the latest `partialResult` (live output tail). */
  partial?: string;
  done?: boolean;
  isError?: boolean;
  result?: unknown;
  durationMs?: number;
  parentId?: string;
  startedAt: number;
};

/** Coarse run state (`run_state` events + `agent_start/end`, `compaction_*`, `auto_retry_*`). */
export type RunPhase = 'idle' | 'running' | 'compacting' | 'retrying' | 'waiting';
export type RunInfo = {
  phase: RunPhase;
  /** An agent run is active (agent_start … agent_end). */
  running: boolean;
  /** When the current run started (epoch ms); the "Working for" clock. */
  startedAt?: number;
  waitingOn?: string[];
  retry?: { attempt: number; maxAttempts: number; delayMs: number; error: string };
};

/** pi's two queues (`queue_update`). */
export type QueueState = { steering: string[]; followUp: string[] };

/** Host-only events (`crashed` / `taken_over` / `reloaded`) shown as a banner above the timeline. */
export type BannerKind = 'crashed' | 'taken_over' | 'reloaded' | 'wake_failed';
export type Banner = { kind: BannerKind; text: string; at: number; from?: string };

/**
 * The stream connection (Z-50):
 * - `connecting`: first open in flight;
 * - `live`: snapshot applied, events flowing;
 * - `reconnecting`: the stream dropped, retrying with `sinceSeq` (backoff 0.5/1/2/5 s);
 * - `lost`: gave up retrying (banner "Lost the connection · Retry");
 * - `closed`: final (session gone / closed by us).
 */
export type ConnState = 'connecting' | 'live' | 'reconnecting' | 'lost' | 'closed';

/**
 * An outgoing prompt (Z-50, ported `Pending`/`Outboxed`):
 * - `waiting`: typed while not live; persisted, sent in order once live ("Waiting for connection… ✕");
 * - `sending`: the command is in flight;
 * - `acked`: the host took it; the bubble stays until its user `message_start`/`message_end` arrives;
 * - `lost`: the command failed or never got an ack ("Not sent · Retry").
 */
export type OutboxStatus = 'waiting' | 'sending' | 'acked' | 'lost';
export type OutboxMode = 'prompt' | 'steer' | 'followUp';
export type OutboxImage = { mimeType: string; data: string };
export type OutboxItem = {
  ref: string;
  text: string;
  images?: OutboxImage[];
  mode: OutboxMode;
  at: number;
  status: OutboxStatus;
  /** When the host acked it (the `acked` timeout counts from here). */
  ackedAt?: number;
  error?: string;
};

/** A sticky bus topic value (`topic` events, snapshot `topics`). */
export type TopicValue = { data: unknown; sticky: boolean; publisher: string };

/** A slash command (`get_commands` shape). */
export type CommandInfo = { name: string; description?: string; source?: string; [key: string]: unknown };

/** Per open session (app spec §3.3 `session` store). */
export type SessionState = {
  hostId: string;
  sessionId: string;
  /** Last applied event seq (0 before any). */
  seq: number;
  /** Every entry the host sent (all branches when the snapshot has them), oldest first. */
  entries: SessionEntry[];
  /** id → index into `entries`. */
  byId: Record<string, number>;
  leafId: string | null;
  /** Bumped whenever `entries` change other than by appending (snapshot, reset, reconcile, tree). */
  entriesRev: number;
  streaming?: Streaming;
  liveTools: Record<string, LiveTool>;
  run: RunInfo;
  queue: QueueState;
  topics: Record<string, TopicValue>;
  commands: CommandInfo[];
  /** `RpcSessionState` (`get_state`) as the snapshot carried it. */
  info: Record<string, unknown>;
  /** Display name (`session_info_changed` / `state.sessionName`). */
  name?: string;
  /** The selected model (`model_changed` / `state.model`). */
  model?: { provider?: string; id?: string; name?: string; [key: string]: unknown };
  thinkingLevel?: string;
  /** Open extension dialogs (needs-you), by request id. */
  dialogs: string[];
  conn: ConnState;
  connError?: AppError;
  outbox: OutboxItem[];
  banner?: Banner;
  /** perf-open: `entries` is the newest part of the branch; older pages load on scroll. */
  history?: HistoryState;
  /** The snapshot came from the session file while the runtime wakes (`woken` clears it). */
  waking?: boolean;
};

/** Paged history (snapshot `page`, `get_entries{before}` replies). */
export type HistoryState = {
  /** Entries on the whole current branch. */
  total: number;
  /** Older entries exist before the loaded ones. */
  hasMore: boolean;
  /** `before` for the next older page (the first loaded entry). */
  cursor?: string;
  /** A "load earlier" fetch is in flight. */
  loading?: boolean;
  /** The last fetch failed (shown with a retry; auto-loading stops). */
  error?: string;
};
