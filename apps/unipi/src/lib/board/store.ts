/**
 * Board store (UNI-130): the reactive data for one (PC, project) at a
 * time — projects + rules + global running list of the PC, tasks +
 * problems of the open project, loading/error flags.
 *
 * Built for the follow-up work (UNI-131: detail, actions, SSE):
 *  - `reload()` refetches just the open project's tasks (what an SSE
 *    `revision` should call, debounced), `refresh()` refetches
 *    everything (pull-to-refresh).
 *  - `applyTask(task)` upserts a server response; `removeTask(id)`;
 *    `optimistic(id, patch)` applies a local patch and returns a
 *    rollback for when the server refuses.
 *  - `target()` is the open (hostId, slug) — the live controller
 *    (live.ts, UNI-132) keys on it; `conn` / `setConn` hold the
 *    live-updates state for the header LED (written only by it).
 * Stale responses (PC/project switched mid-flight) are dropped.
 */
import { createRoot, createSignal, type Accessor } from 'solid-js';
import { createStore, produce, reconcile } from 'solid-js/store';
import {
  api,
  ApiError,
  PERMISSIVE_RULES,
  type Problem,
  type ProjectSummary,
  type Rules,
  type RunningItem,
  type Task,
} from './api';
import { setAttachmentBase } from './markdown';

export type ConnState = 'idle' | 'connecting' | 'live' | 'reconnecting';
export type BoardTarget = { hostId: string; slug: string | null };

export type BoardState = {
  projects: ProjectSummary[];
  projectsLoaded: boolean;
  projectsLoading: boolean;
  projectsError: ApiError | undefined;
  rules: Rules;
  running: RunningItem[];
  tasks: Task[];
  problems: Problem[];
  /** A tasks request is in flight. */
  loading: boolean;
  /** Tasks of the open project arrived at least once. */
  loaded: boolean;
  error: ApiError | undefined;
  /** Epoch ms of the last successful tasks load. */
  updatedAt: number | undefined;
};

export type BoardStore = {
  state: BoardState;
  target: Accessor<BoardTarget | undefined>;
  conn: Accessor<ConnState>;
  setConn: (state: ConnState) => void;
  /** Opens a project (null = only the PC's project list). Resets tasks when the target changes. */
  open: (hostId: string, slug: string | null) => Promise<void>;
  /** Refetches projects, rules, running and tasks (pull-to-refresh). */
  refresh: () => Promise<void>;
  /** Refetches only the open project's tasks (SSE revision handler). */
  reload: () => Promise<void>;
  loadProjects: () => Promise<void>;
  /** Upserts one task from a server response. */
  applyTask: (task: Task) => void;
  removeTask: (id: string) => void;
  /** Applies a local patch now; the returned function restores the original. */
  optimistic: (id: string, patch: Partial<Task>) => () => void;
  taskById: (id: string | null | undefined) => Task | undefined;
  currentProject: () => ProjectSummary | undefined;
};

function initialState(): BoardState {
  return {
    projects: [],
    projectsLoaded: false,
    projectsLoading: false,
    projectsError: undefined,
    rules: { ...PERMISSIVE_RULES, statuses: [...PERMISSIVE_RULES.statuses] },
    running: [],
    tasks: [],
    problems: [],
    loading: false,
    loaded: false,
    error: undefined,
    updatedAt: undefined,
  };
}

const asApiError = (error: unknown): ApiError =>
  error instanceof ApiError ? error : new ApiError(error instanceof Error ? error.message : String(error), 0);

