/**
 * Task detail (UNI-131) — the kanboard web UI's TaskPanel drawer as a
 * full-screen page pushed over the board (it covers the bottom nav, like
 * the terminal layer): header (Back, status + id, ⋯), the title edited
 * in place, status / priority chips (anchored menus: only allowed moves),
 * the In Review / Blocked report with one-tap next steps, the agent run
 * (stale runs can be released), labels, the description (rendered
 * markdown; a full-page editor), dependencies (ready / waiting / locked,
 * tap to open, link / unlink without cycles), dependents, attachments
 * (thumbnails → lightbox, files → share/save), the activity timeline with
 * the report reader, details, and the note composer pinned above the
 * keyboard.
 *
 * Why a page and not a sheet: the detail is long (report + activity) and
 * has its own overlays (move menu, comment sheet, pickers, reader,
 * image viewer); a sheet would stack sheets and fight its own
 * swipe-to-dismiss while scrolling. Back (and ←) returns to the board —
 * or to the previous task after a dependency jump.
 */
import { createEffect, createSignal, For, on, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { Portal } from 'solid-js/web';
import { ArrowLeft, BookOpen, Check, ChevronDown, CircleSlash, EllipsisVertical, Link2, Lock, Paperclip, Pencil, Plus, RotateCcw, Tag, X } from 'lucide-solid';
import { displayTitle, laneLabel, PRIORITY_LABEL, type Attachment, type Task } from '../../lib/board/api';
import { attKind, relativeTime } from '../../lib/board/markdown';
import { formatBytes } from '../../lib/board/attach';
import { boardStore } from '../../lib/board/store';
import { boardUi } from '../../lib/board/ui';
import { elapsed, hue } from '../../lib/board/view';
import { claimNote, claimState, dependentsOf, statusMoves, statusReport } from '../../lib/board/report';
import {
  addLabel,
  boardCtx,
  copyToClipboard,
  editTask,
  moveTask,
  scheduleDeps,
  setDependency,
  toggleLabel,
  uploadToTask,
} from '../../lib/board/taskActions';
import { api } from '../../lib/board/api';
import { pushBack } from '../../lib/back';
import { highlightTask } from '../../lib/board/actions';
import { BoardSheet } from './BoardSheet';
import { BoardToasts } from './BoardToasts';
import { PriorityGlyph, StatusGlyph } from './Glyphs';
import { now } from './TaskCard';
import { Markdown, openFile, taskFilePath } from './Markdown';
import { ActivityList } from './Activity';
import { TaskComposer, AttachMenu, createFilePickers } from './TaskComposer';
import { BodyEditor } from './BodyEditor';
import { MoveMenu, PriorityMenu, TaskMoreMenu } from './TaskMenus';
import { DepPicker, LabelPicker } from './NewTaskSheet';

export function TaskDetail() {
  const ui = boardUi();
  return <Show when={ui.openedTask()}>{(id) => <TaskPage id={id()} />}</Show>;
}

function TaskPage(props: { id: string }) {
  const store = boardStore();
  const ui = boardUi();
  const task = () => store.taskById(props.id);
  const [title, setTitle] = createSignal('');
  const [body, setBody] = createSignal('');
  const [editing, setEditing] = createSignal(false);
  const [savingBody, setSavingBody] = createSignal(false);
  const [note, setNote] = createSignal('');
  const [uploading, setUploading] = createSignal(0);
  const [confirmDiscard, setConfirmDiscard] = createSignal(false);
  const [menu, setMenu] = createSignal<{ kind: 'more' | 'move' | 'priority' | 'attach'; anchor: DOMRect } | null>(null);
  const [sheet, setSheet] = createSignal<'labels' | 'deps' | null>(null);
  let scroller: HTMLDivElement | undefined;
  let titleArea: HTMLTextAreaElement | undefined;

  // Drafts reset when a different task opens (not on every live refresh).
  createEffect(
    on(
      () => props.id,
      () => {
        setTitle(task()?.title ?? '');
        setBody(task()?.body ?? '');
        setEditing(false);
        setNote('');
        setConfirmDiscard(false);
        setMenu(null);
        setSheet(null);
        scroller?.scrollTo?.({ top: 0 });
        queueMicrotask(fitTitle);
      },
    ),
  );
  createEffect(
    on(
      () => task()?.title,
      (next) => {
        if (document.activeElement === titleArea) return;
        setTitle(next ?? '');
        queueMicrotask(fitTitle);
      },
    ),
  );

  function fitTitle() {
    if (!titleArea) return;
    titleArea.style.height = 'auto';
    titleArea.style.height = `${titleArea.scrollHeight}px`;
  }

  /** Web saveTitle: clearing it is fine while the body carries the point; otherwise keep the old title. */
  function saveTitle() {
    const current = task();
    if (!current) return;
    const next = title().trim();
    if (next === current.title) return;
    if (next || (current.body ?? '').trim()) void editTask(current, { title: next });
    else setTitle(current.title);
  }

  const dirty = () => note().trim().length > 0 || uploading() > 0;
  function requestBack() {
    if (titleArea && document.activeElement === titleArea) titleArea.blur();
    saveTitle();
    if (confirmDiscard() || !dirty()) {
      setConfirmDiscard(false);
      const last = ui.detail().length === 1;
      const id = props.id; // read before popTask unmounts this page
      ui.popTask();
      // Back on the board: show where this task sits (lane page + flash).
      if (last) highlightTask(id, true);
      return;
    }
    setConfirmDiscard(true);
  }
  onMount(() => onCleanup(pushBack(requestBack)));
  // The title's height follows its width: re-fit when the pane/window is resized
  // (desktop splitter, crossing the 900px shell breakpoint, rotating a phone).
  let titleObserver: ResizeObserver | undefined;
  onCleanup(() => titleObserver?.disconnect());
  function bindTitle(el: HTMLTextAreaElement) {
    titleArea = el;
    titleObserver?.disconnect();
    if (typeof ResizeObserver === 'undefined') return;
    let lastWidth = -1;
    titleObserver = new ResizeObserver(() => {
      const width = el.clientWidth;
      if (width === lastWidth) return;
      lastWidth = width;
      fitTitle();
    });
    titleObserver.observe(el);
  }

  function startEdit() {
    setBody(task()?.body ?? '');
    setEditing(true);
  }
  async function saveBody() {
    const current = task();
    if (!current) return;
    if (body() === (current.body ?? '')) return setEditing(false);
    setSavingBody(true);
    const saved = await editTask(current, { body: body() }, 'Description saved');
    setSavingBody(false);
    if (saved) setEditing(false);
  }

  const openMenu = (kind: 'more' | 'move' | 'priority' | 'attach') => (event: MouseEvent) =>
    setMenu({ kind, anchor: (event.currentTarget as HTMLElement).getBoundingClientRect() });
  const closeMenu = () => setMenu(null);

  return (
    <Portal>
      <div class="board-page task-page" role="dialog" aria-label={`${props.id} details`} data-testid="task-detail" data-lane={task()?.status}>
        <Show
          when={task()}
          fallback={
            <>
              <header class="task-head">
                <button type="button" class="ghost-icon-button" aria-label="Back" onClick={() => ui.popTask()}>
                  <ArrowLeft size={22} />
                </button>
              </header>
              <div class="board-state">
                <CircleSlash size={40} class="text-text-dim" />
                <h2>{props.id} isn't on this board</h2>
                <p>It may have been archived or removed on the PC.</p>
              </div>
            </>
          }
        >
          {(current) => (
            <>
              <header class="task-head">
                <button type="button" class="ghost-icon-button" aria-label={ui.detail().length > 1 ? 'Back to the previous task' : 'Back to the board'} onClick={requestBack}>
                  <ArrowLeft size={22} />
                </button>
                <button type="button" class="task-crumb" aria-label={`Copy ${current().id}`} onClick={() => void copyToClipboard(current().id, current().id)}>
                  <StatusGlyph status={current().status} size={15} />
                  <span class="font-mono">{current().id}</span>
                  <span class="task-crumb-lane">{laneLabel(current().status)}</span>
                </button>
                <span class="flex-1" />
                <button type="button" class="ghost-icon-button" aria-label="Task actions" aria-expanded={menu()?.kind === 'more'} onClick={openMenu('more')}>
                  <EllipsisVertical size={20} />
                </button>
              </header>
              <Show when={confirmDiscard()}>
                <div class="discard-strip page-strip" role="alertdialog" aria-label="Discard note?">
                  <span class="flex-1 text-[15px]">{uploading() > 0 ? 'An upload is still running.' : 'Discard your unsent note?'}</span>
                  <button type="button" class="board-text-button" onClick={() => setConfirmDiscard(false)}>
                    Keep
                  </button>
                  <button type="button" class="discard-button" onClick={requestBack}>
                    Discard
                  </button>
                </div>
              </Show>

              <div class="board-page-scroll task-scroll" ref={scroller}>
                <textarea
                  ref={bindTitle}
                  class="task-title-edit"
                  classList={{ derived: !title() }}
                  rows={1}
                  value={title()}
                  placeholder={current().title ? 'Untitled — tap to add a title' : displayTitle(current())}
                  aria-label="Task title"
                  enterkeyhint="done"
                  onInput={(event) => {
                    setTitle(event.currentTarget.value.replace(/\n/g, ' '));
                    fitTitle();
                  }}
                  onBlur={saveTitle}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter') {
                      event.preventDefault();
                      event.currentTarget.blur();
                    }
                  }}
                />
                <Show when={!current().title}>
                  <p class="task-derived-note">No title yet — the card shows the first line of the description.</p>
                </Show>

                <div class="task-chips">
                  <button
                    type="button"
                    class="prop-chip"
                    data-lane={current().status}
                    aria-label={`Status: ${laneLabel(current().status)}`}
                    aria-expanded={menu()?.kind === 'move'}
                    onClick={openMenu('move')}
                  >
                    <StatusGlyph status={current().status} size={15} />
                    {laneLabel(current().status)}
                    <ChevronDown size={15} class="text-text-dim" />
                  </button>
                  <button
                    type="button"
                    class="prop-chip"
                    classList={{ muted: current().priority === 'none' }}
                    aria-label={`Priority: ${PRIORITY_LABEL[current().priority] ?? current().priority}`}
                    aria-expanded={menu()?.kind === 'priority'}
                    onClick={openMenu('priority')}
                  >
                    <PriorityGlyph priority={current().priority} size={15} />
                    {PRIORITY_LABEL[current().priority] ?? current().priority}
                    <ChevronDown size={15} class="text-text-dim" />
                  </button>
                  <For each={current().labels ?? []}>
                    {(label) => (
                      <button type="button" class="prop-chip label-chip" style={{ '--label-hue': hue(label) }} aria-label={`Label ${label} — edit labels`} onClick={() => setSheet('labels')}>
                        {label}
                      </button>
                    )}
                  </For>
                  <button type="button" class="prop-chip add-chip" aria-label="Edit labels" onClick={() => setSheet('labels')}>
                    <Tag size={15} />
                    <Show when={(current().labels ?? []).length === 0}>Labels</Show>
                    <Show when={(current().labels ?? []).length > 0}>
                      <Plus size={14} />
                    </Show>
                  </button>
                </div>

                <ReportCard task={current()} />
                <RunCard task={current()} />

                <Section
                  title="Description"
                  action={<SectionAction icon={<Pencil size={15} />} label="Edit description" text="Edit" onClick={startEdit} />}
                >
                  <Show
                    when={(current().body ?? '').trim()}
                    fallback={
                      <button type="button" class="body-empty" onClick={startEdit}>
                        <Pencil size={16} /> Add a description…
                      </button>
                    }
                  >
                    <div class="body-card">
                      <Markdown text={current().body} label="Description" />
                    </div>
                  </Show>
                </Section>

                <Show when={(current().deps ?? []).length > 0}>
                  <DepsSection task={current()} onAdd={() => setSheet('deps')} />
                </Show>
                <DependentsSection task={current()} />
                <AttachmentsSection task={current()} onAdd={openMenu('attach')} uploading={uploading()} setUploading={setUploading} menuOpen={menu()?.kind === 'attach'} onCloseMenu={closeMenu} anchor={menu()?.kind === 'attach' ? menu()!.anchor : undefined} />
                <Show when={(current().deps ?? []).length === 0 || ((current().attachments ?? []).length === 0 && uploading() === 0)}>
                  <div class="task-add-row">
                    <Show when={(current().deps ?? []).length === 0}>
                      <button type="button" class="prop-add" aria-label="Add dependency" onClick={() => setSheet('deps')}>
                        <Link2 size={15} /> Runs after…
                      </button>
                    </Show>
                    <Show when={(current().attachments ?? []).length === 0 && uploading() === 0}>
                      <button type="button" class="prop-add" aria-label="Add attachment" aria-expanded={menu()?.kind === 'attach'} onClick={openMenu('attach')}>
                        <Paperclip size={15} /> Attach
                      </button>
                    </Show>
                  </div>
                </Show>

                <Section title="Activity" count={current().activity.length}>
                  <ActivityList task={current()} />
                </Section>

                <Section title="Details">
                  <dl class="meta-lines">
                    <dt>Created</dt>
                    <dd title={current().created}>
                      {relativeTime(current().created)}
                      <Show when={current().creator}> by {current().creator === 'agent' ? 'an agent' : 'you'}</Show>
                    </dd>
                    <dt>Updated</dt>
                    <dd title={current().updated}>{relativeTime(current().updated)}</dd>
                  </dl>
                  <Show when={current().path}>
                    <button type="button" class="path-box" aria-label="Copy file path" onClick={() => void copyToClipboard(current().path ?? '', 'the file path')}>
                      <span class="truncate">{current().path}</span>
                    </button>
                  </Show>
                </Section>
              </div>

              <div class="task-toasts">
                <BoardToasts />
              </div>
              <TaskComposer task={current()} value={note()} onInput={setNote} uploading={uploading()} setUploading={setUploading} />

              <Show when={editing()}>
                <BodyEditor
                  task={current()}
                  value={body()}
                  onInput={setBody}
                  dirty={body() !== (current().body ?? '')}
                  busy={savingBody()}
                  onSave={() => void saveBody()}
                  onCancel={() => setEditing(false)}
                  uploading={uploading()}
                  setUploading={setUploading}
                />
              </Show>

              <Show when={menu()?.kind === 'more'}>
                <TaskMoreMenu task={current()} anchor={menu()!.anchor} onDismiss={closeMenu} />
              </Show>
              <Show when={menu()?.kind === 'move'}>
                <MoveMenu task={current()} anchor={menu()!.anchor} onDismiss={closeMenu} />
              </Show>
              <Show when={menu()?.kind === 'priority'}>
                <PriorityMenu task={current()} anchor={menu()!.anchor} onDismiss={closeMenu} />
              </Show>

              <BoardSheet
                open={sheet() === 'labels'}
                onClose={() => setSheet(null)}
                label="Labels"
                testId="labels-sheet"
                class="board-sheet-tall"
                footer={
                  <button type="button" class="pair-secondary" onClick={() => setSheet(null)}>
                    Done
                  </button>
                }
              >
                <LabelPicker
                  selected={current().labels ?? []}
                  onToggle={(label) => void toggleLabel(current(), label)}
                  onCreate={(label) => void addLabel(current(), label)}
                />
              </BoardSheet>
              <BoardSheet
                open={sheet() === 'deps'}
                onClose={() => setSheet(null)}
                label="Runs after"
                testId="deps-sheet"
                class="board-sheet-tall"
                footer={
                  <button type="button" class="pair-secondary" onClick={() => setSheet(null)}>
                    Done
                  </button>
                }
              >
                <p class="picker-hint">{current().id} waits until every task it runs after reaches {store.state.rules.chainGate === 'done' ? 'Done' : 'In Review'}. Tasks that would make a loop aren't listed.</p>
                <DepPicker
                  selfId={current().id}
                  picked={current().deps ?? []}
                  onToggle={(id) => void setDependency(current(), id, !(store.taskById(current().id)?.deps ?? []).includes(id))}
                />
              </BoardSheet>
            </>
          )}
        </Show>
      </div>
    </Portal>
  );
}

