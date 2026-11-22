import { createEffect, createMemo, createSignal, For, Match, onCleanup, Show, Switch, untrack } from 'solid-js';
import { Portal } from 'solid-js/web';
import { useLocation, useSearchParams } from '@solidjs/router';
import { ArrowLeft, ArrowUpDown, Check, CheckCheck, ChevronDown, ChevronLeft, ChevronRight, ClipboardCheck, Hand, PanelLeft, Search, ShieldAlert, SlidersHorizontal, X } from 'lucide-solid';
import { layout } from '../../lib/layout';
import { wrapTab } from '../../lib/focusTrap';
import { fuzzyRank } from '../../lib/fuzzy';
import { boardUi, type PaneImage, type ReaderEntry } from '../../lib/board/ui';
import { boardStore } from '../../lib/board/store';
import { reportEntry } from '../../lib/board/report';
import { api, PRIORITIES, PRIORITY_LABEL } from '../../lib/board/api';
import { patchWorkbenchView, readDrafts, readWorkbenchView, resolveTicket, writeDraft, WORKBENCH_SORTS, type TaskRef, type WorkbenchSort } from '../../lib/superagent/workbenchState';
import { filterQueue, SORT_LABEL, sortQueue } from '../../lib/superagent/reviewQueue';
import { hue } from '../../lib/board/view';
import { flushMoves, isPending, onMoveUndone, onMovesSettled } from '../../lib/board/pendingMoves';
import { endQueue, openFromSuperagent, stepQueue, taskOrigin, type QueueRef } from '../../lib/board/taskOrigin';
import { SheetFrame } from '../phone/Sheet';
import { FilterChips, SearchField } from '../phone/controls';
import { AnchoredMenu, MenuCheckRow, MenuLabel, MenuRow, MenuSeparator } from '../board/AnchoredMenu';
import { isApproval, isQuestion, type NeedsItem } from './api';
import { ApprovalQueue, Question } from './NeedsYou';
import type { SuperagentState } from './state';
import { ReviewTaskLayer } from './ReviewTaskLayer';
// The Workbench mounts this outside the Board screen: without the sheet `.att-thumb` falls back to composer.css's 28px chip (OIN-92).
import '../../screens/board.css';

/** The workbench sections: approvals and questions are answered one at a time; reviews walk a queue. */
type Filter = 'approve' | 'review' | 'escalate';
type Row = { projectId: string; taskId: string };
const FILTERS: Filter[] = ['approve', 'review', 'escalate'];
const LABEL: Record<Filter, string> = { approve: 'Approve', review: 'Review', escalate: 'Escalate' };
const validFilter = (value: unknown): value is Filter => value === 'approve' || value === 'review' || value === 'escalate';
const key = (r: { projectId: string; taskId: string }) => `${r.projectId}/${r.taskId}`;
/** From this window width the queue shrinks to a rail while the side pane is open. */
const RAIL_BELOW = 1440;
const TEXT_FIELD = 'input,textarea,select,[contenteditable=true]';
const SCROLL_SAVE_MS = 150;
/** Most urgent first, as the board lists them. */
const PRIORITY_CHOICES = [...PRIORITIES].reverse();

