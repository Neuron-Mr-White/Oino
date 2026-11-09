import { createEffect, createMemo, createSignal, For, onCleanup, Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import { useSearchParams } from '@solidjs/router';
import { ArrowLeft, ClipboardCheck, Hand, X } from 'lucide-solid';
import { layout } from '../../lib/layout';
import { wrapTab } from '../../lib/focusTrap';
import { boardUi } from '../../lib/board/ui';
import { endQueue, openFromSuperagent, stepQueue, taskOrigin, type QueueRef } from '../../lib/board/taskOrigin';
import { SheetFrame } from '../phone/Sheet';
import { isQuestion } from './api';
import type { SuperagentState } from './state';
import { ReviewTaskLayer } from './ReviewTaskLayer';

type Filter = 'all' | 'review' | 'escalate';
type Row = { projectId: string; taskId: string; kind: 'review' | 'escalate'; question?: string | null };
const validFilter = (value: unknown): value is Filter => value === 'all' || value === 'review' || value === 'escalate';
const key = (r: { projectId: string; taskId: string }) => `${r.projectId}/${r.taskId}`;

export function ReviewWorkspace(props: { sa: SuperagentState; hostId: () => string | undefined }) {
  const [params, setParams] = useSearchParams();
  const [project, setProject] = createSignal('');
  const [search, setSearch] = createSignal('');
  const [removed, setRemoved] = createSignal<string[]>([]);
  const [done, setDone] = createSignal(false);
  const [reply, setReply] = createSignal('');
  const [mount, setMount] = createSignal<HTMLDivElement>();
  let trigger: HTMLElement | undefined;
  let dialog: HTMLDivElement | undefined;
  let prior: ReturnType<typeof taskOrigin>;
  let opening = false;
  let ignoredTask: string | undefined;
  const open = () => validFilter(params.review);
  const filter = (): Filter => validFilter(params.review) ? params.review : 'review';
  const phone = () => !layout().desktop();
  const activeProject = (id: string) => !props.sa.status()?.projects?.some((p) => p.projectId === id && (p.archived || p.blockers?.some((b) => b.code === 'archived')));
  const all = createMemo<Row[]>(() => {
    const status = props.sa.status();
    if (!status) return [];
    const reviews: Row[] = status.review.filter((r) => r.status !== 'archived' && props.sa.meta(r.projectId, r.taskId)?.status !== 'archived' && activeProject(r.projectId)).map((r) => ({ ...r, kind: 'review' }));
    const questions: Row[] = status.needsYou.filter((r) => isQuestion(r) && props.sa.meta(r.projectId, r.taskId)?.status !== 'archived' && activeProject(r.projectId)).map((r) => ({ ...r, kind: 'escalate' }));
    return [...reviews, ...questions].filter((r) => !removed().includes(key(r)));
  });
  const count = (kind: Filter) => kind === 'all' ? all().length : all().filter((r) => r.kind === kind).length;
  const rows = createMemo(() => all().filter((r) => (filter() === 'all' || r.kind === filter()) && (!project() || r.projectId === project()) && `${r.taskId} ${props.sa.projectName(r.projectId)} ${props.sa.meta(r.projectId, r.taskId)?.title ?? ''}`.toLowerCase().includes(search().toLowerCase())));
  const selected = () => all().find((r) => r.taskId === boardUi().openedTask() && r.projectId === taskOrigin()?.items.find((i) => i.task === r.taskId)?.project);
  const projects = createMemo(() => [...new Set(all().map((r) => r.projectId))]);
  createEffect(() => { const s = props.sa.status(); if (s) void props.sa.ensureTitles([...s.review, ...s.needsYou]); });
  let wasOpen = false;
  createEffect(() => {
    const now = open();
    if (wasOpen && !now) { endQueue(); boardUi().closeTask(); prior = undefined; queueMicrotask(() => trigger?.focus()); }
    wasOpen = now;
  });
  createEffect(() => {
    const origin = taskOrigin();
    if (open() && prior && origin && !opening) {
      const missing = prior.items.filter((old) => !origin.items.some((item) => item.project === old.project && item.task === old.task));
      if (missing.length) setRemoved((old) => [...new Set([...old, ...missing.map((r) => `${r.project}/${r.task}`)])]);
    }
    prior = origin;
  });
  createEffect(() => {
    const id = boardUi().openedTask();
    if (!open()) return;
    if (id && params.task !== id) setParams({ task: id }, { replace: true });
    if (!id && taskOrigin() && !opening) {
      endQueue();
      setDone(true);
      setParams({ task: undefined }, { replace: true });
      void props.sa.refresh();
    }
  });
  async function choose(row: Row) {
    const host = props.hostId(); if (!host) return;
    const items: QueueRef[] = rows().map((r) => ({ project: r.projectId, task: r.taskId }));
    opening = true; setDone(false);
    try { await openFromSuperagent(host, row.kind === 'review' ? 'review' : 'needs', items, { project: row.projectId, task: row.taskId }); }
    finally { opening = false; }
  }
  // Restore a deep-linked task once status has arrived.
  createEffect(() => {
    const id = typeof params.task === 'string' ? params.task : undefined;
    if (!id) { ignoredTask = undefined; return; }
    if (!open() || id === ignoredTask || boardUi().openedTask() || opening) return;
    const row = rows().find((r) => r.taskId === id);
    if (row) void choose(row);
  });
  function openFilter(next: Filter, element: HTMLElement) {
    trigger = element; setDone(false);
    setParams({ review: next, task: undefined });
    queueMicrotask(() => dialog?.focus());
  }
  function close() {
    ignoredTask = boardUi().openedTask(); endQueue(); prior = undefined; boardUi().closeTask();
    setParams({ review: undefined, task: undefined }, { replace: true });
    void props.sa.refresh();
    queueMicrotask(() => trigger?.focus());
  }
  function backToList() { ignoredTask = boardUi().openedTask(); setParams({ task: undefined }, { replace: true }); endQueue(); prior = undefined; boardUi().closeTask(); }
  function changeFilter(next: Filter) { backToList(); setDone(false); setParams({ review: next }, { replace: true }); }
  function keys(e: KeyboardEvent) {
    if (e.key === 'Escape') { e.stopPropagation(); e.preventDefault(); close(); return; }
    if (e.key === 'Tab') { wrapTab(e, phone() ? (e.currentTarget as HTMLElement).closest('[role=dialog]') ?? e.currentTarget as HTMLElement : e.currentTarget as HTMLElement); return; }
    const target = e.target as HTMLElement;
    if (target.closest('input,textarea,select,[contenteditable=true]')) return;
    if (e.key === '[' || e.key === ']') { e.preventDefault(); void stepQueue(e.key === '[' ? -1 : 1); return; }
    if (['j', 'k', 'ArrowDown', 'ArrowUp'].includes(e.key)) {
      e.preventDefault(); const list = rows(); const at = list.findIndex((r) => r.taskId === boardUi().openedTask());
      const next = Math.max(0, Math.min(list.length - 1, at + (e.key === 'j' || e.key === 'ArrowDown' ? 1 : -1)));
      if (list[next]) void choose(list[next]!);
    }
  }
  onCleanup(() => { if (open() && taskOrigin()) { boardUi().closeTask(); endQueue(); } });
  const content = () => <div class="sa-review-workspace" onKeyDown={phone() ? keys : undefined}>
    <div class="sa-review-controls">
      <div class="sa-review-tabs" role="tablist" aria-label="Review filters"><For each={['all', 'review', 'escalate'] as Filter[]}>{(f) => <button type="button" role="tab" aria-selected={filter() === f} onClick={() => changeFilter(f)}>{f === 'all' ? 'All' : f === 'review' ? 'Review' : 'Escalate'} · {count(f)}</button>}</For></div>
      <select aria-label="Filter by project" value={project()} onChange={(e) => { backToList(); setProject(e.currentTarget.value); }}><option value="">All projects</option><For each={projects()}>{(p) => <option value={p}>{props.sa.projectName(p)}</option>}</For></select>
      <input type="search" aria-label="Search review queue" placeholder="Search tasks" value={search()} onInput={(e) => { backToList(); setSearch(e.currentTarget.value); }} />
    </div>
    <div class="sa-review-body" data-detail={phone() && boardUi().openedTask() ? '' : undefined}>
      <div class="sa-review-list-pane"><Show when={rows().length} fallback={<p class="sa-empty">Nothing to review.</p>}><For each={rows()}>{(r) => <button type="button" class="sa-review-row" aria-label={`Open ${r.taskId}`} aria-current={boardUi().openedTask() === r.taskId ? 'true' : undefined} onClick={() => void choose(r)}><span class="sa-project">{props.sa.projectName(r.projectId)} · {r.kind === 'review' ? 'Review' : 'Escalate'}</span><strong>{r.taskId}{props.sa.meta(r.projectId, r.taskId)?.title ? ` · ${props.sa.meta(r.projectId, r.taskId)!.title}` : ''}</strong></button>}</For></Show><Show when={done()}><p class="sa-empty" role="status">Queue done</p></Show></div>
      <div class="sa-review-detail"><div class="sa-review-task-mount" ref={setMount} /><Show when={selected()?.kind === 'escalate'}><form class="sa-reply sa-review-reply" onSubmit={(e) => { e.preventDefault(); const row = selected(); if (row && reply().trim()) void props.sa.answer(row.projectId, row.taskId, reply()).then((ok) => { if (ok) setReply(''); }); }}><textarea aria-label="Reply to lead" placeholder="Reply to the lead" value={reply()} onInput={(e) => setReply(e.currentTarget.value)} /><button type="submit" disabled={!reply().trim()}>Send</button></form></Show><Show when={!boardUi().openedTask()}><p class="sa-empty">Select a task to review.</p></Show></div>
    </div>
    <Show when={mount()}>{(el) => <ReviewTaskLayer mount={el()} embedded onBack={backToList} />}</Show>
  </div>;
  return <>
    <div class="sa-review-capsule" role="group" aria-label={`Review queue: ${count('review')} to review, ${count('escalate')} to escalate`}>
      <button type="button" aria-label={`Open review (${count('review')})`} aria-haspopup="dialog" aria-expanded={open() && filter() === 'review'} onClick={(e) => openFilter('review', e.currentTarget)}><ClipboardCheck size={16} /><span class="sa-capsule-label">Review</span><span class="sa-badge" data-zero={count('review') === 0 ? '' : undefined}>{count('review')}</span></button>
      <Show when={count('escalate') > 0}><button type="button" aria-label={`Open escalations (${count('escalate')})`} aria-haspopup="dialog" aria-expanded={open() && filter() === 'escalate'} onClick={(e) => openFilter('escalate', e.currentTarget)}><Hand size={16} /><span class="sa-capsule-label">Escalate</span><span class="sa-badge sa-escalate-badge">{count('escalate')}</span></button></Show>
    </div>
    <Show when={open()}><Show when={!phone()} fallback={<SheetFrame open={open()} onClose={close} label="Review" title="Review" detents={['full']} class="sa-review-sheet" headerAction={<Show when={boardUi().openedTask()} fallback={<button type="button" class="ghost-icon-button" aria-label="Close review" onClick={close}><X size={20} /></button>}><button type="button" class="ghost-icon-button" aria-label="Back to review list" onClick={backToList}><ArrowLeft size={20} /></button></Show>}>{content()}</SheetFrame>}>
      <Portal><div class="sm-overlay sa-review-overlay" onClick={(e) => { if (e.target === e.currentTarget) close(); }}><div class="sm-dialog sa-review-dialog" role="dialog" aria-modal="true" aria-label="Review" tabIndex={-1} ref={dialog} onKeyDown={keys}><header class="sa-review-heading"><h2>Review</h2><button type="button" class="ghost-icon-button" aria-label="Close review" onClick={close}><X size={20} /></button></header>{content()}</div></div></Portal>
    </Show></Show>
  </>;
}