function Section(props: { title: string; count?: number; action?: JSX.Element; children: JSX.Element }) {
  return (
    <section class="task-section" aria-label={props.title}>
      <div class="task-section-head">
        <h3>
          {props.title}
          <Show when={props.count !== undefined}>
            <span class="task-section-count">{props.count}</span>
          </Show>
        </h3>
        {props.action}
      </div>
      {props.children}
    </section>
  );
}

function SectionAction(props: { icon: JSX.Element; label: string; text: string; onClick: (event: MouseEvent) => void; expanded?: boolean }) {
  return (
    <button type="button" class="section-action" aria-label={props.label} aria-expanded={props.expanded} onClick={(event) => props.onClick(event)}>
      {props.icon}
      {props.text}
    </button>
  );
}

/** The In Review / Blocked report, prominent, with the next steps as buttons (web banner hint → actions). */
function ReportCard(props: { task: Task }) {
  const store = boardStore();
  const ui = boardUi();
  const report = () => statusReport(props.task);
  const moves = () => statusMoves(store.state.rules, props.task);
  return (
    <Show when={report()}>
      {(current) => (
        <div class="report-card" data-kind={current().kind} role={current().kind === 'blocked' ? 'alert' : 'status'} aria-label={current().kind === 'blocked' ? 'Blocked report' : 'Review report'}>
          <div class="report-card-head">
            <StatusGlyph status={current().kind === 'blocked' ? 'blocked' : 'in_review'} size={16} />
            <strong>{current().kind === 'blocked' ? 'Blocked — the agent needs you' : 'Ready for review'}</strong>
            <span class="report-card-meta">
              {current().actor} · {relativeTime(current().at)}
            </span>
          </div>
          <div class="report-card-body">
            <Markdown text={current().text} />
          </div>
          <button
            type="button"
            class="link-button report-read"
            onClick={() =>
              ui.setReader({
                title: current().kind === 'blocked' ? `${props.task.id} · Blocked` : `${props.task.id} · Report`,
                text: current().text,
                actor: current().actor,
                at: current().at,
              })
            }
          >
            <BookOpen size={15} /> Read the full report
          </button>
          <Show when={props.task.status === current().kind || (props.task.status === 'in_review' && current().kind === 'review')}>
            <div class="report-actions">
              <Show when={props.task.status === 'in_review' && moves().includes('done')}>
                <button type="button" class="report-button primary" onClick={() => void moveTask(props.task, 'done', undefined, { confirmed: true })}>
                  <Check size={17} /> Mark done
                </button>
              </Show>
              <Show when={props.task.status === 'in_review' && moves().includes('todo')}>
                <button type="button" class="report-button" onClick={() => ui.setComment({ task: props.task, to: 'todo', hint: '', optional: true })}>
                  <RotateCcw size={16} /> Rework…
                </button>
              </Show>
              <Show when={props.task.status === 'blocked' && moves().includes('todo')}>
                <button type="button" class="report-button primary" onClick={() => ui.setComment({ task: props.task, to: 'todo', hint: '', optional: true })}>
                  <RotateCcw size={16} /> Answer & move to Todo
                </button>
              </Show>
            </div>
          </Show>
        </div>
      )}
    </Show>
  );
}

