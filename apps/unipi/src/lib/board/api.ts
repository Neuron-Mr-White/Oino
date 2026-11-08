/**
 * Kanboard API client v2 (WP-A22, G-18): the same surface the Board UI has
 * always consumed (types, constants, helpers unchanged from the daemon era),
 * but every call maps onto a `unipi-kanboard` subcommand run through
 * `boardTransport().cli()` (ext-abi §5.5), actor `user`, `--json`.
 *
 * What moved where (no daemon anymore):
 *  - `rules` is a client mirror of the transition table's user rows
 *    (`USER_RULES`) — the daemon served the same JSON; the CLI does not;
 *  - `projects`, `running` and `dashboard` are derived from
 *    `project list` + per-project `list --all` exactly the way the daemon
 *    derived them server-side (dashboard.ts keeps the view logic);
 *  - `doneLane`/`archiveLane` are composed per-task `move`s (the bulk
 *    commands are daemon leftovers, not CLI subcommands);
 *  - attachments upload by staging the bytes as a host blob
 *    (`blob_put_bytes` + `fs_import` into `~/.oino/uploads`) and handing the
 *    host path to `attach`; downloads read the stored file by its board path
 *    through `blob_get_bytes`.
 */
import { boardTransport, type BoardCliOutcome } from './transport';
import { hostClient } from '../host/factory';

// ─── shapes (the CLI's JSON, same as the v1 daemon's) ──────────────────────

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
  owner?: 'runner' | 'agent' | 'dispatcher';
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
  autonomy?: 'low' | 'medium' | 'high';
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

// Dashboard (the cross-project overview; derived from per-project lists).
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

// ─── constants & helpers (unchanged) ───────────────────────────────────────

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

/**
 * The user-actor rows of the kanboard transition table
 * (`apps/unipi/crates/unipi-kanboard/src/transitions.rs` RULES), mirrored the way the
 * daemon's `/api/rules` served them. The server still decides — this only
 * dims invalid targets and opens the comment sheet before a refusal.
 */
export const USER_RULES: Rules = {
  statuses: [...STATUSES],
  allowedMoves: {
    backlog: ['todo', 'cancelled'],
    todo: ['backlog', 'cancelled'],
    in_progress: ['todo', 'backlog'],
    blocked: ['todo', 'backlog', 'done', 'cancelled'],
    in_review: ['done', 'todo', 'backlog', 'archived'],
    done: ['archived'],
    cancelled: ['archived'],
    archived: [],
  },
  commentRequired: {
    in_progress: {
      todo: 'why the stale run is being released',
      backlog: 'why the stale run is being released',
    },
  },
  final: ['done', 'cancelled', 'archived'],
};

/** Fallback when a host predates the board delta: permissive, the server still decides. */
export const PERMISSIVE_RULES: Rules = {
  statuses: [...STATUSES],
  allowedMoves: {},
  commentRequired: {},
  final: ['done', 'cancelled', 'archived'],
};

/** Can a card move from its lane to `to`, according to the rules? */
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
  /** 0 = the transport itself failed (PC offline, no route); 409 = a rule refusal. */
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

/** A CLI failure line (`{ok:false, error, kind}`) → `ApiError`. */
export function parseError(outcome: { ok: false; error?: string; kind?: string }): ApiError {
  const message = outcome.error?.trim() || 'the board refused the change';
  const status = outcome.kind === 'usage' ? 400 : outcome.kind === 'not_found' ? 404 : 409;
  const needsComment = outcome.kind === 'rule' && /requires --comment/i.test(message);
  const needsAgent = outcome.kind === 'rule' && /\bagent\b/i.test(message) && /only/i.test(message);
  return new ApiError(message, status, needsComment, needsAgent);
}

/** Runs one CLI call; throws `ApiError` for refusals and host failures alike. */
async function cli<T>(hostId: string, args: string[], opts?: { project?: string }): Promise<T> {
  let outcome: BoardCliOutcome;
  try {
    outcome = await boardTransport().cli(hostId, args, opts?.project);
  } catch (error) {
    const message = describe(error) || 'the PC is unreachable';
    throw new ApiError(message, 0);
  }
  if (!outcome.ok) throw parseError(outcome);
  return outcome.payload as T;
}

