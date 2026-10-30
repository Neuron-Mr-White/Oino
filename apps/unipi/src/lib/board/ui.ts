/**
 * Board UI state (UNI-130): view + display prefs, filters and query,
 * the active lane page, open sheets, the remembered PC/project, and
 * toasts. Module-level (like the web's state.ts) so it survives
 * leaving the Board tab; persisted bits live in localStorage.
 */
import { createRoot, createSignal } from 'solid-js';
import { createStore } from 'solid-js/store';
import type { Task } from './api';
import { DEFAULT_DISPLAY, EMPTY_FILTERS, type DisplayPrefs, type Filters, type Scope, type View } from './view';

const DISPLAY_KEY = 'unipi.board.display';
const VIEW_KEY = 'unipi.board.view';
const LAST_KEY = 'unipi.board.last';

function readDisplay(): DisplayPrefs {
  try {
    const parsed = JSON.parse(localStorage.getItem(DISPLAY_KEY) ?? 'null') as Partial<DisplayPrefs> | null;
    if (parsed && Array.isArray(parsed.hidden)) {
      return {
        hidden: parsed.hidden.filter((lane) => typeof lane === 'string'),
        excerpt: parsed.excerpt !== false,
        density: parsed.density === 'compact' ? 'compact' : 'comfortable',
        chains: parsed.chains !== false,
      };
    }
  } catch {
    /* corrupt prefs → defaults */
  }
  return { ...DEFAULT_DISPLAY, hidden: [...DEFAULT_DISPLAY.hidden] };
}

export type LastBoard = { hostId?: string; projects: Record<string, string> };
export function readLast(): LastBoard {
  try {
    const parsed = JSON.parse(localStorage.getItem(LAST_KEY) ?? 'null') as Partial<LastBoard> | null;
    if (parsed && typeof parsed === 'object') {
      return { hostId: typeof parsed.hostId === 'string' ? parsed.hostId : undefined, projects: { ...(parsed.projects ?? {}) } };
    }
  } catch {
    /* ignore */
  }
  return { projects: {} };
}
/** Remembers the PC and its project for the next visit. */
export function rememberLast(hostId: string, slug: string | null): void {
  const last = readLast();
  last.hostId = hostId;
  if (slug) last.projects[hostId] = slug;
  try {
    localStorage.setItem(LAST_KEY, JSON.stringify(last));
  } catch {
    /* storage full / disabled */
  }
}

export type NewTaskRequest = { lane: 'backlog' | 'todo'; recreateFrom?: Task };
export type ToastKind = 'info' | 'success' | 'warning' | 'error';
export type ToastAction = { label: string; run: () => void | Promise<void> };
export type Toast = { id: number; kind: ToastKind; message: string; action?: ToastAction; count?: number };

/** A move that needs (or invites) a note first — the web's CommentDialog. */
export type CommentRequest = {
  task: Task;
  to: string;
  /** What the note must say (the daemon's hint), e.g. "why the stale run is being released". */
  hint: string;
  /** Optional note (rework / unblock answer): Move works with an empty text too. */
  optional?: boolean;
  /** Runs after the move succeeded (a drag & drop applies its drop position here). */
  after?: (moved: Task) => void | Promise<void>;
};
/** A bulk / irreversible action that asks first (phone safeguard). */
export type ConfirmRequest = { title: string; detail?: string; confirm: string; dismiss?: string; danger?: boolean; run: () => void | Promise<void> };
/** The full-text reader (web ReportReader). */
export type ReaderEntry = { title: string; text: string; actor?: string; at?: string; mono?: boolean };
/** The full-screen image viewer: daemon file paths of the images to page through. */
export type ViewerItem = { path: string; name: string; size?: number };
export type ViewerRequest = { items: ViewerItem[]; index: number };