/** The claim: who runs it, since when; a stale run can be released (needs a note). */
function RunCard(props: { task: Task }) {
  const store = boardStore();
  const claim = () => claimState(props.task);
  const releases = () => (['todo', 'backlog'] as const).filter((to) => statusMoves(store.state.rules, props.task).includes(to));
  return (
    <Show when={props.task.run}>
      {(run) => (
        <div class="run-card" data-claim={claim()}>
          <div class="run-card-head">
            <span class="agent-chip">
              <span class="pulse" aria-hidden="true" />
              {claim() === 'stale' ? 'Stale run' : claim() === 'unknown' ? 'Claimed on another host' : 'Agent running'}
            </span>
            <span class="run-card-time tabular-nums">{elapsed(run().started, now())}</span>
          </div>
          <dl class="run-grid">
            <dt>session</dt>
            <dd class="font-mono truncate">{run().session ?? '—'}</dd>
            <dt>mode</dt>
            <dd>{run().mode ?? 'direct'}</dd>
            <dt>host</dt>
            <dd>{run().host ?? '—'}</dd>
            <dt>pid</dt>
            <dd class="tabular-nums">{run().pid ?? '—'}</dd>
            <Show when={run().goal}>
              <dt>goal</dt>
              <dd>{run().goal}</dd>
            </Show>
          </dl>
          <p class="run-note">{claimNote(props.task)}</p>
          <Show when={claim() === 'stale' && releases().length > 0}>
            <div class="report-actions">
              <For each={releases()}>
                {(to, index) => (
                  <button type="button" class={`report-button${index() === 0 ? ' primary' : ''}`} onClick={() => void moveTask(props.task, to)}>
                    <StatusGlyph status={to} size={15} /> Release to {laneLabel(to)}
                  </button>
                )}
              </For>
            </div>
          </Show>
        </div>
      )}
    </Show>
  );
}