/** Board-relative path of an attachment file (`att:<TASK>/<name>` refs resolve here; stored names are already filesystem-safe). */
export function filePath(slug: string, taskId: string, name: string): string {
  return `projects/${slug}/attachments/${taskId}/${name}`;
}

// ─── per-host board home (attachment paths resolve against it) ─────────────

const homes = new Map<string, string>();

/** Remembers the board home a `status` call reported. */
export function setBoardHome(hostId: string, home: string | undefined): void {
  if (home) homes.set(hostId, home.replace(/\/$/, ''));
  else homes.delete(hostId);
}

export function boardHome(hostId: string): string | undefined {
  return homes.get(hostId);
}

const objectUrls = new Map<string, Promise<string>>();

// ─── endpoints ─────────────────────────────────────────────────────────────

/** One project's tasks + problems (`list --all`: every lane, like the daemon). */
export const api = {
  /** CLI liveness + home (the transport's availability() wraps this). */
  health: (hostId: string) => {
    interface StatusPayload {
      ok: boolean;
      home?: string;
      binary?: string;
      hostname?: string;
    }
    return (async () => {
      const payload = await cli<StatusPayload>(hostId, ['status']);
      setBoardHome(hostId, payload.home);
      return payload;
    })();
  },
  /** Every project with counts/running/updatedAt/problems, derived per project. */
  projects: async (hostId: string): Promise<ProjectSummary[]> => {
    const projects = await cli<Array<{ slug: string; name: string; root?: string; prefix?: string; archived?: boolean }>>(hostId, ['project', 'list']);
    const summaries = await Promise.all(
      projects.map(async (project) => summarizeProject(hostId, project)),
    );
    return summaries;
  },
  /** Every claimed task across all projects (the sidebar's Agents list). */
  running: async (hostId: string): Promise<{ running: RunningItem[] }> => {
    const index = await boardIndex(hostId);
    return { running: index.running };
  },
  /** The user-actor transition table (a client mirror — no call). */
  rules: (hostId: string): Promise<Rules> => Promise.resolve({ ...USER_RULES, statuses: [...USER_RULES.statuses] }),
  /** Cross-project overview, derived exactly as the daemon derived it. */
  dashboard: async (hostId: string): Promise<DashboardData> => {
    const index = await boardIndex(hostId);
    return deriveDashboard(index, Date.now());
  },
  tasks: (hostId: string, slug: string) => cli<{ tasks: Task[]; problems: Problem[] }>(hostId, ['list', '--all'], { project: slug }),
  task: (hostId: string, slug: string, id: string) => cli<Task>(hostId, ['show', id], { project: slug }),
  create: (hostId: string, slug: string, body: CreateTaskBody) => {
    const args = ['add'];
    if (body.title) args.push(body.title);
    if (body.body !== undefined) args.push(`--body=${body.body}`);
    if (body.status) args.push('--status', body.status);
    if (body.priority) args.push('--priority', body.priority);
    for (const dep of body.after ?? []) args.push('--after', dep);
    for (const label of body.labels ?? []) args.push('--label', label);
    if ((body.labels ?? []).length > 0) args.push('--new-label');
    return cli<Task>(hostId, args, { project: slug });
  },
  move: (hostId: string, slug: string, id: string, status: string, comment?: string) => {
    const args = ['move', id, status];
    if (comment) args.push('--comment', comment);
    return cli<Task>(hostId, args, { project: slug });
  },
  note: (hostId: string, slug: string, id: string, text: string) =>
    cli<Task>(hostId, ['note', id, '--', text], { project: slug }),
  edit: (hostId: string, slug: string, id: string, patch: EditTaskPatch) => {
    const args = ['edit', id];
    if (patch.title !== undefined) args.push(`--title=${patch.title}`);
    if (patch.body !== undefined) args.push(`--body=${patch.body}`);
    if (patch.priority !== undefined) args.push('--priority', patch.priority);
    if (patch.labels !== undefined) args.push(`--labels=${patch.labels.join(',')}`, '--new-label');
    return cli<Task>(hostId, args, { project: slug });
  },
  link: (hostId: string, slug: string, id: string, dep: string) =>
    cli<Task>(hostId, ['link', id, '--after', dep], { project: slug }),
  unlink: (hostId: string, slug: string, id: string, dep: string) =>
    cli<Task>(hostId, ['unlink', id, '--after', dep], { project: slug }),
  order: (hostId: string, slug: string, id: string, target: OrderTarget) => {
    const args = ['order', id];
    if (target.before) args.push('--before', target.before);
    else if (target.afterPos) args.push('--after-pos', target.afterPos);
    else if (target.top) args.push('--top');
    else args.push('--bottom');
    return cli<Task>(hostId, args, { project: slug });
  },
  duplicate: (hostId: string, slug: string, id: string) => cli<Task>(hostId, ['duplicate', id], { project: slug }),
  /** Uploads raw bytes as an attachment: stage on the host, then `attach`. */
  upload: async (hostId: string, slug: string, id: string, name: string, bytes: Uint8Array): Promise<Attachment> => {
    const staged = await stageBytes(hostId, name, bytes);
    try {
      return await cli<Attachment>(hostId, ['attach', id, staged.path, '--name', name], { project: slug });
    } finally {
      void staged.cleanup().catch(() => {});
    }
  },
  /** Convenience: uploads a File/Blob (reads it into bytes first). */
  uploadFile: async (hostId: string, slug: string, id: string, file: File): Promise<Attachment> =>
    api.upload(hostId, slug, id, file.name, new Uint8Array(await file.arrayBuffer())),
  /** Downloads an attachment (or any board-relative path) as a Blob. */
  file: async (hostId: string, path: string): Promise<Blob> => {
    const absolute = resolveBoardPath(hostId, path);
    const result = (await hostClient(hostId).request('blob_get_bytes', { path: absolute })) as { mime: string; bytes: number[] };
    return new Blob([new Uint8Array(result.bytes)], { type: result.mime || 'application/octet-stream' });
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
  /** "Add project": registers a folder on the PC (the host checks it is inside the folders the app may see). */
  addProject: async (hostId: string, root: string, name?: string): Promise<void> => {
    await cli(hostId, ['project', 'add', '--root', root, ...(name ? ['--name', name] : [])]);
  },
  updateProject: async (hostId: string, slug: string, patch: { archived?: boolean }): Promise<ProjectSummary> => {
    if (patch.archived === undefined) return (await api.projects(hostId)).find((project) => project.slug === slug)!;
    await cli(hostId, ['project', patch.archived ? 'archive' : 'unarchive', slug]);
    const projects = await api.projects(hostId);
    return projects.find((project) => project.slug === slug)!;
  },
  /** Moves every In Review task to Done (composed per-task moves). */
  doneLane: async (hostId: string, slug: string): Promise<{ moved: string[] }> => {
    const { tasks } = await api.tasks(hostId, slug);
    const moved: string[] = [];
    for (const task of tasks.filter((candidate) => candidate.status === 'in_review')) {
      try {
        await api.move(hostId, slug, task.id, 'done');
        moved.push(task.id);
      } catch {
        // A rule refusal skips that task (the daemon's bulk pass did the same).
      }
    }
    return { moved };
  },
  /** Archives every task of a lane (done or in_review), skipping refusals. */
  archiveLane: async (hostId: string, slug: string, status: 'done' | 'in_review'): Promise<{ archived: string[]; skipped: string[] }> => {
    const { tasks } = await api.tasks(hostId, slug);
    const archived: string[] = [];
    const skipped: string[] = [];
    for (const task of tasks.filter((candidate) => candidate.status === status)) {
      try {
        await api.move(hostId, slug, task.id, 'archived');
        archived.push(task.id);
      } catch {
        skipped.push(task.id);
      }
    }
    return { archived, skipped };
  },
};

// ─── staging (attachment upload) ───────────────────────────────────────────

type Staged = { path: string; cleanup: () => Promise<void> };

/** Upload bytes to the host (`blob_put_bytes`) and import them into `~/.oino/uploads`. */
async function stageBytes(hostId: string, name: string, bytes: Uint8Array): Promise<Staged> {
  const safe = name.replace(/[^\w.-]+/g, '-').replace(/^[-.]+|[-.]+$/g, '') || 'file';
  const put = (await hostClient(hostId).request('blob_put_bytes', {
    bytes: Array.from(bytes),
    mime: 'application/octet-stream',
    name: safe,
  })) as { sha256: string };
  const path = `~/.oino/uploads/board-${Date.now().toString(36)}-${safe}`;
  await hostClient(hostId).request('fs_import', { sha256: put.sha256, to: path });
  return {
    path,
    cleanup: () => hostClient(hostId).request('fs_delete', { path, confirm: true }),
  };
}

/** A board-relative path → the absolute host path (needs the home from `status`). */
function resolveBoardPath(hostId: string, path: string): string {
  if (path.startsWith('/')) return path;
  const home = boardHome(hostId);
  if (!home) throw new ApiError('the board home is not known yet — open the board once', 0);
  return `${home}/${path}`;
}

/** Test hook: forget cached object URLs and homes. */
export function resetFileUrlCache(): void {
  objectUrls.clear();
  homes.clear();
}

// ─── derivation (what the daemon computed server-side) ─────────────────────

type ProjectMeta = { slug: string; name: string; root?: string; prefix?: string; archived?: boolean };

type BoardIndex = {
  projects: ProjectMeta[];
  perProject: Array<{ meta: ProjectMeta; tasks: Task[]; problems: Problem[] }>;
  running: RunningItem[];
};

/** `project list` + one `list --all` per project (the daemon's dashboard scan). */
async function boardIndex(hostId: string): Promise<BoardIndex> {
  const projects = await cli<ProjectMeta[]>(hostId, ['project', 'list']);
  const perProject = await Promise.all(
    projects.map(async (meta) => {
      try {
        const payload = await cli<{ tasks: Task[]; problems: Problem[] }>(hostId, ['list', '--all'], { project: meta.slug });
        return { meta, tasks: payload.tasks ?? [], problems: payload.problems ?? [] };
      } catch {
        // An unreadable project shows as empty with its problems, like the daemon.
        return { meta, tasks: [], problems: [] as Problem[] };
      }
    }),
  );
  const running: RunningItem[] = [];
  for (const { meta, tasks } of perProject) {
    for (const task of tasks) {
      if (task.run) running.push({ slug: meta.slug, project: meta.name, task });
    }
  }
  return { projects, perProject, running };
}

/** One project's sidebar summary: counts, total, running, newest change, problems. */
async function summarizeProject(hostId: string, meta: ProjectMeta): Promise<ProjectSummary> {
  let tasks: Task[] = [];
  let problems: Problem[] = [];
  try {
    const payload = await cli<{ tasks: Task[]; problems: Problem[] }>(hostId, ['list', '--all'], { project: meta.slug });
    tasks = payload.tasks ?? [];
    problems = payload.problems ?? [];
  } catch {
    // Unreadable board: keep the project listed with no counts.
  }
  return projectSummaryOf(meta, tasks, problems);
}

/** Pure half of `summarizeProject` (tests). */
export function projectSummaryOf(meta: ProjectMeta, tasks: Task[], problems: Problem[]): ProjectSummary {
  const counts: Record<string, number> = {};
  for (const status of STATUSES) counts[status] = 0;
  for (const task of tasks) counts[task.status] = (counts[task.status] ?? 0) + 1;
  return {
    slug: meta.slug,
    name: meta.name,
    root: meta.root,
    prefix: meta.prefix,
    archived: meta.archived,
    counts,
    total: tasks.length,
    running: tasks.filter((task) => task.run).length,
    problems,
    updatedAt: tasks.reduce<string | null>(
      (newest, task) => (newest === null || task.updated > newest ? task.updated : newest),
      null,
    ),
  };
}

/** The status an activity entry moved its task INTO (the write path's vocabulary). */
export function enteredStatus(text: string): string | null {
  const t = text.trim();
  if (t.startsWith('finished:')) return 'in_review';
  if (t.startsWith('released to ')) return t.slice(12).split(':')[0]?.trim() ?? null;
  if (t === 'blocked' || t.startsWith('blocked:')) return 'blocked';
  if (t === 'unblocked' || t.startsWith('unblocked:') || t === 'rework' || t.startsWith('rework:')) return 'todo';
  if (t === 'cancelled' || t.startsWith('cancelled:')) return 'cancelled';
  if (t === 'archived' || t.startsWith('archived ') || t.startsWith('archived:')) return 'archived';
  if (t.startsWith('moved ')) {
    const to = t.slice(6).split(' → ').pop()?.trim() ?? '';
    return to.split(':')[0]?.replace(/ \(bulk\)$/, '').trim() || null;
  }
  if (t.startsWith('undo: ')) {
    const from = t.slice(6).split(' → ').pop()?.split(' (')[0]?.trim() ?? '';
    return from || null;
  }
  if (t.startsWith('created in ')) return t.slice(11).split(/\s+/)[0] ?? null;
  return null;
}

/** The latest activity entry that moved `task` INTO `status`. */
function transitionInto(task: Task, status: string): Activity | undefined {
  for (let i = task.activity.length - 1; i >= 0; i -= 1) {
    const entry = task.activity[i]!;
    if (enteredStatus(entry.text) === status) return entry;
  }
  return undefined;
}

/** First line that carries text. */
function firstNonEmptyLine(text: string): string {
  return (
    text
      .split('\n')
      .map((line) => line.trim())
      .find((line) => line.length > 0) ?? ''
  );
}

/** Drop the transition prefix so the excerpt is just what the agent said. */
function stripTransitionPrefix(text: string): string {
  const t = text.trim();
  if (t.startsWith('released to ')) {
    const rest = t.slice(12);
    const at = rest.indexOf(':');
    return at === -1 ? rest : rest.slice(at + 1).trim();
  }
  for (const prefix of ['blocked:', 'finished:']) {
    if (t.startsWith(prefix)) return t.slice(prefix.length).trim();
  }
  return t;
}

/** Remove markdown image embeds — they never read well in a one-line excerpt. */
function stripMarkdownImages(text: string): string {
  return text.replace(/!\[[^\]]*\]\([^)]*\)/g, '');
}

