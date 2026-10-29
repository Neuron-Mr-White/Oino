/**
 * Kanboard API client + shared types (UNI-130). Mirrors the kanboard web
 * UI's `crates/kanboard/web/src/api.ts` — same shapes, same endpoints,
 * same helpers (LANES, PRIORITIES, canMove, needsComment, displayTitle) —
 * but every call goes through `boardTransport()` (lib/board/transport.ts)
 * and takes the PC (`hostId`) it talks to first. Never fetch() directly.
 */
import { boardTransport, type BoardMethod } from './transport';

// ─── shapes (mirror the daemon JSON) ───────────────────────────────────────

export interface Problem {
  file: string;
  line: number;
  error: string;
  fixable?: boolean;
}

export interface Run {
  session?: string;
  pid?: number;
  host?: string;
  mode?: string;
  /** "agent" when a session claimed it with `start`; "runner" only on old claims. */
  owner?: 'runner' | 'agent';
  goal?: string | null;
  started?: string;
}

export interface Activity {
  at: string;
  actor: string;
  /** Claiming session for agent writes (rendered as [agent:<session>]). */
  session?: string;
  text: string;
}

export interface Attachment {
  name: string;
  original: string;
  ref: string;
  path: string;
  size: number;
  mime: string;
  kind: 'image' | 'video' | 'audio' | 'pdf' | 'text' | 'file';
  markdown: string;
}

export interface Task {
  id: string;
  title: string;
  /** Title derived from the first body line when `title` is empty. */
  displayTitle?: string;
  /** Who created it — user|agent. */
  creator?: string;
  body?: string;
  status: string;
  priority: string;
  order: number;
  deps: string[];
  labels: string[];
  created: string;
  updated: string;
  run?: Run | null;
  activity: Activity[];
  ready?: boolean;
  waitingFor?: string[];
  /** Deps still in Backlog (or missing): only a human can schedule them. */
  lockedBy?: string[];
  attachments?: Attachment[];
  depsStatus?: Array<{ id: string; status: string | null }>;
  staleness?: string;
  allowedMoves?: string[];
  path?: string;
  /** Why the latest move into blocked happened. */
  blockedReason?: { text: string; actor: string; at: string };
  /** The full report behind In Review (finish summary) or Blocked (block reason). */
  statusReport?: { kind: 'review' | 'blocked'; text: string; actor: string; at: string };
}

export interface ProjectSummary {
  slug: string;
  archived?: boolean;
  name: string;
  root?: string;
  prefix?: string;
  counts?: Record<string, number>;
  total?: number;
  /** Tasks with a live run block. */
  running?: number;
  problems?: Problem[];
  /** Newest task change (RFC 3339), null for an empty project. */
  updatedAt?: string | null;
}

export interface Rules {
  statuses: string[];
  chainGate?: string;
  maxSessions?: number;
  allowedMoves: Record<string, string[]>;
  commentRequired: Record<string, Record<string, string>>;
  final: string[];
}

export interface Settings {
  piCommand: string[];
  models: string[];
  /** "" = pi's default model. */
  summaryModel: string;
  summaryInstruction: string;
  defaultSummaryInstruction: string;
  /** Session limits (older daemons call this `runner`). */
  sessions?: { maxSessions: number; turnAddLimit: number; chainGate: string };
  runner?: { maxSessions: number; turnAddLimit: number; chainGate: string };
  archive: { archiveAfterDays: number; retentionDays: number };
}

/** Flat PUT patch the daemon validates + writes. */
export interface SettingsPatch {
  summaryModel?: string;
  summaryInstruction?: string;
  maxSessions?: number;
  turnAddLimit?: number;
  chainGate?: string;
  archiveAfterDays?: number;
  retentionDays?: number;
}

export interface SummaryResult {
  summary: string;
  taskIds: string[];
}

export interface ArchiveResult {
  path: string;
  archived: string[];
  skipped: string[];
}

