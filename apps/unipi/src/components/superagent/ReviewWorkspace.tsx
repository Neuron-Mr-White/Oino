import { createEffect, createMemo, createSignal, For, Match, onCleanup, Show, Switch } from 'solid-js';
import { Portal } from 'solid-js/web';
import { useSearchParams } from '@solidjs/router';
import { ArrowLeft, ChevronLeft, ChevronRight, ClipboardCheck, Hand, ShieldAlert, X } from 'lucide-solid';
import { layout } from '../../lib/layout';
import { wrapTab } from '../../lib/focusTrap';
import { boardUi } from '../../lib/board/ui';
import { endQueue, openFromSuperagent, stepQueue, taskOrigin, type QueueRef } from '../../lib/board/taskOrigin';
import { SheetFrame } from '../phone/Sheet';
import { isApproval, isQuestion, type NeedsItem } from './api';
import { ApprovalQueue, Question } from './NeedsYou';
import type { SuperagentState } from './state';
import { ReviewTaskLayer } from './ReviewTaskLayer';

/** The workbench sections: approvals and questions are answered one at a time; reviews walk a queue. */
type Filter = 'approve' | 'review' | 'escalate';
type Row = { projectId: string; taskId: string };
const FILTERS: Filter[] = ['approve', 'review', 'escalate'];
const LABEL: Record<Filter, string> = { approve: 'Approve', review: 'Review', escalate: 'Escalate' };
const validFilter = (value: unknown): value is Filter => value === 'approve' || value === 'review' || value === 'escalate';
const key = (r: { projectId: string; taskId: string }) => `${r.projectId}/${r.taskId}`;