function depStateOf(task: Task, dep: string, exists: boolean): { kind: string; text: string } {
  if (!exists) return { kind: 'missing', text: 'missing' };
  if ((task.lockedBy ?? []).includes(dep)) return { kind: 'locked', text: 'in Backlog' };
  if ((task.waitingFor ?? []).includes(dep)) return { kind: 'waiting', text: 'waiting' };
  return { kind: 'ready', text: 'ready' };
}

function DepsSection(props: { task: Task; onAdd: () => void }) {
  const store = boardStore();
  const ui = boardUi();
  const [scheduling, setScheduling] = createSignal(false);
  const open = (id: string) => store.taskById(id) && ui.openTask(id, true);
  return (
    <Section
      title="Runs after"
      count={(props.task.deps ?? []).length || undefined}
      action={<SectionAction icon={<Plus size={15} />} label="Add dependency" text="Add" onClick={props.onAdd} />}
    >
      <Show when={(props.task.lockedBy ?? []).length > 0}>
        <div class="lock-note">
          <Lock size={15} class="shrink-0 mt-0.5" />
          <span class="flex-1">
            Locked — {(props.task.lockedBy ?? []).join(', ')} {(props.task.lockedBy ?? []).length === 1 ? 'is' : 'are'} still in Backlog, and Backlog work is never picked up.
          </span>
          <button
            type="button"
            class="report-button"
            disabled={scheduling()}
            onClick={async () => {
              const ctx = boardCtx();
              if (!ctx) return;
              setScheduling(true);
              await scheduleDeps(ctx, [...(props.task.lockedBy ?? [])]);
              setScheduling(false);
            }}
          >
            Move to Todo
          </button>
        </div>
      </Show>
      <div class="dep-list">
        <For each={props.task.deps ?? []} fallback={<span class="section-empty">Runs whenever an agent is free</span>}>
          {(dep) => {
            const depTask = () => store.taskById(dep);
            const state = () => depStateOf(props.task, dep, !!depTask());
            return (
              <div class="dep-row">
                <button type="button" class="dep-open" aria-label={`Open ${dep}`} disabled={!depTask()} onClick={() => open(dep)}>
                  <StatusGlyph status={depTask()?.status ?? 'backlog'} size={16} />
                  <span class="bcard-id">{dep}</span>
                  <span class="min-w-0 flex-1 truncate" classList={{ derived: !depTask()?.title }}>
                    {depTask() ? displayTitle(depTask()!) : 'missing task'}
                  </span>
                  <span class="dep-state" data-state={state().kind}>
                    {state().text}
                  </span>
                </button>
                <button type="button" class="dep-unlink" aria-label={`Remove dependency ${dep}`} onClick={() => void setDependency(props.task, dep, false)}>
                  <X size={16} />
                </button>
              </div>
            );
          }}
        </For>
      </div>
    </Section>
  );
}