export function createBoardStore(): BoardStore {
  const [state, setState] = createStore<BoardState>(initialState());
  const [target, setTarget] = createSignal<BoardTarget | undefined>(undefined);
  const [conn, setConn] = createSignal<ConnState>('idle');
  let tasksSeq = 0;
  let projectsSeq = 0;

  const same = (a: BoardTarget | undefined, b: BoardTarget | undefined) =>
    !!a && !!b && a.hostId === b.hostId && a.slug === b.slug;

  async function loadProjects(): Promise<void> {
    const at = target();
    if (!at) return;
    const seq = ++projectsSeq;
    setState('projectsLoading', true);
    try {
      const [projects, running] = await Promise.all([
        api.projects(at.hostId),
        api.running(at.hostId).then((r) => r.running).catch(() => [] as RunningItem[]),
      ]);
      if (seq !== projectsSeq || target()?.hostId !== at.hostId) return;
      setState('projects', reconcile(projects, { key: 'slug' }));
      setState('running', running);
      setState('projectsError', undefined);
    } catch (error) {
      if (seq !== projectsSeq || target()?.hostId !== at.hostId) return;
      setState('projectsError', asApiError(error));
    } finally {
      if (seq === projectsSeq) setState({ projectsLoading: false, projectsLoaded: true });
    }
  }

  async function loadRules(): Promise<void> {
    const at = target();
    if (!at) return;
    try {
      const rules = await api.rules(at.hostId);
      if (target()?.hostId === at.hostId) setState('rules', rules);
    } catch {
      // Older daemon: per-task allowedMoves still work; the server refuses the rest.
      if (target()?.hostId === at.hostId) setState('rules', { ...PERMISSIVE_RULES, statuses: [...PERMISSIVE_RULES.statuses] });
    }
  }

  async function reload(): Promise<void> {
    const at = target();
    if (!at?.slug) return;
    const seq = ++tasksSeq;
    setState('loading', true);
    try {
      const payload = await api.tasks(at.hostId, at.slug);
      if (seq !== tasksSeq || !same(target(), at)) return;
      setState('tasks', reconcile(payload.tasks ?? [], { key: 'id' }));
      setState({ problems: payload.problems ?? [], loaded: true, error: undefined, updatedAt: Date.now() });
    } catch (error) {
      if (seq !== tasksSeq || !same(target(), at)) return;
      setState('error', asApiError(error));
    } finally {
      if (seq === tasksSeq) setState('loading', false);
    }
  }

  async function open(hostId: string, slug: string | null): Promise<void> {
    const previous = target();
    const next = { hostId, slug };
    if (same(previous, next) && (state.loaded || state.loading)) return;
    const hostChanged = previous?.hostId !== hostId;
    tasksSeq += 1; // drop in-flight responses for the old target
    setTarget(next);
    setAttachmentBase(slug);
    // conn is owned by the live controller (live.ts), which re-subscribes on the target change.
    setState({ tasks: [], problems: [], loaded: false, loading: false, error: undefined, updatedAt: undefined });
    if (hostChanged) {
      projectsSeq += 1;
      setState({
        projects: [],
        projectsLoaded: false,
        projectsError: undefined,
        running: [],
        rules: { ...PERMISSIVE_RULES, statuses: [...PERMISSIVE_RULES.statuses] },
      });
      await Promise.all([loadProjects(), loadRules(), reload()]);
    } else {
      await reload();
    }
  }

  async function refresh(): Promise<void> {
    await Promise.all([loadProjects(), loadRules(), reload()]);
  }

  function applyTask(task: Task): void {
    const index = state.tasks.findIndex((candidate) => candidate.id === task.id);
    if (index === -1) setState('tasks', (current) => [...current, task]);
    else setState('tasks', index, reconcile(task));
  }

  function removeTask(id: string): void {
    setState('tasks', (current) => current.filter((task) => task.id !== id));
  }

  function optimistic(id: string, patch: Partial<Task>): () => void {
    const index = state.tasks.findIndex((task) => task.id === id);
    if (index === -1) return () => {};
    // Snapshot first: store proxies reflect later writes.
    const original = JSON.parse(JSON.stringify(state.tasks[index])) as Task;
    setState(
      'tasks',
      index,
      produce((task) => Object.assign(task, patch)),
    );
    return () => applyTask(original);
  }

  return {
    state,
    target,
    conn,
    setConn,
    open,
    refresh,
    reload,
    loadProjects,
    applyTask,
    removeTask,
    optimistic,
    taskById: (id) => (id ? state.tasks.find((task) => task.id === id) : undefined),
    currentProject: () => state.projects.find((project) => project.slug === target()?.slug),
  };
}

let shared: BoardStore | undefined;
/** The app-wide board store (survives leaving the Board tab). */
export function boardStore(): BoardStore {
  if (!shared) shared = createRoot(() => createBoardStore());
  return shared;
}
/** Test hook. */
export function resetBoardStore(): void {
  shared = undefined;
}