// Dashboard (`GET /api/dashboard`, kanboard UNI-67): the cross-project overview.
export interface DashboardProject {
  slug: string;
  review: number;
  blocked: number;
  ready: number;
  lastActivity: string | null;
  problems: number;
}
export interface DashboardInboxItem {
  slug: string;
  project: string;
  task: Task;
  waitingSince: string;
  excerpt: string | null;
}
export interface DashboardUpNextItem {
  slug: string;
  project: string;
  task: Task;
}
export interface DashboardActivityItem {
  slug: string;
  project: string;
  taskId: string;
  title: string;
  at: string;
  actor: string;
  session: string | null;
  text: string;
}
export interface DashboardData {
  generatedAt: string;
  inbox: DashboardInboxItem[];
  /** Every ready todo, uncapped (upNext is the capped top slice). */
  readyTotal: number;
  upNext: DashboardUpNextItem[];
  activity: DashboardActivityItem[];
  doneAt: string[];
  reviewWaits: number[];
  projects: DashboardProject[];
}

export interface RunningItem {
  slug: string;
  project: string;
  task: Task;
}

export type CreateTaskBody = {
  title: string;
  body?: string;
  status?: string;
  priority?: string;
  after?: string[];
  labels?: string[];
};

export type EditTaskPatch = { title?: string; body?: string; priority?: string; labels?: string[] };
export type OrderTarget = { before?: string; afterPos?: string; top?: boolean; bottom?: boolean };
export type SummarizeOptions = { instruction?: string; scope?: 'done' | 'board'; previous?: string; note?: string };

// ─── constants & helpers (identical to the web UI) ─────────────────────────

export const STATUSES = ['backlog', 'todo', 'in_progress', 'in_review', 'blocked', 'done', 'cancelled', 'archived'] as const;

export const LANES = [
  { id: 'backlog', label: 'Backlog' },
  { id: 'todo', label: 'Todo' },
  { id: 'in_progress', label: 'In Progress' },
  { id: 'blocked', label: 'Blocked' },
  { id: 'in_review', label: 'In Review' },
  { id: 'done', label: 'Done' },
  { id: 'cancelled', label: 'Cancelled' },
  { id: 'archived', label: 'Archive' },
] as const;
export type Lane = (typeof LANES)[number];

export const PRIORITIES = ['none', 'low', 'medium', 'high', 'urgent'] as const;
export const PRIORITY_LABEL: Record<string, string> = {
  none: 'No priority',
  low: 'Low',
  medium: 'Medium',
  high: 'High',
  urgent: 'Urgent',
};
export const PRIORITY_RANK: Record<string, number> = { urgent: 0, high: 1, medium: 2, low: 3, none: 4 };
export const MUTED_LANES = new Set(['done', 'cancelled', 'archived']);
/** Lanes counted as "open" work (project picker counts, like the web sidebar). */
export const OPEN_LANES = ['backlog', 'todo', 'in_progress', 'in_review', 'blocked'] as const;

export function laneLabel(status: string): string {
  return LANES.find((lane) => lane.id === status)?.label ?? status.replace('_', ' ');
}

/** The title a card/row shows: the server-derived one when `title` is empty. */
export function displayTitle(task: { title: string; displayTitle?: string }): string {
  return task.displayTitle || task.title || '(untitled)';
}

/** Fallback when `/api/rules` is unavailable: permissive, the server still decides. */
export const PERMISSIVE_RULES: Rules = {
  statuses: [...STATUSES],
  allowedMoves: {},
  commentRequired: {},
  final: ['done', 'cancelled', 'archived'],
};

/** Can a card move from its lane to `to`, according to the server's rules? */
export function canMove(rules: Rules, task: Task, to: string): boolean {
  if (to === task.status) return true;
  const explicit = task.allowedMoves;
  if (Array.isArray(explicit) && explicit.length > 0) return explicit.includes(to);
  const allowed = rules.allowedMoves[task.status];
  if (!allowed) return true; // unknown → let the server refuse (with a reason)
  return allowed.includes(to);
}

export function needsComment(rules: Rules, from: string, to: string): string | null {
  return rules.commentRequired[from]?.[to] ?? null;
}

// ─── errors ────────────────────────────────────────────────────────────────

export class ApiError extends Error {
  /** HTTP status; 0 = the transport itself failed (PC offline, no route). */
  readonly status: number;
  /** The move needs a comment (`commentRequired`). */
  readonly needsComment: boolean;
  /** Only an agent may do this (e.g. claim a task). */
  readonly needsAgent: boolean;

  constructor(message: string, status: number, needsComment = false, needsAgent = false) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.needsComment = needsComment;
    this.needsAgent = needsAgent;
  }
}

export function describe(error: unknown): string {
  if (error instanceof Error) return error.message;
  return String(error);
}

const decoder = new TextDecoder();