function DependentsSection(props: { task: Task }) {
  const store = boardStore();
  const ui = boardUi();
  const dependents = () => dependentsOf(store.state.tasks, props.task.id);
  return (
    <Show when={dependents().length > 0}>
      <Section title="Needed by" count={dependents().length}>
        <div class="dep-list">
          <For each={dependents()}>
            {(other) => (
              <div class="dep-row">
                <button type="button" class="dep-open" aria-label={`Open ${other.id}`} onClick={() => ui.openTask(other.id, true)}>
                  <StatusGlyph status={other.status} size={16} />
                  <span class="bcard-id">{other.id}</span>
                  <span class="min-w-0 flex-1 truncate" classList={{ derived: !other.title }}>
                    {displayTitle(other)}
                  </span>
                  <Link2 size={14} class="text-text-dim shrink-0" />
                </button>
              </div>
            )}
          </For>
        </div>
      </Section>
    </Show>
  );
}

function Thumb(props: { task: Task; item: Attachment; onOpen: () => void }) {
  const store = boardStore();
  const [url, setUrl] = createSignal<string | undefined>();
  createEffect(() => {
    const hostId = store.target()?.hostId;
    if (!hostId) return;
    api
      .fileUrl(hostId, taskFilePath(props.task.id, props.item.name))
      .then(setUrl)
      .catch(() => setUrl(undefined));
  });
  return (
    <button type="button" class="att-thumb" aria-label={`View ${props.item.original}`} onClick={props.onOpen}>
      <Show when={url()} fallback={<span class="att-thumb-skeleton skeleton" />}>
        <img src={url()} alt={props.item.original} loading="lazy" />
      </Show>
    </button>
  );
}