export function ReviewWorkspace(props: { sa: SuperagentState; hostId: () => string | undefined; onOpenSession?: (id: string) => void }) {
  const [params, setParams] = useSearchParams();
  const location = useLocation();
  const [project, setProject] = createSignal('');
  const [search, setSearch] = createSignal('');
  const [removed, setRemoved] = createSignal<string[]>([]);
  const [done, setDone] = createSignal(false);
  const [answered, setAnswered] = createSignal<string[]>([]);
  const [qIndex, setQIndex] = createSignal(0);
  const [approveAt, setApproveAt] = createSignal(0);
  /** True once the state kept for this host was applied: from then on changes are saved (before, they would overwrite it). */
  const [ready, setReady] = createSignal(false);
  /** The side pane to put back once its ticket has loaded. */
  const [pendingPane, setPendingPane] = createSignal<{ task: string; reader?: ReaderEntry; image?: PaneImage }>();
  const [mount, setMount] = createSignal<HTMLDivElement>();
  const [pane, setPane] = createSignal<HTMLElement>();
  const [projectMenu, setProjectMenu] = createSignal<DOMRect>();
  const [projectQuery, setProjectQuery] = createSignal('');
  /** Review queue filters and order (kept per host with the rest of the view). */
  const [priorities, setPriorities] = createSignal<string[]>([]);
  const [labelSel, setLabelSel] = createSignal<string[]>([]);
  const [sort, setSort] = createSignal<WorkbenchSort>('default');
  const [filterMenu, setFilterMenu] = createSignal<DOMRect>();
  const [sortMenu, setSortMenu] = createSignal<DOMRect>();
  const [peek, setPeek] = createSignal(false);
  const [phoneSearch, setPhoneSearch] = createSignal(false);
  let trigger: HTMLElement | undefined;
  let dialog: HTMLDivElement | undefined;
  let root: HTMLDivElement | undefined;
  let capsule: HTMLDivElement | undefined;
  let prior: ReturnType<typeof taskOrigin>;
  let opening = false;
  let ignoredTask: string | undefined;
  /** The list the first row was last opened for: Back to the list does not open it again. */
  let autoKey: string | undefined;
  const ui = boardUi();
  const open = () => validFilter(params.review);
  const filter = (): Filter => validFilter(params.review) ? params.review : 'review';
  const phone = () => !layout().desktop();
  const paneOpen = () => !phone() && (!!ui.reader() || !!ui.paneImage());
  const railable = () => !phone() && filter() === 'review' && paneOpen() && layout().viewport() < RAIL_BELOW;
  const railed = () => railable() && !peek();
  const activeProject = (id: string) => !props.sa.status()?.projects?.some((p) => p.projectId === id && (p.archived || p.blockers?.some((b) => b.code === 'archived')));
  const live = (r: Row) => props.sa.meta(r.projectId, r.taskId)?.status !== 'archived' && activeProject(r.projectId);
  const reviews = createMemo<Row[]>(() => (props.sa.status()?.review ?? []).filter((r) => r.status !== 'archived' && live(r) && !removed().includes(key(r)) && !isPending(r.projectId, r.taskId)));
  const questions = createMemo<NeedsItem[]>(() => (props.sa.status()?.needsYou ?? []).filter((r) => isQuestion(r) && live(r) && !answered().includes(key(r))));
  const approvals = createMemo(() => (props.sa.status()?.needsYou ?? []).filter(isApproval).filter(live));
  const count = (kind: Filter) => kind === 'approve' ? approvals().length : kind === 'review' ? reviews().length : questions().length;
  const metaOf = (r: Row) => props.sa.meta(r.projectId, r.taskId);
  const rows = createMemo(() => sortQueue(
    filterQueue(reviews().filter((r) => (!project() || r.projectId === project()) && `${r.taskId} ${props.sa.projectName(r.projectId)} ${metaOf(r)?.title ?? ''}`.toLowerCase().includes(search().toLowerCase())), metaOf, priorities(), labelSel()),
    metaOf, sort()));
  /** Every label on a ticket in the queue, plus any chosen one that no ticket carries now (so it can be switched off). */
  const labelChoices = createMemo(() => [...new Set([...reviews().flatMap((r) => metaOf(r)?.labels ?? []), ...labelSel()])].sort((a, b) => a.localeCompare(b)));
  const filterCount = () => priorities().length + labelSel().length;
  const qAt = () => Math.max(0, Math.min(qIndex(), questions().length - 1));
  const question = () => questions()[qAt()];
  /** Projects with something to review: "All projects" first, then the newest activity (the newest `updated` among their review tickets). */
  const projects = createMemo(() => {
    const latest = new Map<string, number>();
    for (const r of reviews()) {
      const at = Date.parse(props.sa.meta(r.projectId, r.taskId)?.updated ?? '') || 0;
      latest.set(r.projectId, Math.max(latest.get(r.projectId) ?? 0, at));
    }
    return [...latest.keys()].sort((a, b) => latest.get(b)! - latest.get(a)!);
  });
  const projectCount = (id: string) => reviews().filter((r) => r.projectId === id).length;
  const shownProjects = createMemo(() => {
    const q = projectQuery().trim();
    return q ? fuzzyRank(q, projects(), (p) => [props.sa.projectName(p), p]).map((r) => r.item) : projects();
  });
  createEffect(() => { const s = props.sa.status(); if (s) void props.sa.ensureTitles([...s.review, ...s.needsYou]); });
  // A ticket the host no longer lists (moved, saved after its undo window) can come back later: forget it was removed.
  createEffect(() => {
    const s = props.sa.status();
    if (!s) return;
    setRemoved((old) => { const next = old.filter((k) => s.review.some((r) => key(r) === k)); return next.length === old.length ? old : next; });
  });
  let wasOpen = false;
  createEffect(() => {
    const now = open();
    if (wasOpen && !now) {
      ui.setReader(null); ui.setPaneImage(null); endQueue(); ui.closeTask(); prior = undefined; autoKey = undefined; setPeek(false); returnFocus();
      // Closed on this page (Esc, Back): forget the visit. Leaving the page also drops `?review`, but that must keep it.
      const host = props.hostId();
      if (host && untrack(() => location.pathname).startsWith('/superagent')) patchWorkbenchView(host, { open: undefined, task: undefined, queue: undefined, reader: undefined, image: undefined, listScroll: undefined, ticketScroll: undefined });
    } else if (now && ready()) {
      const host = props.hostId();
      if (host) patchWorkbenchView(host, { open: filter() });
    }
    wasOpen = now;
  });

  // ── The Workbench keeps its place (lib/superagent/workbenchState) ──────────────────────────────
  // Leaving the page drops `?review`/`?task` and disposes this component, so the section, project,
  // search, ticket, pager positions, side pane, scroll and replies are kept per host and put back
  // once the host and its queue are known.
  const scrollTop = { list: 0, tickets: {} as Record<string, number> };
  let pendingScroll: { list?: number; task?: string; ticket?: number } = {};
  let restoredFor: string | undefined;
  let scrollTimer: ReturnType<typeof setTimeout> | undefined;
  const savedTicket = (): TaskRef | undefined => {
    const id = ui.openedTask();
    if (!id) return undefined;
    const origin = taskOrigin();
    const project = origin?.items.find((i) => i.task === id)?.project ?? boardStore().target()?.slug;
    return project ? { project, task: id } : undefined;
  };
  createEffect(() => {
    const host = props.hostId();
    if (!host || !props.sa.status() || restoredFor === host) return;
    restoredFor = host;
    untrack(() => restore(host));
    setReady(true);
  });
  function restore(host: string) {
    const kept = readWorkbenchView(host);
    // Half-typed notes of tickets come back into the board's note store (replies to a question restore themselves).
    for (const [k, text] of Object.entries(readDrafts(host))) if (!k.startsWith('answer:') && !ui.notes[k]) ui.setNote(k, text);
    scrollTop.list = kept.listScroll ?? 0;
    scrollTop.tickets = { ...kept.ticketScroll };
    const urlOpen = open();
    const want: Filter | undefined = urlOpen ? filter() : kept.open;
    // A project with nothing left to review is not in the project menu: show all of them.
    setProject(kept.project && reviews().some((r) => r.projectId === kept.project) ? kept.project : '');
    setSearch(kept.search ?? '');
    setPriorities(kept.priorities ?? []);
    setLabelSel(kept.labels ?? []);
    setSort(kept.sort ?? 'default');
    setQIndex(kept.questionAt ?? 0);
    setApproveAt(kept.approveAt ?? 0);
    if (!want) return;
    if (want !== 'review') { if (!urlOpen) { setDone(false); setParams({ review: want, task: undefined }, { replace: true }); } return; }
    const urlTask = typeof params.task === 'string' ? params.task : undefined;
    const listed = urlTask ? reviews().find((r) => r.taskId === urlTask) : undefined;
    const selected: TaskRef | undefined = urlOpen ? (urlTask ? (kept.task?.task === urlTask ? kept.task : listed && { project: listed.projectId, task: listed.taskId }) : undefined) : kept.task;
    const live: TaskRef[] = rows().map((r) => ({ project: r.projectId, task: r.taskId }));
    const next = resolveTicket(selected, kept.queue, live);
    // Back on the list (no ticket kept): the first row must not open by itself.
    if (!selected && !phone()) autoKey = `${project()}|${search()}`;
    if (next && selected && kept.task && next.task === kept.task.task) {
      pendingScroll = { list: kept.listScroll, task: next.task, ticket: kept.ticketScroll?.[next.task] };
      if (!phone() && (kept.reader || kept.image)) setPendingPane({ task: next.task, reader: kept.reader, image: kept.image });
    } else pendingScroll = { list: kept.listScroll };
    if (!urlOpen || (next && next.task !== urlTask)) { setDone(false); setParams({ review: 'review', task: next?.task }, { replace: true }); }
  }
  // Which ticket is on show, with the order it came from (its neighbour takes over if it is gone on return).
  createEffect(() => {
    const ticket = savedTicket();
    const queue = taskOrigin()?.items;
    const host = untrack(props.hostId);
    if (!ready() || !host || !open() || filter() !== 'review' || !ticket) return;
    patchWorkbenchView(host, { task: ticket, queue: queue ? [...queue] : undefined });
  });
  // The side pane of the ticket on show (a report, a file or an image); closing it forgets it.
  createEffect(() => {
    const reader = ui.reader();
    const image = ui.paneImage();
    const host = untrack(props.hostId);
    if (!ready() || !host || !open() || filter() !== 'review' || !ui.openedTask()) return;
    if (!reader && !image && untrack(pendingPane)) return;
    patchWorkbenchView(host, { reader: reader ?? undefined, image: image ?? undefined });
  });
  createEffect(() => {
    const host = props.hostId();
    if (!ready() || !host || !open()) return;
    patchWorkbenchView(host, { project: project(), search: search(), priorities: priorities(), labels: labelSel(), sort: sort(), questionAt: qAt(), approveAt: approveAt() });
  });
  // The pane of the ticket the owner left comes back once that ticket has loaded (its own effects open the report first).
  createEffect(() => {
    const p = pendingPane();
    const id = ui.openedTask();
    if (p && id && id !== p.task) return setPendingPane(undefined);
    if (!p || phone() || id !== p.task || !boardStore().taskById(id)) return;
    setPendingPane(undefined);
    setTimeout(() => {
      if (ui.openedTask() !== p.task) return;
      if (p.image) { ui.setReader(null); ui.setPaneImage(p.image); }
      else if (p.reader) ui.setReader(p.reader);
    }, 0);
  });
  // Typed notes of tickets are mirrored as they change; a sent (cleared) one is dropped.
  const mirrored = new Map<string, string>();
  createEffect(() => {
    const host = props.hostId();
    if (!ready() || !host) return;
    const seen = new Set<string>();
    for (const k of Object.keys(ui.notes)) {
      seen.add(k);
      const text = ui.notes[k] ?? '';
      if (mirrored.get(k) !== text) { mirrored.set(k, text); writeDraft(host, k, text); }
    }
    for (const k of [...mirrored.keys()]) if (!seen.has(k)) { mirrored.delete(k); writeDraft(host, k, ''); }
  });
  /** Puts a scroller back at `top`; its content arrives a moment after it mounts, so it tries for about a second. */
  function scrollBack(find: () => HTMLElement | null | undefined, top: number | undefined, still: () => boolean) {
    if (!top) return;
    let tries = 0;
    const step = () => {
      if (!still()) return;
      const el = find();
      if (el) { el.scrollTop = top; if (Math.abs(el.scrollTop - top) < 2) return; }
      if (++tries < 60) requestAnimationFrame(step);
    };
    step();
  }
  createEffect(() => {
    if (!open() || filter() !== 'review' || !ready() || !pendingScroll.list) return;
    const top = pendingScroll.list;
    if (!rows().length) return;
    pendingScroll.list = undefined;
    scrollBack(() => root?.querySelector<HTMLElement>('.sa-review-list-pane'), top, () => open());
  });
  createEffect(() => {
    const id = ui.openedTask();
    if (!id || !pendingScroll.ticket || pendingScroll.task !== id || !boardStore().taskById(id)) return;
    const top = pendingScroll.ticket;
    pendingScroll = { ...pendingScroll, ticket: undefined };
    scrollBack(() => root?.querySelector<HTMLElement>('.task-scroll'), top, () => ui.openedTask() === id);
  });
  /** Scroll events do not bubble: the Workbench listens for them in the capture phase and saves the offsets a moment after the last one. */
  function noteScroll(e: Event) {
    const el = e.target as HTMLElement | null;
    const host = props.hostId();
    if (!el?.classList || !host || !ready()) return;
    if (el.classList.contains('task-scroll')) { const id = ui.openedTask(); if (id) scrollTop.tickets[id] = el.scrollTop; }
    else if (el.classList.contains('sa-review-list-pane') && filter() === 'review') scrollTop.list = el.scrollTop;
    else return;
    clearTimeout(scrollTimer);
    scrollTimer = setTimeout(() => patchWorkbenchView(host, { listScroll: scrollTop.list, ticketScroll: { ...scrollTop.tickets } }), SCROLL_SAVE_MS);
  }
  onCleanup(() => {
    const host = props.hostId();
    if (scrollTimer && host && ready()) { clearTimeout(scrollTimer); patchWorkbenchView(host, { listScroll: scrollTop.list, ticketScroll: { ...scrollTop.tickets } }); }
  });
  createEffect(() => { if (!paneOpen()) setPeek(false); });
  createEffect(() => {
    const origin = taskOrigin();
    if (open() && prior && origin && !opening) {
      const missing = prior.items.filter((old) => !origin.items.some((item) => item.project === old.project && item.task === old.task));
      if (missing.length) setRemoved((old) => [...new Set([...old, ...missing.map((r) => `${r.project}/${r.task}`)])]);
    }
    prior = origin;
  });
  createEffect(() => {
    const id = ui.openedTask();
    if (!open()) return;
    if (id && params.task !== id) setParams({ task: id }, { replace: true });
    if (!id && taskOrigin() && !opening) {
      endQueue();
      closePane();
      setDone(true);
      const host = props.hostId();
      if (host) patchWorkbenchView(host, { task: undefined, queue: undefined, reader: undefined, image: undefined });
      setParams({ task: undefined }, { replace: true });
      void props.sa.refresh();
    }
  });
  /** Whoever opened a ticket (a click, a key, the queue moving on) lands on its container, so the keys work at once. */
  createEffect(() => {
    if (!ui.openedTask() || !open() || phone()) return;
    queueMicrotask(() => { if (!document.activeElement?.closest(TEXT_FIELD)) focusTicket(); });
  });
  createEffect(() => { if (done() && !phone()) queueMicrotask(() => root?.querySelector<HTMLElement>('.sa-queue-done button')?.focus()); });
  function focusTicket() { root?.querySelector<HTMLElement>('.task-page[data-embedded]')?.focus({ preventScroll: true }); }
  function focusNote(end = false) {
    const field = root?.querySelector<HTMLTextAreaElement>('.review-dock textarea');
    if (!field) return false;
    field.focus();
    if (end) field.setSelectionRange(field.value.length, field.value.length);
    return true;
  }
  async function choose(row: Row) {
    const host = props.hostId(); if (!host) return;
    const items: QueueRef[] = rows().map((r) => ({ project: r.projectId, task: r.taskId }));
    opening = true; setDone(false);
    try { await openFromSuperagent(host, 'review', items, { project: row.projectId, task: row.taskId }); }
    finally { opening = false; }
  }
  // Desktop: opening Review opens the first row of the filtered list (the list stays put once it was shown).
  createEffect(() => {
    if (!open() || phone() || filter() !== 'review' || done() || typeof params.task === 'string' || ui.openedTask() || opening) return;
    const first = rows()[0];
    const at = `${project()}|${search()}`;
    if (!first || !props.hostId() || autoKey === at) return;
    autoKey = at;
    void choose(first);
  });
  /** The ticket behind an approval or a question, beside the card (a queue of one). */
  async function openOne(project: string, task: string) {
    const host = props.hostId(); if (!host) return;
    const ref = { project, task };
    opening = true; setDone(false);
    try { await openFromSuperagent(host, 'needs', [ref], ref); }
    finally { opening = false; }
  }
  const pager = () => <span class="sa-pager" role="group" aria-label="Questions">
    <button type="button" class="sa-icon-btn" aria-label="Previous question" disabled={qAt() === 0} onClick={() => setQIndex(qAt() - 1)}><ChevronLeft size={16} /></button>
    <span aria-live="polite">{qAt() + 1} of {questions().length}</span>
    <button type="button" class="sa-icon-btn" aria-label="Next question" disabled={qAt() >= questions().length - 1} onClick={() => setQIndex(qAt() + 1)}><ChevronRight size={16} /></button>
  </span>;
  function openSession(id: string) { close(); props.onOpenSession?.(id); }
  // Restore a deep-linked task once status has arrived.
  createEffect(() => {
    const id = typeof params.task === 'string' ? params.task : undefined;
    if (!id) { ignoredTask = undefined; return; }
    if (!open() || id === ignoredTask || ui.openedTask() || opening) return;
    const row = rows().find((r) => r.taskId === id);
    if (row) void choose(row);
  });
  // Undo puts the ticket back in the queue, opens it with its note and puts the caret at the end of the field.
  onCleanup(onMoveUndone(({ project: slug, task }) => {
    setRemoved((old) => old.filter((k) => k !== `${slug}/${task}`));
    if (!open()) return;
    const row = rows().find((r) => r.projectId === slug && r.taskId === task);
    if (row) void choose(row).then(() => setTimeout(() => focusNote(true), 60));
  }));
  onCleanup(onMovesSettled(() => { void props.sa.refresh(); }));
  /** Back to the capsule segment that opened the workbench; if acting emptied it (it hides at zero), to Review, which stays. */
  function returnFocus() {
    queueMicrotask(() => (trigger?.isConnected ? trigger : capsule?.querySelector<HTMLElement>('button'))?.focus());
  }
  /** The card the owner acted on is gone with its focus: hand it to the next reply box (or the dialog), so Esc and Tab keep working. */
  function settle(toReply: boolean) {
    queueMicrotask(() => {
      const host = root?.closest<HTMLElement>('[role=dialog]') ?? root;
      if (!host) return;
      const box = toReply ? host.querySelector<HTMLElement>('.sa-escalation textarea') : null;
      if (box) box.focus();
      else if (!host.contains(document.activeElement) || document.activeElement === host) host.focus();
    });
  }
  /** Steps the one-at-a-time pager (Approve, Escalate) through its own buttons. */
  function stepPager(direction: -1 | 1) {
    root?.querySelector<HTMLButtonElement>(`.sa-pager button[aria-label^="${direction < 0 ? 'Previous' : 'Next'}"]:not(:disabled)`)?.click();
  }
  function openFilter(next: Filter, element: HTMLElement) {
    trigger = element; setDone(false);
    setParams({ review: next, task: undefined });
    queueMicrotask(() => dialog?.focus());
  }
  function close() {
    ignoredTask = ui.openedTask(); endQueue(); prior = undefined; ui.closeTask();
    setParams({ review: undefined, task: undefined }, { replace: true });
    // Moves still inside their undo window are saved now; the queue is read again once they are.
    void flushMoves().then(() => props.sa.refresh());
    returnFocus();
  }
  function backToList() {
    ignoredTask = ui.openedTask(); setParams({ task: undefined }, { replace: true }); endQueue(); prior = undefined; ui.closeTask(); closePane();
    const host = props.hostId();
    if (host && ready()) patchWorkbenchView(host, { task: undefined, queue: undefined, reader: undefined, image: undefined });
  }
  function changeFilter(next: Filter) { backToList(); setDone(false); autoKey = undefined; setParams({ review: next }, { replace: true }); }
  function closePane() { ui.setReader(null); ui.setPaneImage(null); }
  /** `O`: the pane shows the open ticket's full report, or goes away. */
  function togglePane() {
    if (paneOpen()) return closePane();
    const id = ui.openedTask();
    const store = boardStore();
    const task = id ? store.taskById(id) : undefined;
    const entry = task ? reportEntry(task) : null;
    if (!entry) return;
    ui.setReader(entry);
    // `list --brief` cuts the report in the list row: show the trimmed text,
    // then swap in the full `show` report (on failure keep the trimmed one).
    if (!task?.statusReportTrimmed) return;
    const host = props.hostId();
    const slug = store.target()?.slug;
    if (!host || !slug || !id) return;
    void api.task(host, slug, id).then((full) => {
      const fullEntry = reportEntry(full);
      if (fullEntry && ui.reader()?.text === entry.text) ui.setReader(fullEntry);
    }).catch(() => {});
  }
  function pickProject(next: string) { setProjectMenu(undefined); setProjectQuery(''); if (next === project()) return; backToList(); autoKey = undefined; setProject(next); }
  /** A changed filter or order changes the queue: back to the list, and the first row opens again (as with the project). */
  function changeQueue(apply: () => void) { backToList(); autoKey = undefined; apply(); }
  const toggled = (list: string[], v: string) => (list.includes(v) ? list.filter((x) => x !== v) : [...list, v]);
  function keys(e: KeyboardEvent) {
    const target = e.target as HTMLElement;
    // Sheets, menus and the previewer are portals: their key events bubble up here through Solid's tree, but they are not the Workbench's own.
    if (!(e.currentTarget as HTMLElement | null)?.contains(target)) return;
    const typing = !!target.closest?.(TEXT_FIELD);
    if (e.key === 'Escape') {
      e.stopPropagation(); e.preventDefault();
      // In a field Esc leaves it; elsewhere it closes the pane first, then the workbench.
      if (typing) { (target as HTMLElement).blur(); (filter() === 'review' && ui.openedTask() ? focusTicket() : (dialog ?? root)?.focus()); return; }
      if (paneOpen()) { closePane(); return; }
      close(); return;
    }
    if (e.key === 'Tab') { wrapTab(e, phone() ? (e.currentTarget as HTMLElement).closest('[role=dialog]') ?? e.currentTarget as HTMLElement : e.currentTarget as HTMLElement); return; }
    if (typing || e.ctrlKey || e.metaKey || e.altKey) return;
    if (filter() !== 'review') {
      const step = ['[', 'k', 'ArrowLeft'].includes(e.key) ? -1 : [']', 'j', 'ArrowRight'].includes(e.key) ? 1 : 0;
      if (step) { e.preventDefault(); stepPager(step); return; }
    }
    if (e.key === '[' || e.key === ']') { e.preventDefault(); void stepQueue(e.key === '[' ? -1 : 1); return; }
    if (filter() !== 'review') return;
    if (['j', 'k', 'ArrowDown', 'ArrowUp'].includes(e.key)) {
      e.preventDefault(); const list = rows(); const at = list.findIndex((r) => r.taskId === ui.openedTask());
      const next = Math.max(0, Math.min(list.length - 1, at + (e.key === 'j' || e.key === 'ArrowDown' ? 1 : -1)));
      if (list[next]) void choose(list[next]!);
      return;
    }
    // D / R / O live in the Workbench only: the Board already uses R.
    const k = e.key.toLowerCase();
    if (k === 'd') { const button = root?.querySelector<HTMLButtonElement>('.review-dock [data-action=done]'); if (button) { e.preventDefault(); button.click(); } }
    else if (k === 'r') { if (focusNote()) e.preventDefault(); }
    else if (k === 'o') { e.preventDefault(); togglePane(); }
    else if (e.key === '/') { e.preventDefault(); root?.querySelector<HTMLInputElement>('.sa-review-controls .ph-search input')?.focus(); }
    else if (e.key === '\\' && railable()) { e.preventDefault(); setPeek(!peek()); }
  }
  onCleanup(() => { if (open()) { ui.setReader(null); ui.setPaneImage(null); } if (open() && taskOrigin()) { ui.closeTask(); endQueue(); } void flushMoves(); });
  const tile = (id: string) => <span class="sa-tile" style={{ background: hue(id) }} aria-hidden="true">{props.sa.projectName(id).slice(0, 1).toUpperCase()}</span>;
  const projectChip = () => <button type="button" class="ph-chip sa-project-chip" aria-haspopup="menu" aria-expanded={projectMenu() !== undefined} aria-label={`Project: ${project() ? props.sa.projectName(project()) : 'All projects'}`}
    onClick={(e) => setProjectMenu(e.currentTarget.getBoundingClientRect())}>
    <Show when={project()} fallback={<span class="sa-tile sa-tile-all" aria-hidden="true"><CheckCheck size={11} /></span>}>{tile(project())}</Show>
    <span class="sa-chip-label">{project() ? props.sa.projectName(project()) : 'All projects'}</span>
    <ChevronDown size={14} aria-hidden="true" />
  </button>;
  const filterChip = () => <button type="button" class="ph-chip sa-project-chip sa-filter-chip" data-active={filterCount() ? '' : undefined} aria-haspopup="menu" aria-expanded={filterMenu() !== undefined} aria-label={filterCount() ? `Filter tasks (${filterCount()} on)` : 'Filter tasks'}
    onClick={(e) => setFilterMenu(e.currentTarget.getBoundingClientRect())}>
    <SlidersHorizontal size={14} aria-hidden="true" />
    <span class="sa-chip-label">Filter</span>
    <Show when={filterCount()}><span class="sa-badge">{filterCount()}</span></Show>
  </button>;
  const sortChip = () => <button type="button" class="ph-chip sa-project-chip sa-sort-chip" data-active={sort() !== 'default' ? '' : undefined} aria-haspopup="menu" aria-expanded={sortMenu() !== undefined} aria-label={`Sort: ${SORT_LABEL[sort()]}`}
    onClick={(e) => setSortMenu(e.currentTarget.getBoundingClientRect())}>
    <ArrowUpDown size={14} aria-hidden="true" />
    <span class="sa-chip-label">{sort() === 'default' ? 'Sort' : SORT_LABEL[sort()]}</span>
  </button>;
  const content = () => <div class="sa-review-workspace" ref={(el) => { root = el; el.addEventListener('scroll', noteScroll, true); }} onKeyDown={phone() ? keys : undefined}>
    <div class="sa-review-controls" data-phone-search={phoneSearch() ? '' : undefined}>
      <FilterChips label="Workbench sections" value={filter()} options={FILTERS.map((f) => ({ value: f, label: LABEL[f], count: count(f) }))} onChange={(f: Filter) => changeFilter(f)} />
      <div class="sa-review-filters" data-off={filter() !== 'review' ? '' : undefined} aria-hidden={filter() !== 'review' ? 'true' : undefined}>
        <Show when={!phone() || !phoneSearch()}>{projectChip()}{filterChip()}{sortChip()}</Show>
        <Show when={phone()} fallback={<SearchField value={search()} placeholder="Search tasks" onInput={(q) => { backToList(); setSearch(q); }} />}>
          <Show when={phoneSearch()} fallback={<button type="button" class="ph-icon-btn sa-search-toggle" aria-label="Search tasks" onClick={() => setPhoneSearch(true)}><Search size={20} /></button>}>
            <SearchField value={search()} placeholder="Search tasks" autofocus onInput={(q) => { backToList(); setSearch(q); }} />
            <button type="button" class="ph-icon-btn sa-search-toggle" aria-label="Close search" onClick={() => { setPhoneSearch(false); if (search()) { backToList(); setSearch(''); } }}><X size={20} /></button>
          </Show>
        </Show>
      </div>
    </div>
    <div class="sa-review-body" data-section={filter()} data-ticket={ui.openedTask() ? '' : undefined} data-detail={phone() && ui.openedTask() ? '' : undefined} data-done={filter() === 'review' && done() ? '' : undefined} data-rail={railed() ? '' : undefined} data-peek={railable() && peek() ? '' : undefined} data-pane={paneOpen() ? '' : undefined}>
      <div class="sa-review-list-pane" data-section={filter()}>
        <Show when={railable()}>
          <button type="button" class="sa-queue-toggle" aria-label="Queue" aria-expanded={!railed()} title="Queue (\)" onClick={() => setPeek(!peek())}><PanelLeft size={16} /><span class="sa-queue-toggle-label">Queue</span></button>
        </Show>
        <Show when={props.sa.error()}><div class="sa-message" role="alert">{props.sa.error()} <button type="button" onClick={() => props.sa.clearError()}>Dismiss</button></div></Show>
        <Show when={props.sa.notice()}><p class="sa-message" role="status">{props.sa.notice()}</p></Show>
        <Switch>
          <Match when={filter() === 'approve'}>
            <Show when={props.hostId()}>{(host) => <ApprovalQueue hostId={host()} sa={props.sa} items={approvals()} onOpen={openSession} onTicket={(p, t) => void openOne(p, t)} onAdvance={() => settle(false)} initialAt={approveAt()} onAt={setApproveAt} />}</Show>
          </Match>
          <Match when={filter() === 'escalate'}>
            <Show when={question() ? key(question()!) : undefined} keyed fallback={<p class="sa-empty">No questions waiting.</p>}>{(k) => {
              const item = questions().find((q) => key(q) === k)!;
              return <div class="sa-escalation" data-testid="sa-escalation">
                <Question sa={props.sa} item={item} pager={questions().length > 1 ? pager() : undefined} onTicket={(p, t) => void openOne(p, t)} onSent={() => { setAnswered((old) => [...old, k]); settle(true); }} />
              </div>;
            }}</Show>
          </Match>
          <Match when={filter() === 'review'}>
            <Show when={rows().length} fallback={<p class="sa-empty">{done() ? '' : reviews().length ? 'No match' : 'Nothing to review.'}</p>}><For each={rows()}>{(r, i) => {
              const title = () => props.sa.meta(r.projectId, r.taskId)?.title;
              return <button type="button" class="sa-review-row" aria-label={`Open ${r.taskId}`} title={`${props.sa.projectName(r.projectId)} · ${r.taskId}${title() ? ` · ${title()}` : ''}`} aria-current={ui.openedTask() === r.taskId ? 'true' : undefined} onClick={() => void choose(r)}>
                <span class="sa-rail" aria-hidden="true"><span class="sa-rail-dot" style={{ background: hue(r.projectId) }} /><span class="sa-rail-pos">{i() + 1}</span></span>
                <span class="sa-row-text"><span class="sa-project">{props.sa.projectName(r.projectId)}</span><strong>{r.taskId}{title() ? ` · ${title()}` : ''}</strong></span>
              </button>;
            }}</For></Show>
          </Match>
        </Switch>
      </div>
      <div class="sa-review-detail">
        <div class="sa-review-task-mount" ref={setMount} />
        <Show when={!ui.openedTask()}>
          <Show when={done()} fallback={<p class="sa-empty">{filter() === 'review' ? (rows().length ? 'Select a task to review.' : reviews().length ? 'No task matches the project or search.' : '') : 'Open the ticket to see its details here.'}</p>}>
            <div class="sa-queue-done" role="status">
              <CheckCheck size={28} aria-hidden="true" />
              <h3>Queue done</h3>
              <p>Nothing left to review.</p>
              <button type="button" class="btn btn-secondary dock-btn" data-kind="secondary" onClick={close}>Close workbench</button>
            </div>
          </Show>
        </Show>
      </div>
      <Show when={!phone()}><aside class="sa-review-pane" ref={setPane} data-open={paneOpen() ? '' : undefined} aria-label="Side pane" /></Show>
    </div>
    <Show when={mount()}>{(el) => <ReviewTaskLayer mount={el()} readerMount={root} paneMount={phone() ? undefined : pane()} embedded onBack={backToList} />}</Show>
    <Show when={filterMenu()}>{(anchor) => (
      <AnchoredMenu anchor={anchor()} width={264} label="Filter tasks" class="sa-filter-menu" onDismiss={() => setFilterMenu(undefined)}>
        <MenuLabel>Priority</MenuLabel>
        <For each={PRIORITY_CHOICES}>{(id) => <MenuCheckRow label={PRIORITY_LABEL[id] ?? id} checked={priorities().includes(id)} testId={`filter-priority-${id}`} onSelect={() => changeQueue(() => setPriorities(toggled(priorities(), id)))} />}</For>
        <MenuSeparator />
        <MenuLabel>Labels</MenuLabel>
        <For each={labelChoices()} fallback={<p class="sa-empty">No labels on these tasks.</p>}>
          {(label) => <MenuCheckRow label={label} checked={labelSel().includes(label)} testId={`filter-label-${label}`} onSelect={() => changeQueue(() => setLabelSel(toggled(labelSel(), label)))} />}
        </For>
        <Show when={filterCount()}>
          <MenuSeparator />
          <MenuRow label="Clear filters" testId="filter-clear" onSelect={() => changeQueue(() => { setPriorities([]); setLabelSel([]); })} />
        </Show>
      </AnchoredMenu>
    )}</Show>
    <Show when={sortMenu()}>{(anchor) => (
      <AnchoredMenu anchor={anchor()} width={232} label="Sort tasks" class="sa-sort-menu" onDismiss={() => setSortMenu(undefined)}>
        <For each={WORKBENCH_SORTS}>{(id) => <MenuRow label={SORT_LABEL[id]} testId={`sort-${id}`} hint={sort() === id ? <Check size={14} /> : null} onSelect={() => { setSortMenu(undefined); if (id !== sort()) changeQueue(() => setSort(id)); }} />}</For>
      </AnchoredMenu>
    )}</Show>
    <Show when={projectMenu()}>{(anchor) => (
      <AnchoredMenu anchor={anchor()} width={288} label="Filter by project" class="sa-project-menu" onDismiss={() => { setProjectMenu(undefined); setProjectQuery(''); }}>
        <div class="sa-project-search">
          <Search size={15} aria-hidden="true" />
          <input type="text" aria-label="Search projects" placeholder="Search projects" autocomplete="off" spellcheck={false} ref={(el) => setTimeout(() => el.focus(), 0)} value={projectQuery()}
            onInput={(e) => setProjectQuery(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === 'Home' || e.key === 'End') e.stopPropagation();
              // The menu is a portal: Esc here closes only the menu, not the Workbench behind it.
              if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); setProjectMenu(undefined); setProjectQuery(''); }
              if (e.key === 'Enter') { e.preventDefault(); const first = shownProjects()[0]; if (first) pickProject(first); else if (!projectQuery().trim()) pickProject(''); }
            }} />
        </div>
        <Show when={!projectQuery().trim()}>
          <MenuRow icon={<span class="sa-tile sa-tile-all"><CheckCheck size={11} /></span>} label="All projects" hint={<><span class="sa-badge">{reviews().length}</span>{!project() ? <Check size={14} /> : null}</>} onSelect={() => pickProject('')} />
        </Show>
        <For each={shownProjects()} fallback={<Show when={projectQuery().trim()}><p class="sa-empty">No project matches.</p></Show>}>
          {(p) => <MenuRow icon={tile(p)} label={props.sa.projectName(p)} hint={<><span class="sa-badge">{projectCount(p)}</span>{project() === p ? <Check size={14} /> : null}</>} onSelect={() => pickProject(p)} />}
        </For>
      </AnchoredMenu>
    )}</Show>
  </div>;
  return <>
    <div class="sa-review-capsule" ref={capsule} role="group" aria-label={`Workbench: ${count('approve')} to approve, ${count('review')} to review, ${count('escalate')} to escalate`}>
      <Show when={count('approve') > 0}><button type="button" aria-label={`Open approvals (${count('approve')})`} aria-haspopup="dialog" aria-expanded={open() && filter() === 'approve'} onClick={(e) => openFilter('approve', e.currentTarget)}><ShieldAlert size={14} /><span class="sa-capsule-label">Approve</span><span class="sa-badge sa-approve-badge">{count('approve')}</span></button></Show>
      <button type="button" aria-label={`Open review (${count('review')})`} aria-haspopup="dialog" aria-expanded={open() && filter() === 'review'} onClick={(e) => openFilter('review', e.currentTarget)}><ClipboardCheck size={14} /><span class="sa-capsule-label">Review</span><span class="sa-badge" data-zero={count('review') === 0 ? '' : undefined}>{count('review')}</span></button>
      <Show when={count('escalate') > 0}><button type="button" aria-label={`Open escalations (${count('escalate')})`} aria-haspopup="dialog" aria-expanded={open() && filter() === 'escalate'} onClick={(e) => openFilter('escalate', e.currentTarget)}><Hand size={14} /><span class="sa-capsule-label">Escalate</span><span class="sa-badge sa-escalate-badge">{count('escalate')}</span></button></Show>
    </div>
    <Show when={open()}><Show when={!phone()} fallback={<SheetFrame open={open()} onClose={close} label="Workbench" title="Workbench" detents={['full']} class="sa-review-sheet" headerAction={<Show when={ui.openedTask()} fallback={<button type="button" class="ghost-icon-button" aria-label="Close workbench" onClick={close}><X size={20} /></button>}><button type="button" class="ghost-icon-button" aria-label="Back to the workbench" onClick={backToList}><ArrowLeft size={20} /></button></Show>}>{content()}</SheetFrame>}>
      <Portal><div class="sm-overlay sa-review-overlay" onClick={(e) => { if (e.target === e.currentTarget) close(); }}><div class="sm-dialog sa-review-dialog" role="dialog" aria-modal="true" aria-label="Workbench" tabIndex={-1} ref={(el) => { dialog = el; queueMicrotask(() => el.focus()); }} onKeyDown={keys}><header class="sa-review-heading"><h2>Workbench</h2><button type="button" class="ghost-icon-button" aria-label="Close workbench" onClick={close}><X size={20} /></button></header>{content()}</div></div></Portal>
    </Show></Show>
  </>;
}