/** The daemon's error JSON (`{error, needsComment?, needsAgent?}`) → ApiError. */
export function parseError(status: number, text: string): ApiError {
  let payload: unknown = null;
  try {
    payload = text.length > 0 ? JSON.parse(text) : null;
  } catch {
    payload = null;
  }
  const record = (payload && typeof payload === 'object' ? payload : {}) as {
    error?: unknown;
    message?: unknown;
    needsComment?: unknown;
    needsAgent?: unknown;
  };
  const message =
    typeof record.error === 'string' && record.error
      ? record.error
      : typeof record.message === 'string' && record.message
        ? record.message
        : text && !payload && text.length < 200
          ? text.trim()
          : `request failed (${status})`;
  return new ApiError(message, status, record.needsComment === true, record.needsAgent === true);
}

async function send(hostId: string, method: BoardMethod, path: string, body?: string | Uint8Array) {
  try {
    return await boardTransport().request(hostId, { method, path, body });
  } catch (error) {
    throw new ApiError(describe(error) || 'the PC is unreachable', 0);
  }
}

async function request<T>(hostId: string, method: BoardMethod, path: string, json?: unknown): Promise<T> {
  const response = await send(hostId, method, path, json === undefined ? undefined : JSON.stringify(json));
  const text = decoder.decode(response.body);
  if (response.status < 200 || response.status >= 300) throw parseError(response.status, text);
  if (text.length === 0) return null as T;
  try {
    return JSON.parse(text) as T;
  } catch {
    throw new ApiError('the board answered with something that is not JSON', response.status);
  }
}

const enc = encodeURIComponent;
const taskPath = (slug: string, id: string) => `/api/tasks/${enc(slug)}/${enc(id)}`;

/** Daemon path of an attachment file (`att:<TASK>/<name>` refs resolve here). */
export function filePath(slug: string, taskId: string, name: string): string {
  return `/api/files/${enc(slug)}/${enc(taskId)}/${enc(name)}`;
}

const objectUrls = new Map<string, Promise<string>>();

// ─── endpoints ─────────────────────────────────────────────────────────────

