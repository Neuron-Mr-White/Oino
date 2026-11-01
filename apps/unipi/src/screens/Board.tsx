/**
 * Board (DESIGN §3.9, UNI-130): the kanboard web UI's board on a phone.
 *
 * Header: PC chip (all paired PCs + board availability) above the
 * project title (project picker), search + ⋯ on the right. Toolbar:
 * Board/List, Filter, Display, active filter chips. Board view: lane
 * strip + swipeable lane pages; List view: grouped rows. A FAB creates
 * a task (lane-aware). Card tap / long-press, lane bulk actions and
 * project actions go through lib/board/actions.ts; UNI-131 renders the
 * task detail page, the card menu and the comment / confirm / summary /
 * settings sheets, the report reader and the image viewer here.
 */
import { createEffect, createMemo, createSignal, on, onCleanup, Show, untrack } from 'solid-js';
import {
  Archive,
  ArchiveRestore,
  ArrowLeft,
  ChevronDown,
  Columns3,
  EllipsisVertical,
  Eye,
  Filter,
  FolderPlus,
  Keyboard,
  LayoutDashboard,
  List,
  MessagesSquare,
  OctagonX,
  Plus,
  RefreshCw,
  Search,
  SlidersHorizontal,
  TriangleAlert,
  X,
} from 'lucide-solid';
import { useHosts } from '../hosts/context';
import { useSettings } from '../settings/context';
import { EmptyState, NoHostState, OfflineState } from '../components/EmptyState';
import { PageHeader } from '../components/PageHeader';
import { PcChip } from '../components/PcChip';
import { setAddHostOpen } from '../components/Pairing';
import { hapticTick, type HapticKind } from '../lib/haptics';
import { canMove, laneLabel, type RunningItem } from '../lib/board/api';
import { boardDrag } from '../lib/board/dnd';
import { boardStore, type ConnState } from '../lib/board/store';
import { useBoardLive } from '../lib/board/live';
import { boardUi, readLast, rememberLast } from '../lib/board/ui';
import { clearBoardJump, clearChatPick, pendingBoardJump, pendingChatPick, pickProjectForCwd, pickProjectForRef, requestChatPick, type BoardJump } from '../lib/board/portal';
import { clearReturnToChat, hasMarkedReturnToChat, openChat, openNewChat, resumeChatAfterTerminal } from '../lib/session/open';
import { pushBack } from '../lib/back';
import { queueMention } from '../lib/chat/mention';
import { composerChips } from '../lib/composer/drafts';
import type { ChatTarget } from '../lib/session/list';
import { ChatPickSheet } from '../components/board/ChatPickSheet';
import { boardActions, registerHighlighter } from '../lib/board/actions';
import { availabilityText, boardPcs, pcAvailability } from '../lib/board/pcs';
import { filterCount, hue, matches, visibleLanes } from '../lib/board/view';
import { LanePager, LaneStrip, CardSkeletons } from '../components/board/BoardLanes';
import { BoardList } from '../components/board/BoardList';
import { FilterSheet, DisplaySheet } from '../components/board/FilterDisplaySheets';
import { AddProjectSheet } from '../components/board/AddProjectSheet';
import { PcSheet, ProjectMenu, ProjectSheet } from '../components/board/PickerSheets';
import { SearchSheet } from '../components/board/SearchSheet';
import { NewTaskSheet } from '../components/board/NewTaskSheet';
import { AnchoredMenu, MenuLabel, MenuRow, MenuSeparator } from '../components/board/AnchoredMenu';
import { ProjectTile } from '../components/board/Glyphs';
import { TaskDetail } from '../components/board/TaskDetail';
import { CardMenu } from '../components/board/TaskMenus';
import { CommentSheet, ConfirmSheet } from '../components/board/MoveSheets';
import { ImageViewer, ReportReader } from '../components/board/Viewers';
import { registerActionHaptic } from '../lib/board/taskActions';
import { Dashboard } from '../components/board/Dashboard';
import { BoardKeys, BoardShortcutsSheet } from '../components/board/BoardKeys';
import { desktopShell } from '../lib/layout';
import { can } from '../lib/platform';
import './board.css';