function createUi() {
  const [display, setDisplayStore] = createStore<DisplayPrefs>(readDisplay());
  const [filters, setFilters] = createStore<Filters>({ ...EMPTY_FILTERS, priorities: [], labels: [] });
  const [query, setQuery] = createSignal('');
  const [view, setViewSignal] = createSignal<View>(localStorage.getItem(VIEW_KEY) === 'list' ? 'list' : 'board');
  const [scope, setScope] = createSignal<Scope>('all');
  const [activeLane, setActiveLane] = createSignal<string>('todo');
  const [newTask, setNewTask] = createSignal<NewTaskRequest | null>(null);
  const [sheet, setSheet] = createSignal<'pc' | 'project' | 'filter' | 'display' | 'search' | null>(null);
  const [toasts, setToasts] = createSignal<Toast[]>([]);
  /** Task detail history: the last id is shown; Back pops (dep jumps push). */
  const [detail, setDetail] = createSignal<string[]>([]);
  const [cardMenu, setCardMenu] = createSignal<{ taskId: string; anchor: DOMRect } | null>(null);
  const [projectMenu, setProjectMenu] = createSignal<{ slug: string; anchor: DOMRect } | null>(null);
  const [comment, setComment] = createSignal<CommentRequest | null>(null);
  const [confirm, setConfirm] = createSignal<ConfirmRequest | null>(null);
  const [reader, setReader] = createSignal<ReaderEntry | null>(null);
  const [viewer, setViewer] = createSignal<ViewerRequest | null>(null);
  const [settingsOpen, setSettingsOpen] = createSignal(false);
  /** Keyboard selection (desktop J/K, web parity) and the shortcuts sheet. */
  const [selected, setSelected] = createSignal<string | null>(null);
  const [shortcutsOpen, setShortcutsOpen] = createSignal(false);
  /** The cross-project Overview (kanboard Dashboard) instead of the open project's lanes. */
  const [overview, setOverview] = createSignal(false);

  function setDisplay<K extends keyof DisplayPrefs>(key: K, value: DisplayPrefs[K]): void {
    setDisplayStore(key, value as never);
    try {
      localStorage.setItem(DISPLAY_KEY, JSON.stringify({ ...display, hidden: [...display.hidden] }));
    } catch {
      /* ignore */
    }
  }
  function toggleLane(laneId: string): void {
    setDisplay('hidden', display.hidden.includes(laneId) ? display.hidden.filter((lane) => lane !== laneId) : [...display.hidden, laneId]);
  }
  function setView(next: View, nextScope: Scope = 'all'): void {
    setViewSignal(next);
    setScope(nextScope);
    try {
      localStorage.setItem(VIEW_KEY, next);
    } catch {
      /* ignore */
    }
  }
  function clearFilters(): void {
    setFilters({ priorities: [], labels: [], running: false, waiting: false });
    setQuery('');
  }

  let toastId = 0;
  const timers = new Map<number, ReturnType<typeof setTimeout>>();
  function dismissToast(id: number): void {
    clearTimeout(timers.get(id));
    timers.delete(id);
    setToasts((current) => current.filter((item) => item.id !== id));
  }
  /** Web parity: identical toasts fold into ×N, at most 3 visible. */
  function toast(message: string, kind: ToastKind = 'info', action?: ToastAction): void {
    const ttl = action ? 8000 : kind === 'error' ? 9000 : 4500;
    const same = !action ? toasts().find((item) => !item.action && item.kind === kind && item.message === message) : undefined;
    if (same) {
      setToasts((current) => current.map((item) => (item.id === same.id ? { ...item, count: (item.count ?? 1) + 1 } : item)));
      clearTimeout(timers.get(same.id));
      timers.set(same.id, setTimeout(() => dismissToast(same.id), ttl));
      return;
    }
    const entry: Toast = { id: ++toastId, kind, message, action };
    setToasts((current) => [...current, entry].slice(-3));
    timers.set(entry.id, setTimeout(() => dismissToast(entry.id), ttl));
  }

  return {
    display,
    setDisplay,
    toggleLane,
    filters,
    setFilters,
    query,
    setQuery,
    clearFilters,
    view,
    setView,
    scope,
    setScope,
    activeLane,
    setActiveLane,
    newTask,
    /** Opens the New task sheet (lane = backlog|todo; recreateFrom prefills creation fields). */
    openNewTask: (request: NewTaskRequest) => setNewTask(request),
    closeNewTask: () => setNewTask(null),
    sheet,
    setSheet,
    toasts,
    toast,
    dismissToast,
    /** Ids of the open task detail pages (last = shown). */
    detail,
    openedTask: () => detail().at(-1),
    /** Opens a task's detail; from inside a detail (dep jump) it stacks so Back returns. */
    openTask: (id: string, stack = false) =>
      setDetail((current) => (stack && current.length > 0 ? [...current.filter((item) => item !== id), id] : [id])),
    /** Back from the detail: the previous task, or the board. */
    popTask: () => setDetail((current) => current.slice(0, -1)),
    closeTask: () => setDetail([]),
    cardMenu,
    setCardMenu,
    projectMenu,
    setProjectMenu,
    comment,
    setComment,
    confirm,
    setConfirm,
    reader,
    setReader,
    viewer,
    setViewer,
    settingsOpen,
    setSettingsOpen,
    selected,
    setSelected,
    shortcutsOpen,
    setShortcutsOpen,
    overview,
    setOverview,
  };
}

export type BoardUi = ReturnType<typeof createUi>;
let shared: BoardUi | undefined;
export function boardUi(): BoardUi {
  if (!shared) shared = createRoot(() => createUi());
  return shared;
}
/** Test hook. */
export function resetBoardUi(): void {
  shared = undefined;
}