function AttachmentsSection(props: {
  task: Task;
  onAdd: (event: MouseEvent) => void;
  uploading: number;
  setUploading: (fn: (count: number) => number) => void;
  menuOpen: boolean;
  anchor: DOMRect | undefined;
  onCloseMenu: () => void;
}) {
  const items = () => props.task.attachments ?? [];
  const images = () => items().filter((item) => item.kind === 'image');
  const files = () => items().filter((item) => item.kind !== 'image');
  const viewerItems = () => images().map((item) => ({ path: taskFilePath(props.task.id, item.name), name: item.original, size: item.size }));
  async function add(list: File[]) {
    props.setUploading((count) => count + list.length);
    try {
      const done = await uploadToTask(props.task, list);
      if (done.length > 0) boardUi().toast(`Attached ${done.map((item) => item.name.replace(/^[0-9a-f]{8}-/, '')).join(', ')}`, 'success');
    } finally {
      props.setUploading((count) => Math.max(0, count - list.length));
    }
  }
  const pickers = createFilePickers((list) => void add(list), 'Attach to task');
  return (
    <>
      {pickers.element}
      <Show when={props.anchor}>{(anchor) => <AttachMenu anchor={anchor()} alignEnd onDismiss={props.onCloseMenu} onPick={pickers.pick} />}</Show>
      <Show when={items().length > 0 || props.uploading > 0}>
    <Section
      title="Attachments"
      count={items().length || undefined}
      action={<SectionAction icon={<Plus size={15} />} label="Add attachment" text="Add" expanded={props.menuOpen} onClick={props.onAdd} />}
    >
      <Show when={items().length > 0 || props.uploading > 0}>
        <Show when={images().length > 0}>
          <div class="att-grid">
            <For each={images()}>
              {(item, index) => (
                <Thumb
                  task={props.task}
                  item={item}
                  onOpen={() => boardUi().setViewer({ items: viewerItems(), index: index() })}
                />
              )}
            </For>
          </div>
        </Show>
        <div class="att-files">
          <For each={files()}>
            {(item) => (
              <button
                type="button"
                class="att-file"
                aria-label={`Open ${item.original}`}
                onClick={() => void openFile(taskFilePath(props.task.id, item.name), item.name, [], item.size)}
              >
                <span class={`att-badge att-${attKind(item.name)}`}>{(item.original.split('.').pop() ?? 'file').slice(0, 4).toUpperCase()}</span>
                <span class="min-w-0 flex-1 truncate">{item.original}</span>
                <span class="text-text-dim text-[13px] tabular-nums">{formatBytes(item.size)}</span>
              </button>
            )}
          </For>
          <Show when={props.uploading > 0}>
            <div class="att-file uploading">
              <span class="spinner" /> Uploading {props.uploading}…
            </div>
          </Show>
        </div>
      </Show>
    </Section>
      </Show>
    </>
  );
}