export function ReviewWorkspace(props: { sa: SuperagentState; hostId: () => string | undefined; onOpenSession?: (id: string) => void }) {
  const [params, setParams] = useSearchParams();
  const [project, setProject] = createSignal('');
  const [search, setSearch] = createSignal('');
  const [removed, setRemoved] = createSignal<string[]>([]);
  const [done, setDone] = createSignal(false);
  const [answered, setAnswered] = createSignal<string[]>([]);
  const [qIndex, setQIndex] = createSignal(0);
  const [mount, setMount] = createSignal<HTMLDivElement>();
  let trigger: HTMLElement | undefined;
  let dialog: HTMLDivElement | undefined;
  let root: HTMLDivElement | undefined;
  let capsule: HTMLDivElement | undefined;
  let prior: ReturnType<typeof taskOrigin>;
  let opening = false;
  let ignoredTask: string | undefined;
  const open = () => validFilter(params.review);
  const filter = (): Filter => validFilter(params.review) ? params.review : 'review';
  const phone = () => !layout().desktop();
  const activeProject = (id: string) => !props.sa.status()?.projects?.some((p) => p.projectId === id && (p.archived || p.blockers?.some((b) => b.code === 'archived')));
  const live = (r: Row) => props.sa.meta(r.projectId, r.taskId)?.status !== 'archived' && activeProject(r.projectId);
  const reviews = createMemo<Row[]>(() => (props.sa.status()?.review ?? []).filter((r) => r.status !== 'archived' && live(r) && !removed().includes(key(r))));
  const questions = createMemo<NeedsItem[]>(() => (props.sa.status()?.needsYou ?? []).filter((r) => isQuestion(r) && live(r) && !answered().includes(key(r))));
  const approvals = createMemo(() => (props.sa.status()?.needsYou ?? []).filter(isApproval).filter(live));
  const count = (kind: Filter) => kind === 'approve' ? approvals().length : kind === 'review' ? reviews().length : questions().length;
  const rows = createMemo(() => reviews().filter((r) => (!project() || r.projectId === project()) && `${r.taskId} ${props.sa.projectName(r.projectId)} ${props.sa.meta(r.projectId, r.taskId)?.title ?? ''}`.toLowerCase().includes(search().toLowerCase())));
  const qAt = () => Math.max(0, Math.min(qIndex(), questions().length - 1));
  const question = () => questions()[qAt()];
  const projects = createMemo(() => [...new Set(reviews().map((r) => r.projectId))]);
  createEffect(() => { const s = props.sa.status(); if (s) void props.sa.ensureTitles([...s.review, ...s.needsYou]); });
  let wasOpen = false;
  createEffect(() => {
    const now = open();
    if (wasOpen && !now) { endQueue(); boardUi().closeTask(); prior = undefined; returnFocus(); }
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
    try { await openFromSuperagent(host, 'review', items, { project: row.projectId, task: row.taskId }); }
    finally { opening = false; }
  }
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
    if (!open() || id === ignoredTask || boardUi().openedTask() || opening) return;
    const row = rows().find((r) => r.taskId === id);
    if (row) void choose(row);
  });
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
    ignoredTask = boardUi().openedTask(); endQueue(); prior = undefined; boardUi().closeTask();
    setParams({ review: undefined, task: undefined }, { replace: true });
    void props.sa.refresh();
    returnFocus();
  }
  function backToList() { ignoredTask = boardUi().openedTask(); setParams({ task: undefined }, { replace: true }); endQueue(); prior = undefined; boardUi().closeTask(); }
  function changeFilter(next: Filter) { backToList(); setDone(false); setParams({ review: next }, { replace: true }); }
  function keys(e: KeyboardEvent) {
    if (e.key === 'Escape') { e.stopPropagation(); e.preventDefault(); close(); return; }
    if (e.key === 'Tab') { wrapTab(e, phone() ? (e.currentTarget as HTMLElement).closest('[role=dialog]') ?? e.currentTarget as HTMLElement : e.currentTarget as HTMLElement); return; }
    const target = e.target as HTMLElement;
    if (target.closest('input,textarea,select,[contenteditable=true]')) return;
    if (filter() !== 'review') {
      const step = ['[', 'k', 'ArrowLeft'].includes(e.key) ? -1 : [']', 'j', 'ArrowRight'].includes(e.key) ? 1 : 0;
      if (step) { e.preventDefault(); stepPager(step); return; }
    }
    if (e.key === '[' || e.key === ']') { e.preventDefault(); void stepQueue(e.key === '[' ? -1 : 1); return; }
    if (filter() === 'review' && ['j', 'k', 'ArrowDown', 'ArrowUp'].includes(e.key)) {
      e.preventDefault(); const list = rows(); const at = list.findIndex((r) => r.taskId === boardUi().openedTask());
      const next = Math.max(0, Math.min(list.length - 1, at + (e.key === 'j' || e.key === 'ArrowDown' ? 1 : -1)));
      if (list[next]) void choose(list[next]!);
    }
  }
  onCleanup(() => { if (open() && taskOrigin()) { boardUi().closeTask(); endQueue(); } });
  const content = () => <div class="sa-review-workspace" ref={root} onKeyDown={phone() ? keys : undefined}>
    <div class="sa-review-controls">
      <div class="sa-review-tabs" role="tablist" aria-label="Workbench sections"><For each={FILTERS}>{(f) => <button type="button" role="tab" aria-selected={filter() === f} onClick={() => changeFilter(f)}>{LABEL[f]} · {count(f)}</button>}</For></div>
      <Show when={filter() === 'review'}>
        <select aria-label="Filter by project" value={project()} onChange={(e) => { backToList(); setProject(e.currentTarget.value); }}><option value="">All projects</option><For each={projects()}>{(p) => <option value={p}>{props.sa.projectName(p)}</option>}</For></select>
        <input type="search" aria-label="Search review queue" placeholder="Search tasks" value={search()} onInput={(e) => { backToList(); setSearch(e.currentTarget.value); }} />
      </Show>
    </div>
    <div class="sa-review-body" data-section={filter()} data-ticket={boardUi().openedTask() ? '' : undefined} data-detail={phone() && boardUi().openedTask() ? '' : undefined}>
      <div class="sa-review-list-pane" data-section={filter()}>
        <Show when={props.sa.error()}><div class="sa-message" role="alert">{props.sa.error()} <button type="button" onClick={() => props.sa.clearError()}>Dismiss</button></div></Show>
        <Show when={props.sa.notice()}><p class="sa-message" role="status">{props.sa.notice()}</p></Show>
        <Switch>
          <Match when={filter() === 'approve'}>
            <Show when={props.hostId()}>{(host) => <ApprovalQueue hostId={host()} sa={props.sa} items={approvals()} onOpen={openSession} onTicket={(p, t) => void openOne(p, t)} onAdvance={() => settle(false)} />}</Show>
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
            <Show when={rows().length} fallback={<p class="sa-empty">Nothing to review.</p>}><For each={rows()}>{(r) => <button type="button" class="sa-review-row" aria-label={`Open ${r.taskId}`} aria-current={boardUi().openedTask() === r.taskId ? 'true' : undefined} onClick={() => void choose(r)}><span class="sa-project">{props.sa.projectName(r.projectId)}</span><strong>{r.taskId}{props.sa.meta(r.projectId, r.taskId)?.title ? ` · ${props.sa.meta(r.projectId, r.taskId)!.title}` : ''}</strong></button>}</For></Show>
            <Show when={done()}><p class="sa-empty" role="status">Queue done</p></Show>
          </Match>
        </Switch>
      </div>
      <div class="sa-review-detail"><div class="sa-review-task-mount" ref={setMount} /><Show when={!boardUi().openedTask()}><p class="sa-empty">{filter() === 'review' ? 'Select a task to review.' : 'Open the ticket to see its details here.'}</p></Show></div>
    </div>
    <Show when={mount()}>{(el) => <ReviewTaskLayer mount={el()} embedded onBack={backToList} />}</Show>
  </div>;
  return <>
    <div class="sa-review-capsule" ref={capsule} role="group" aria-label={`Workbench: ${count('approve')} to approve, ${count('review')} to review, ${count('escalate')} to escalate`}>
      <Show when={count('approve') > 0}><button type="button" aria-label={`Open approvals (${count('approve')})`} aria-haspopup="dialog" aria-expanded={open() && filter() === 'approve'} onClick={(e) => openFilter('approve', e.currentTarget)}><ShieldAlert size={14} /><span class="sa-capsule-label">Approve</span><span class="sa-badge sa-approve-badge">{count('approve')}</span></button></Show>
      <button type="button" aria-label={`Open review (${count('review')})`} aria-haspopup="dialog" aria-expanded={open() && filter() === 'review'} onClick={(e) => openFilter('review', e.currentTarget)}><ClipboardCheck size={14} /><span class="sa-capsule-label">Review</span><span class="sa-badge" data-zero={count('review') === 0 ? '' : undefined}>{count('review')}</span></button>
      <Show when={count('escalate') > 0}><button type="button" aria-label={`Open escalations (${count('escalate')})`} aria-haspopup="dialog" aria-expanded={open() && filter() === 'escalate'} onClick={(e) => openFilter('escalate', e.currentTarget)}><Hand size={14} /><span class="sa-capsule-label">Escalate</span><span class="sa-badge sa-escalate-badge">{count('escalate')}</span></button></Show>
    </div>
    <Show when={open()}><Show when={!phone()} fallback={<SheetFrame open={open()} onClose={close} label="Workbench" title="Workbench" detents={['full']} class="sa-review-sheet" headerAction={<Show when={boardUi().openedTask()} fallback={<button type="button" class="ghost-icon-button" aria-label="Close workbench" onClick={close}><X size={20} /></button>}><button type="button" class="ghost-icon-button" aria-label="Back to the workbench" onClick={backToList}><ArrowLeft size={20} /></button></Show>}>{content()}</SheetFrame>}>
      <Portal><div class="sm-overlay sa-review-overlay" onClick={(e) => { if (e.target === e.currentTarget) close(); }}><div class="sm-dialog sa-review-dialog" role="dialog" aria-modal="true" aria-label="Workbench" tabIndex={-1} ref={(el) => { dialog = el; queueMicrotask(() => el.focus()); }} onKeyDown={keys}><header class="sa-review-heading"><h2>Workbench</h2><button type="button" class="ghost-icon-button" aria-label="Close workbench" onClick={close}><X size={20} /></button></header>{content()}</div></div></Portal>
    </Show></Show>
  </>;
}