/** Char-safe cut with an ellipsis. */
function excerptChars(text: string, max: number): string {
  const trimmed = stripMarkdownImages(firstNonEmptyLine(text)).trim();
  const chars = [...trimmed];
  if (chars.length <= max) return trimmed;
  return `${chars.slice(0, Math.max(0, max - 1)).join('')}…`;
}

const ACTIVITY_WINDOW_MS = 48 * 3600_000;
const THROUGHPUT_WINDOW_MS = 14 * 24 * 3600_000;
const UP_NEXT_CAP = 8;
const ACTIVITY_CAP = 60;
const EXCERPT_MAX = 160;

/** Pure dashboard derivation (the daemon's `GET /api/dashboard` logic). */
export function deriveDashboard(index: BoardIndex, now: number): DashboardData {
  const inbox: DashboardInboxItem[] = [];
  const upNext: DashboardUpNextItem[] = [];
  let upNextBeforeCap = 0;
  const activity: Array<{ at: number; seq: number; item: DashboardActivityItem }> = [];
  let seq = 0;
  const doneAt: string[] = [];
  const reviewWaits: number[] = [];
  const projects: DashboardProject[] = [];

  for (const { meta, tasks, problems } of index.perProject) {
    if (meta.archived) continue;
    let review = 0;
    let blocked = 0;
    let ready = 0;
    let lastActivity: string | null = null;

    for (const task of tasks) {
      if (lastActivity === null || task.updated > lastActivity) {
        lastActivity = task.updated;
      }
      if (task.status === 'in_review' || task.status === 'blocked') {
        if (task.status === 'in_review') review += 1;
        else blocked += 1;
        const entry = transitionInto(task, task.status);
        const waitingSince = entry?.at ?? task.updated;
        // An undo re-entered the status without carrying the agent's words —
        // fall back to the previous real transition into the same status.
        let excerptEntry = entry;
        if (entry && entry.text.startsWith('undo:')) {
          const at = task.activity.lastIndexOf(entry);
          for (let i = at - 1; i >= 0; i -= 1) {
            const candidate = task.activity[i]!;
            if (!candidate.text.startsWith('undo:') && enteredStatus(candidate.text) === task.status) {
              excerptEntry = candidate;
              break;
            }
          }
        }
        inbox.push({
          slug: meta.slug,
          project: meta.name,
          task,
          waitingSince,
          excerpt: excerptEntry ? excerptChars(stripTransitionPrefix(excerptEntry.text), EXCERPT_MAX) : null,
        });
      } else if (task.status === 'todo') {
        const isReady = task.ready === true;
        if (isReady) ready += 1;
        if (isReady && !task.run) {
          upNextBeforeCap += 1;
          upNext.push({ slug: meta.slug, project: meta.name, task });
        }
      }

      // Done events, revert-aware: an `undo: done → …` after a transition into
      // done means that done never happened — drop the recorded stamp.
      const doneStamps: number[] = [];
      for (const entry of task.activity) {
        if (enteredStatus(entry.text) === 'done') doneStamps.push(Date.parse(entry.at));
        else if (entry.text.trim().startsWith('undo: ')) {
          const reverted = entry.text.trim().slice(6).split(' → ')[0]?.trim();
          if (reverted === 'done') doneStamps.pop();
        }
      }
      for (const entry of task.activity) {
        const at = Date.parse(entry.at);
        if (now - at <= ACTIVITY_WINDOW_MS) {
          seq += 1;
          activity.push({
            at,
            seq,
            item: {
              slug: meta.slug,
              project: meta.name,
              taskId: task.id,
              title: displayTitle(task),
              at: entry.at,
              actor: entry.actor,
              session: entry.session ?? null,
              text: entry.text,
            },
          });
        }
      }
      for (const stamp of doneStamps) {
        if (now - stamp <= THROUGHPUT_WINDOW_MS) doneAt.push(new Date(stamp).toISOString());
      }

      // Completed review cycles: entering in_review, next transition leaving it.
      for (let i = 0; i < task.activity.length; i += 1) {
        const entry = task.activity[i]!;
        if (enteredStatus(entry.text) !== 'in_review') continue;
        const exit = task.activity.slice(i + 1).find((later) => {
          const left = enteredStatus(later.text);
          return left !== null && left !== 'in_review';
        });
        if (exit) {
          const wait = (Date.parse(exit.at) - Date.parse(entry.at)) / 1000;
          if (now - Date.parse(exit.at) <= THROUGHPUT_WINDOW_MS) reviewWaits.push(Math.max(0, wait));
        }
      }
    }

    projects.push({
      slug: meta.slug,
      review,
      blocked,
      ready,
      lastActivity,
      problems: problems.length,
    });
  }

  inbox.sort((a, b) => Date.parse(a.waitingSince) - Date.parse(b.waitingSince));
  upNext.sort((a, b) => {
    const rank = (item: DashboardUpNextItem) => PRIORITY_RANK[item.task.priority] ?? 4;
    return (
      rank(a) - rank(b) ||
      a.task.order - b.task.order ||
      Date.parse(a.task.created) - Date.parse(b.task.created)
    );
  });
  upNext.splice(UP_NEXT_CAP);

  activity.sort((a, b) => b.at - a.at || b.seq - a.seq);
  const capped = activity.slice(0, ACTIVITY_CAP).map((entry) => entry.item);
  doneAt.sort();
  reviewWaits.sort((a, b) => a - b);

  return {
    generatedAt: new Date(now).toISOString(),
    inbox,
    upNext,
    readyTotal: upNextBeforeCap,
    activity: capped,
    doneAt,
    reviewWaits,
    projects,
  };
}