/** The project to open on a PC: remembered → settings default → most recently active → first. */
export function pickProject(
  projects: { slug: string; name: string; archived?: boolean; updatedAt?: string | null }[],
  remembered: string | undefined,
  defaultProject: string | null,
): string | undefined {
  if (remembered && projects.some((project) => project.slug === remembered)) return remembered;
  if (defaultProject) {
    const wanted = defaultProject.trim().toLowerCase();
    const hit = projects.find((project) => project.slug.toLowerCase() === wanted || project.name.toLowerCase() === wanted);
    if (hit) return hit.slug;
  }
  const active = projects.filter((project) => !project.archived);
  const pool = active.length > 0 ? active : projects;
  const newest = [...pool].sort((a, b) => Date.parse(b.updatedAt ?? '') - Date.parse(a.updatedAt ?? '') || 0);
  return (newest.find((project) => project.updatedAt) ?? pool[0])?.slug;
}

export default function Board() {
  const hosts = useHosts();
  const [settings, update] = useSettings();
  const store = boardStore();
  const ui = boardUi();
  const avail = pcAvailability();
  const pcs = createMemo(() => boardPcs(hosts.hosts()));
  const [hostId, setHostId] = createSignal<string | undefined>(store.target()?.hostId);
  const [refreshing, setRefreshing] = createSignal(false);
  const [menu, setMenu] = createSignal<DOMRect | undefined>();
  const [addProjectOpen, setAddProjectOpen] = createSignal(false);
  let pagerApi: { scrollToLane: (lane: string, smooth?: boolean) => void; pageFor: (lane: string) => HTMLElement | undefined } | undefined;

  const haptic = (kind: HapticKind = 'tap') => void hapticTick(settings.input.haptics, kind);
  const pc = () => pcs().find((candidate) => candidate.id === hostId());
  const availability = () => (hostId() ? avail.availability(hostId()!) : undefined);
  const slug = () => (store.target()?.hostId === hostId() ? store.target()?.slug : undefined);
  const project = () => store.currentProject();
  const drag = boardDrag();
  // A desktop drag shows Backlog while the card may move there (F25).
  const dragToBacklog = createMemo(() => {
    const current = drag.drag();
    return !!current && desktopShell() && current.task.status !== 'backlog' && canMove(store.state.rules, current.task, 'backlog');
  });
  // Backlog: hidden by default, but it shows whenever it has cards (a hidden column must never hold work).
  const backlogCount = () => store.state.tasks.filter((task) => task.status === 'backlog').length;
  const backlogShown = () => settings.board.showBacklog || dragToBacklog() || backlogCount() > 0;
  const lanes = createMemo(() => visibleLanes(ui.display, backlogShown(), ui.scope()));

  // ── PC + project selection ──────────────────────────────────────────
  createEffect(() => {
    const list = pcs();
    const current = untrack(hostId);
    if (current && list.some((candidate) => candidate.id === current)) return;
    const last = readLast().hostId;
    setHostId(list.find((candidate) => candidate.id === last)?.id ?? list[0]?.id);
  });
  createEffect(on(hostId, (id) => id && void avail.check(id)));
  createEffect(
    on([hostId, () => availability()?.state], ([id, state]) => {
      if (!id || state !== 'ok') return;
      const at = untrack(store.target);
      if (at?.hostId === id && at.slug) return;
      void openHost(id);
    }),
  );
  // Live updates (UNI-132): subscribed while this tab shows a project and
  // the page is visible. Coming back to the tab (or the page) resubscribes;
  // the daemon's connect revision is the one catch-up reload.
  useBoardLive(store, () => {
    const at = store.target();
    const id = hostId();
    if (!id || availability()?.state !== 'ok') return null;
    return { hostId: id, slug: at?.hostId === id ? (at.slug ?? null) : null };
  });

  async function openHost(id: string) {
    if (store.target()?.hostId !== id) await store.open(id, null);
    else if (!store.state.projectsLoaded) await store.loadProjects();
    // A chat → board jump (UNI-181) picks the project itself.
    if (hostId() !== id || jumping) return;
    const chosen = pickProject(store.state.projects, readLast().projects[id], settings.board.defaultProject);
    rememberLast(id, chosen ?? null);
    if (chosen) await store.open(id, chosen);
  }
  function chooseProject(next: string) {
    const id = hostId();
    ui.setSheet(null);
    ui.setOverview(false);
    if (!id) return;
    haptic();
    rememberLast(id, next);
    if (next !== slug()) {
      ui.setScope('all');
      void store.open(id, next);
    }
  }
  function choosePc(id: string) {
    ui.setSheet(null);
    haptic();
    if (id === hostId()) return;
    setHostId(id);
    rememberLast(id, readLast().projects[id] ?? null);
  }
  /** Overview / agents list → open a task in its project (waits for that board). */
  async function openTaskIn(slug: string, id: string) {
    ui.setOverview(false);
    chooseProject(slug);
    const started = Date.now();
    while (Date.now() - started < 8000 && !(store.target()?.slug === slug && store.state.loaded)) {
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    const task = store.taskById(id);
    if (!task) return ui.toast(`${id} isn't on this board any more`, 'info');
    highlight(task.id, true);
    boardActions().openTask(task);
  }
  async function openAgent(item: RunningItem) {
    chooseProject(item.slug);
    // Wait for the board to arrive, then jump.
    const started = Date.now();
    while (Date.now() - started < 8000 && !(store.target()?.slug === item.slug && store.state.loaded)) {
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    const task = store.taskById(item.task.id) ?? item.task;
    highlight(task.id);
    boardActions().openTask(task);
  }

  // ── Chat ↔ board portal (UNI-181) ───────────────────────────────────
  // A chat asked for a task pane (tapped `UNI-172`) or the New task sheet:
  // pick the PC + project, wait for the board, open it — and when that
  // pane/sheet closes, go straight back to the chat.
  let portal: { task?: string; newTask?: boolean } | null = null;
  let jumping = false;
  async function waitProjects(id: string): Promise<void> {
    const started = Date.now();
    while (
      Date.now() - started < 8000 &&
      !(store.target()?.hostId === id && store.state.projectsLoaded && !store.state.projectsLoading)
    ) {
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
  }
  async function waitLoaded(target: string): Promise<boolean> {
    const started = Date.now();
    while (Date.now() - started < 8000 && !(store.target()?.slug === target && store.state.loaded)) {
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    return store.target()?.slug === target && store.state.loaded;
  }
  function backToChat() {
    portal = null;
    resumeChatAfterTerminal();
  }
  async function runJump(jump: BoardJump) {
    jumping = true;
    try {
      await jumpTo(jump);
    } finally {
      jumping = false;
    }
  }
  async function jumpTo(jump: BoardJump) {
    const id = jump.hostId;
    if (store.target()?.hostId !== id) await store.open(id, null);
    else if (!store.state.projectsLoaded) await store.loadProjects();
    // The Board's own mount-time load may supersede ours: wait for the list to settle.
    await waitProjects(id);
    const last = readLast().projects[id];
    const chosen = jump.taskId
      ? pickProjectForRef(store.state.projects, jump.taskId, jump.cwd, last)
      : (pickProjectForCwd(store.state.projects, jump.cwd, last) ?? store.target()?.slug ?? undefined);
    if (!chosen) {
      ui.toast(jump.taskId ? `No board project has ${jump.taskId}` : 'Choose a project first', 'info');
      if (jump.fromChat && !jump.newTask) backToChat();
      return;
    }
    // Remember first, so the Board's own host/project effects pick the same one.
    rememberLast(id, chosen);
    if (hostId() !== id) setHostId(id);
    if (store.target()?.slug !== chosen) await store.open(id, chosen);
    if (!(await waitLoaded(chosen))) {
      ui.toast("The board didn't load", 'error');
      return;
    }
    if (jump.taskId) {
      const task = store.taskById(jump.taskId);
      if (!task) {
        ui.toast(`${jump.taskId} isn't on this board`, 'info');
        if (jump.fromChat) backToChat();
        return;
      }
      if (jump.fromChat) portal = { task: task.id };
      boardActions().openTask(task);
    } else {
      if (jump.fromChat) portal = { newTask: true };
      ui.openNewTask({ lane: 'todo' });
    }
  }
  // ── board → chat: ask which chat (UNI-185) ──────────────────────────
  const pickToChat = (target: ChatTarget) => {
    const pick = pendingChatPick();
    clearChatPick();
    // Leaving for a chat the user chose: drop any armed return-to-chat.
    portal = null;
    clearReturnToChat();
    if (pick?.chip) composerChips.add(`${target.hostId}/${target.session}`, pick.chip);
    else if (pick?.mention) queueMention(`${target.hostId}:${target.session}`, pick.mention);
    // The task pane is a portal above the chat layer: close it so the chat shows.
    if (ui.openedTask()) ui.closeTask();
    openChat(target);
  };
  const pickNewChat = (id: string, cwd?: string) => {
    const pick = pendingChatPick();
    clearChatPick();
    portal = null;
    clearReturnToChat();
    if (pick?.chip) composerChips.add(`draft:${id}`, pick.chip);
    else if (pick?.mention) queueMention(`draft:${id}`, pick.mention);
    if (ui.openedTask()) ui.closeTask();
    openNewChat(id, cwd);
  };
  createEffect(() => {
    if (!hasMarkedReturnToChat() || ui.openedTask() || ui.newTask() || ui.sheet() || pendingChatPick()) return;
    const off = pushBack(backToChat);
    onCleanup(off);
  });
  createEffect(
    on(pendingBoardJump, (jump) => {
      if (!jump) return;
      clearBoardJump();
      void runJump(jump);
    }),
  );
  createEffect(
    on(
      () => ui.openedTask(),
      (now, prev) => {
        if (portal?.task && prev === portal.task && now === undefined) backToChat();
      },
    ),
  );
  createEffect(
    on(
      () => ui.newTask(),
      (now, prev) => {
        if (portal?.newTask && prev && !now) backToChat();
      },
    ),
  );
  onCleanup(() => {
    if (portal) {
      portal = null;
      clearReturnToChat();
    }
  });

  async function refresh() {
    if (refreshing()) return;
    setRefreshing(true);
    try {
      const id = hostId();
      if (id && availability()?.state !== 'ok') {
        await avail.check(id);
      } else await store.refresh();
    } finally {
      setRefreshing(false);
    }
  }

  // ── lanes ───────────────────────────────────────────────────────────
  createEffect(() => {
    const visible = lanes();
    if (visible.length === 0) return;
    const active = untrack(ui.activeLane);
    if (!visible.some((lane) => lane.id === active)) ui.setActiveLane((visible.find((lane) => lane.id === 'todo') ?? visible[0]!).id);
  });
  function pickLane(lane: string) {
    if (lane !== ui.activeLane()) haptic('tick');
    ui.setActiveLane(lane);
    pagerApi?.scrollToLane(lane);
  }
  function hideLane(lane: string) {
    if (lane === 'backlog') update('board.showBacklog', false);
    else ui.toggleLane(lane);
    ui.toast(
      lane === 'backlog' && backlogCount() > 0 ? 'Backlog stays visible while it has cards' : `${laneLabel(lane)} hidden — turn it back on in Display`,
      'info',
    );
  }

  // ── highlight (dep ids, search results, agents) ─────────────────────
  function highlight(id: string, quiet = false) {
    const task = store.taskById(id);
    if (!task) return quiet || ui.toast(`${id} is not on this board`, 'info');
    if (!lanes().some((lane) => lane.id === task.status))
      return quiet || ui.toast(`${id} is in ${laneLabel(task.status)}, which is hidden`, 'info');
    if (!matches(task, ui.query(), ui.filters)) return quiet || ui.toast(`${id} is hidden by the current filter`, 'info');
    let container: HTMLElement | null | undefined;
    if (ui.view() === 'board') {
      ui.setActiveLane(task.status);
      pagerApi?.scrollToLane(task.status);
      container = pagerApi?.pageFor(task.status);
    } else container = document.querySelector<HTMLElement>('.board-list');
    setTimeout(() => {
      const element = container?.querySelector<HTMLElement>(`[data-id="${id}"]`);
      if (!element || !container) return;
      const top = element.getBoundingClientRect().top - container.getBoundingClientRect().top + container.scrollTop;
      container.scrollTo?.({ top: Math.max(0, top - container.clientHeight / 2 + element.offsetHeight / 2), behavior: 'smooth' });
      element.classList.add('flash');
      setTimeout(() => element.classList.remove('flash'), 1600);
    }, 320);
  }
  registerHighlighter(highlight);
  registerActionHaptic(haptic);
  onCleanup(() => {
    registerHighlighter(undefined);
    registerActionHaptic(undefined);
  });

  // ── derived header bits ─────────────────────────────────────────────
  const inLanes = () => {
    const visible = new Set(lanes().map((lane) => lane.id));
    return store.state.tasks.filter((task) => visible.has(task.status));
  };
  const shown = () => inLanes().filter((task) => matches(task, ui.query(), ui.filters)).length;
  /** Filter / text query narrow the cards (the scope is shown as its own chip). */
  const narrowed = () => filterCount(ui.filters) > 0 || ui.query().trim().length > 0;
  const runningCount = () => store.state.tasks.filter((task) => !!task.run).length;
  const count = (lane: string) => store.state.tasks.filter((task) => task.status === lane).length;
  const filtersOn = () => filterCount(ui.filters);
  const anyFilter = () => filtersOn() > 0 || ui.query().trim().length > 0 || ui.scope() !== 'all';
  // New tasks never land in a hidden column: Todo while Backlog is hidden.
  const fabLane = () => (desktopShell() || !backlogShown() || (ui.view() === 'board' && ui.activeLane() === 'todo') ? 'todo' : 'backlog');
  const unavailable = () => {
    const state = availability()?.state;
    return state === 'offline' || state === 'not_installed' || state === 'error';
  };
  const runMenu = (fn: () => void) => () => {
    setMenu(undefined);
    fn();
  };

  return (
    <Show
      when={pcs().length > 0}
      fallback={
        <div class="page-screen">
          <PageHeader title="Board" />
          <NoHostState what="its kanboard projects" onAdd={() => setAddHostOpen(true)} />
        </div>
      }
    >
      <div class="board-screen" data-view={ui.view()}>
        <PageHeader
          class="board-head"
          title="Board"
          lead={
            <PcChip
              name={pc()?.name}
              label={`PC: ${pc()?.name ?? 'none'}${store.conn() === 'live' ? ' · live' : ''}`}
              state={availability()?.state ?? 'checking'}
              live={slug() ? store.conn() : 'idle'}
              onClick={() => ui.setSheet('pc')}
            />
          }
          tools={
            <>
            <Show when={hasMarkedReturnToChat() && can('chat')}>
              <button
                type="button"
                class="ghost-icon-button"
                aria-label="Back to chat"
                title="Back to chat"
                data-testid="board-back-to-chat"
                onClick={backToChat}
              >
                <ArrowLeft size={20} />
              </button>
            </Show>
            <Show when={hostId() && can('chat')}>
              <button type="button" class="ghost-icon-button" aria-label="Go to a chat" onClick={() => requestChatPick({ hostId: hostId()! })}>
                <MessagesSquare size={20} />
              </button>
            </Show>
            <button type="button" class="ghost-icon-button" aria-label="Search" disabled={!slug()} onClick={() => ui.setSheet('search')}>
              <Search size={20} />
            </button>
            <button
              type="button"
              class="ghost-icon-button"
              aria-label="Board menu"
              aria-expanded={menu() !== undefined}
              onClick={(event) => setMenu(event.currentTarget.getBoundingClientRect())}
            >
              <EllipsisVertical size={20} />
            </button>
            </>
          }
          titleSlot={
            <button
              type="button"
              class="project-title"
              aria-label={`Project: ${project()?.name ?? 'none'}`}
              disabled={availability()?.state !== 'ok'}
              onClick={() => ui.setSheet('project')}
            >
              <Show
                when={!ui.overview()}
                fallback={
                  <span class="overview-tile" aria-hidden="true">
                    <LayoutDashboard size={18} />
                  </span>
                }
              >
                <Show when={project()}>
                  {(current) => <ProjectTile name={current().name} hue={hue(current().name)} size={30} running={(current().running ?? 0) > 0} />}
                </Show>
              </Show>
              <span class="large-title truncate">
                {ui.overview() ? 'Overview' : (project()?.name ?? (store.state.projectsLoaded && store.state.projects.length > 0 ? 'Choose a project' : 'Board'))}
              </span>
              <Show when={availability()?.state === 'ok' && store.state.projects.length > 0}>
                <ChevronDown size={22} class="project-chevron" />
              </Show>
            </button>
          }
          meta={
            // No meta line while the PC can't serve a board (offline / not installed): an empty
            // line pushed the shared OfflineState 20 px below the other screens' (UNI-218 F60).
            unavailable() ? undefined :
            <span class="board-meta">
              <Show when={!ui.overview()} fallback={<span>Every project on {pc()?.name ?? 'this PC'}</span>}>
              <Show when={slug() && store.state.loaded} fallback={<span>{availability()?.state === 'checking' ? 'Connecting…' : '\u00a0'}</span>}>
                <span class="tabular-nums">
                  {narrowed() ? `${shown()} of ${inLanes().length} tasks` : `${shown()} ${shown() === 1 ? 'task' : 'tasks'}`}
                </span>
                <Show when={runningCount() > 0}>
                  <span class="board-meta-running">
                    <span class="pulse" /> {runningCount()} running
                  </span>
                </Show>
                <Show when={project()?.archived}>
                  <span class="board-meta-archived">Archived</span>
                </Show>
              </Show>
              </Show>
            </span>
          }
        />

        <Show when={slug() && !ui.overview()}>
          <div class="board-toolbar" role="toolbar" aria-label="Board tools">
            <div class="view-toggle" role="radiogroup" aria-label="View">
              <button type="button" role="radio" aria-checked={ui.view() === 'board'} aria-label="Board" onClick={() => ui.setView('board')}>
                <Columns3 size={17} />
              </button>
              <button type="button" role="radio" aria-checked={ui.view() === 'list'} aria-label="List" onClick={() => ui.setView('list')}>
                <List size={17} />
              </button>
            </div>
            <Show when={ui.scope() !== 'all'}>
              <button type="button" class="tool-chip" data-on="" aria-label="Clear scope" onClick={() => ui.setScope('all')}>
                {ui.scope() === 'in_review' ? 'Review queue' : 'Blocked'} <X size={14} />
              </button>
            </Show>
            <Show when={ui.query().trim()}>
              <button type="button" class="tool-chip" data-on="" aria-label="Clear search filter" onClick={() => ui.setQuery('')}>
                <Search size={14} /> <span class="max-w-[120px] truncate">{ui.query().trim()}</span> <X size={14} />
              </button>
            </Show>
            <button type="button" class="tool-chip" data-on={filtersOn() > 0 ? '' : undefined} onClick={() => ui.setSheet('filter')}>
              <Filter size={15} /> Filter
              <Show when={filtersOn() > 0}>
                <span class="tool-badge">{filtersOn()}</span>
              </Show>
            </button>
            <button type="button" class="tool-chip" onClick={() => ui.setSheet('display')}>
              <SlidersHorizontal size={15} /> Display
            </button>
            <Show when={anyFilter()}>
              <button
                type="button"
                class="tool-chip tool-clear"
                onClick={() => {
                  ui.clearFilters();
                  ui.setScope('all');
                }}
              >
                Clear
              </button>
            </Show>
            <Show when={desktopShell()}>
              <span class="flex-1" />
              <button
                type="button"
                class="btn btn-primary btn-sm board-new-task"
                aria-label={`New task in ${laneLabel(fabLane())}`}
                disabled={!store.state.loaded}
                onClick={() => ui.openNewTask({ lane: fabLane() })}
              >
                <Plus size={16} strokeWidth={2.4} /> New task
              </button>
            </Show>
          </div>
        </Show>

        <div class="board-banners">
          <Show when={!unavailable() && store.state.error && store.state.loaded}>
            <BoardBanner title="Showing the last loaded board" detail={store.state.error!.message} onRetry={() => void store.reload()} />
          </Show>
          <Show when={store.state.problems.length > 0}>
            <aside class="board-banner warn" role="alert">
              <TriangleAlert size={18} class="shrink-0 text-amber" />
              <div class="min-w-0 flex-1">
                <p class="font-medium">
                  {store.state.problems.length} task file{store.state.problems.length === 1 ? '' : 's'} can't be read
                </p>
                <p class="text-text-muted">The rest of the board keeps working.</p>
                <ul class="mt-1 text-[12px] text-text-dim font-mono">
                  {store.state.problems.slice(0, 3).map((problem) => (
                    <li class="truncate">
                      {problem.file.split('/').pop()}:{problem.line} {problem.error}
                    </li>
                  ))}
                </ul>
                <p class="mt-1 text-[12px] text-text-dim">
                  Repair with <code class="font-mono text-gold">unipi-kanboard validate --fix</code>
                </p>
              </div>
            </aside>
          </Show>
        </div>

        <Show when={unavailable() && availability()?.state === 'offline'}>
          <OfflineState
            pc={pc()?.name}
            onRetry={() => hostId() && void avail.check(hostId()!)}
            secondary={pcs().length > 1 ? { label: 'Switch PC', run: () => ui.setSheet('pc') } : undefined}
          />
        </Show>
        <Show when={unavailable() && availability()?.state !== 'offline'}>
          <BoardState
            tone="error"
            title={
              availability()?.state === 'not_installed'
                ? `The board isn't set up on ${pc()?.name ?? 'this PC'}`
                : 'The board is unavailable'
            }
            detail={availabilityText(availability())}
            onRetry={() => hostId() && void avail.check(hostId()!)}
            secondary={pcs().length > 1 ? { label: 'Switch PC', run: () => ui.setSheet('pc') } : undefined}
          />
        </Show>
        <Show when={!unavailable() && store.state.projectsError && !store.state.projects.length}>
          <BoardState tone="error" title="Couldn't load projects" detail={store.state.projectsError!.message} onRetry={() => void refresh()} />
        </Show>
        <Show when={!unavailable() && slug() && store.state.error && !store.state.loaded}>
          <BoardState tone="error" title="Couldn't load the board" detail={store.state.error!.message} onRetry={() => void store.reload()} />
        </Show>
        <Show when={ui.overview() && !unavailable() && availability()?.state === 'ok' && hostId()}>
          <div class="dash-scroll">
            <Dashboard hostId={hostId()!} onOpenTask={(target, id) => void openTaskIn(target, id)} onOpenProject={chooseProject} />
          </div>
        </Show>
        <Show when={!ui.overview() && !unavailable() && !store.state.projectsError && !(store.state.error && !store.state.loaded)}>
          <Show
            when={slug()}
            fallback={
              <Show
                when={store.state.projectsLoaded && store.state.projects.length === 0 && !store.state.projectsError}
                fallback={
                  <Show when={availability()?.state !== 'ok' || !store.state.projectsLoaded}>
                    <div class="board-loading">
                      <div class="lane-strip">
                        <span class="lane-pill skeleton w-24" />
                        <span class="lane-pill skeleton w-28" />
                        <span class="lane-pill skeleton w-20" />
                      </div>
                      <div class="lane-cards px-4">
                        <CardSkeletons />
                      </div>
                    </div>
                  </Show>
                }
              >
                <BoardState
                  title={`No projects on ${pc()?.name ?? 'this PC'}`}
                  detail="Add a repo's folder to start a board for it."
                  onRetry={() => setAddProjectOpen(true)}
                  retryLabel="Add project"
                  secondary={{ label: 'Check again', run: () => void refresh() }}
                />
              </Show>
            }
          >
            <Show
              when={ui.view() === 'board'}
              fallback={<BoardList lanes={lanes()} onRefresh={() => void refresh()} refreshing={refreshing()} onArmed={() => haptic('tick')} />}
            >
              <Show
                when={lanes().length > 0}
                fallback={
                  <div class="list-none">
                    <span>Every column is hidden.</span>
                    <button type="button" class="lane-empty-clear" onClick={() => ui.setSheet('display')}>
                      Display options
                    </button>
                  </div>
                }
              >
                <LaneStrip lanes={lanes()} active={ui.activeLane()} onPick={pickLane} loading={store.state.loading} />
                <LanePager
                  lanes={lanes()}
                  active={ui.activeLane()}
                  onActiveChange={(lane) => ui.setActiveLane(lane)}
                  onRefresh={() => void refresh()}
                  refreshing={refreshing()}
                  onHaptic={haptic}
                  onHideLane={hideLane}
                  ref={(api) => (pagerApi = api)}
                />
              </Show>
            </Show>
          </Show>
        </Show>

        <Show when={slug() && store.state.loaded && !ui.overview() && !desktopShell()}>
          <button
            type="button"
            class="board-fab"
            aria-label={`New task in ${laneLabel(fabLane())}`}
            onClick={() => {
              haptic();
              ui.openNewTask({ lane: fabLane() });
            }}
          >
            <Plus size={26} strokeWidth={2.2} />
          </button>
        </Show>
      </div>

      <Show when={menu()}>
        {(anchor) => (
          <AnchoredMenu anchor={anchor()} alignEnd width={256} label="Board menu" onDismiss={() => setMenu(undefined)}>
            <MenuRow icon={<RefreshCw size={18} />} label="Refresh" onSelect={runMenu(() => void refresh())} />
            <Show when={availability()?.state === 'ok' && hostId()}>
              <MenuRow icon={<FolderPlus size={18} />} label="Add project…" onSelect={runMenu(() => setAddProjectOpen(true))} />
            </Show>
            <Show when={availability()?.state === 'ok'}>
              <MenuRow
                icon={<LayoutDashboard size={18} />}
                label={ui.overview() ? 'Back to the board' : 'Overview of all projects'}
                onSelect={runMenu(() => ui.setOverview(!ui.overview()))}
              />
            </Show>
            <Show when={desktopShell()}>
              <MenuRow icon={<Keyboard size={18} />} label="Keyboard shortcuts" hint="?" onSelect={runMenu(() => ui.setShortcutsOpen(true))} />
            </Show>
            <Show when={slug()}>
              <MenuRow
                icon={<Eye size={18} />}
                label="Review queue"
                hint={count('in_review') || undefined}
                onSelect={runMenu(() => ui.setView('list', 'in_review'))}
              />
              <MenuRow icon={<OctagonX size={18} />} label="Blocked" hint={count('blocked') || undefined} onSelect={runMenu(() => ui.setView('list', 'blocked'))} />
              <MenuSeparator />
              <Show
                when={project()?.archived}
                fallback={<MenuRow icon={<Archive size={18} />} label="Archive project" onSelect={runMenu(() => boardActions().projectAction('archive-project'))} />}
              >
                <MenuRow
                  icon={<ArchiveRestore size={18} />}
                  label="Unarchive project"
                  onSelect={runMenu(() => boardActions().projectAction('unarchive-project'))}
                />
              </Show>
            </Show>
            <MenuSeparator />
            <MenuLabel>
              <span class="live-label">
                <span class="live-led" data-live={store.conn()} />
                Live updates: {liveText(store.conn())}
              </span>
            </MenuLabel>
          </AnchoredMenu>
        )}
      </Show>

      <ChatPickSheet
        open={!!pendingChatPick()}
        hostId={pendingChatPick()?.hostId}
        root={project()?.root}
        label={pendingChatPick()?.mention ? `Add ${pendingChatPick()!.mention!.trim()} to…` : 'Go to a chat'}
        onPick={pickToChat}
        onNewChat={pickNewChat}
        onClose={clearChatPick}
      />
      <PcSheet open={ui.sheet() === 'pc'} onClose={() => ui.setSheet(null)} pcs={pcs()} current={hostId()} onPick={choosePc} />
      <ProjectSheet
        open={ui.sheet() === 'project'}
        onClose={() => ui.setSheet(null)}
        current={slug()}
        onPick={chooseProject}
        onAgent={(item) => {
          ui.setSheet(null);
          void openAgent(item);
        }}
      />
      <AddProjectSheet
        open={addProjectOpen()}
        onClose={() => setAddProjectOpen(false)}
        hostId={hostId()}
        onAdded={() => {
          void refresh().then(() => {
            const first = store.state.projects.find((project) => !project.archived);
            if (!slug() && first) chooseProject(first.slug);
          });
        }}
      />
      <ProjectMenu onPick={chooseProject} />
      <FilterSheet open={ui.sheet() === 'filter'} onClose={() => ui.setSheet(null)} />
      <DisplaySheet open={ui.sheet() === 'display'} onClose={() => ui.setSheet(null)} />
      <SearchSheet open={ui.sheet() === 'search'} onClose={() => ui.setSheet(null)} onOpenProject={chooseProject} />
      <TaskDetail />
      <CardMenu />
      <CommentSheet />
      <ConfirmSheet />
      <NewTaskSheet onHaptic={() => haptic()} />
      <BoardShortcutsSheet />
      <BoardKeys lanes={lanes} project={() => !!slug() && !ui.overview()} />
      <ReportReader />
      <ImageViewer />
    </Show>
  );
}

/** Full-height state (crab + message + Retry) when there is nothing to show. */
function liveText(conn: ConnState): string {
  switch (conn) {
    case 'live':
      return 'on';
    case 'connecting':
      return 'connecting…';
    case 'reconnecting':
      return 'reconnecting…';
    case 'idle':
      return desktopShell() ? 'off · refresh to retry' : 'off · pull to refresh';
  }
}

function BoardState(props: {
  title: string;
  detail?: string;
  tone?: 'error';
  onRetry: () => void;
  retryLabel?: string;
  secondary?: { label: string; run: () => void };
}) {
  return (
    <EmptyState
      tone={props.tone}
      title={props.title}
      body={props.detail && props.detail !== props.title ? props.detail : undefined}
      testId="board-state"
      action={
        <>
          <button type="button" class="btn btn-primary" onClick={props.onRetry}>
            {props.retryLabel ?? 'Retry'}
          </button>
          <Show when={props.secondary}>
            {(secondary) => (
              <button type="button" class="btn btn-secondary" onClick={secondary().run}>
                {secondary().label}
              </button>
            )}
          </Show>
        </>
      }
    />
  );
}

function BoardBanner(props: { title: string; detail?: string; onRetry: () => void; secondary?: { label: string; run: () => void } }) {
  return (
    <aside class="board-banner" role="alert">
      <TriangleAlert size={18} class="shrink-0 text-wm-red" />
      <div class="min-w-0 flex-1">
        <p class="font-medium">{props.title}</p>
        <Show when={props.detail && props.detail !== props.title}>
          <p class="text-text-muted break-words">{props.detail}</p>
        </Show>
        <div class="mt-2 flex gap-2">
          <button type="button" class="banner-button primary" onClick={props.onRetry}>
            Retry
          </button>
          <Show when={props.secondary}>
            {(secondary) => (
              <button type="button" class="banner-button" onClick={secondary().run}>
                {secondary().label}
              </button>
            )}
          </Show>
        </div>
      </div>
    </aside>
  );
}