export const api = {
  /** Daemon liveness/version (the transport's availability() wraps this). */
  health: (hostId: string) => request<{ ok: boolean; pid?: number; version?: string }>(hostId, 'GET', '/api/health'),
  projects: (hostId: string) => request<ProjectSummary[]>(hostId, 'GET', '/api/projects'),
  /** Every claimed task across all projects (the web sidebar's Agents list). */
  running: (hostId: string) => request<{ running: RunningItem[] }>(hostId, 'GET', '/api/running'),
  rules: (hostId: string) => request<Rules>(hostId, 'GET', '/api/rules'),
  /** Cross-project overview: needs-you inbox, up next, activity, throughput. */
  dashboard: (hostId: string) => request<DashboardData>(hostId, 'GET', '/api/dashboard'),
  tasks: (hostId: string, slug: string) =>
    request<{ tasks: Task[]; problems: Problem[] }>(hostId, 'GET', `/api/projects/${enc(slug)}/tasks`),
  task: (hostId: string, slug: string, id: string) => request<Task>(hostId, 'GET', taskPath(slug, id)),
  create: (hostId: string, slug: string, body: CreateTaskBody) =>
    request<Task>(hostId, 'POST', `/api/tasks/${enc(slug)}/create`, body),
  /** Moves a task; hard-to-reverse moves return a one-shot `undoToken`. */
  move: (hostId: string, slug: string, id: string, status: string, comment?: string) =>
    request<Task & { undoToken?: string }>(hostId, 'POST', `${taskPath(slug, id)}/move`, { status, comment }),
  /** Undoes one move with its one-shot token. */
  undoMove: (hostId: string, slug: string, id: string, token: string) =>
    request<Task>(hostId, 'POST', `${taskPath(slug, id)}/undo`, { token }),
  note: (hostId: string, slug: string, id: string, text: string) =>
    request<Task>(hostId, 'POST', `${taskPath(slug, id)}/note`, { text }),
  edit: (hostId: string, slug: string, id: string, patch: EditTaskPatch) =>
    request<Task>(hostId, 'POST', `${taskPath(slug, id)}/edit`, patch),
  link: (hostId: string, slug: string, id: string, dep: string) =>
    request<Task>(hostId, 'POST', `${taskPath(slug, id)}/link`, { dep }),
  unlink: (hostId: string, slug: string, id: string, dep: string) =>
    request<Task>(hostId, 'POST', `${taskPath(slug, id)}/unlink`, { dep }),
  order: (hostId: string, slug: string, id: string, target: OrderTarget) =>
    request<Task>(hostId, 'POST', `${taskPath(slug, id)}/order`, {
      before: target.before,
      after_pos: target.afterPos,
      top: target.top,
      bottom: target.bottom,
    }),
  duplicate: (hostId: string, slug: string, id: string) =>
    request<Task>(hostId, 'POST', `${taskPath(slug, id)}/duplicate`, {}),
  /** Uploads raw bytes as an attachment; returns its `markdown` to embed. */
  upload: async (hostId: string, slug: string, id: string, name: string, bytes: Uint8Array): Promise<Attachment> => {
    const response = await send(
      hostId,
      'POST',
      `${taskPath(slug, id)}/attachments?name=${enc(name || 'pasted')}`,
      bytes,
    );
    const text = decoder.decode(response.body);
    if (response.status === 413) throw new ApiError(`${name} is larger than 25 MB`, 413);
    if (response.status < 200 || response.status >= 300) throw parseError(response.status, text);
    return JSON.parse(text) as Attachment;
  },
  /** Convenience: uploads a File/Blob (reads it into bytes first). */
  uploadFile: async (hostId: string, slug: string, id: string, file: File): Promise<Attachment> =>
    api.upload(hostId, slug, id, file.name, new Uint8Array(await file.arrayBuffer())),
  /** Downloads an attachment (or any `/api/files/…` path) as a Blob. */
  file: async (hostId: string, path: string): Promise<Blob> => {
    const response = await send(hostId, 'GET', path);
    if (response.status < 200 || response.status >= 300) throw parseError(response.status, decoder.decode(response.body));
    return new Blob([response.body as BlobPart], { type: response.contentType || 'application/octet-stream' });
  },
  /** An object URL for a file path, cached per (host, path) for the session. */
  fileUrl: (hostId: string, path: string): Promise<string> => {
    const key = `${hostId}\u0000${path}`;
    let url = objectUrls.get(key);
    if (!url) {
      url = api.file(hostId, path).then((blob) => URL.createObjectURL(blob));
      url.catch(() => objectUrls.delete(key));
      objectUrls.set(key, url);
    }
    return url;
  },
  settings: (hostId: string) => request<Settings>(hostId, 'GET', '/api/settings'),
  saveSettings: (hostId: string, patch: SettingsPatch) => request<Settings>(hostId, 'PUT', '/api/settings', patch),
  /** The daemon-owned model catalog; `refresh` bypasses its cache. */
  models: (hostId: string, refresh = false) =>
    request<{ models: string[] }>(hostId, 'GET', `/api/models${refresh ? '?refresh=1' : ''}`),
  archiveLane: (hostId: string, slug: string, status: 'done' | 'in_review') =>
    request<{ archived: string[]; skipped: string[] }>(hostId, 'POST', `/api/projects/${enc(slug)}/archive-lane`, { status }),
  /** Moves every In Review task to Done. */
  doneLane: (hostId: string, slug: string) =>
    request<{ moved: string[] }>(hostId, 'POST', `/api/projects/${enc(slug)}/done-lane`),
  updateProject: (hostId: string, slug: string, patch: { archived?: boolean }) =>
    request<ProjectSummary>(hostId, 'PUT', `/api/projects/${enc(slug)}`, patch),
  summarize: (hostId: string, slug: string, opts?: SummarizeOptions) => {
    const body: Record<string, string> = {};
    if (opts?.instruction?.trim()) body.instruction = opts.instruction;
    if (opts?.scope) body.scope = opts.scope;
    if (opts?.previous?.trim()) body.previous = opts.previous;
    if (opts?.note?.trim()) body.note = opts.note;
    return request<SummaryResult>(hostId, 'POST', `/api/projects/${enc(slug)}/summarize`, body);
  },
  archiveSummary: (hostId: string, slug: string, body: { markdown: string; taskIds: string[] }) =>
    request<ArchiveResult>(hostId, 'POST', `/api/projects/${enc(slug)}/archive-summary`, body),
};

/** Test hook: forget cached object URLs. */
export function resetFileUrlCache(): void {
  objectUrls.clear();
}
