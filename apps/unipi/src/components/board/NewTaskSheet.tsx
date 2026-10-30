/**
 * New task (UNI-130) — the web UI's NewTaskDialog as a bottom sheet:
 * title (optional), description (markdown, `@` mentions a task), status
 * (Backlog | Todo), priority, labels (search / create picker) and "runs
 * after" dependencies (search picker, newest first, no cycles), "Create
 * more", the web's validation (a title or a description is required)
 * and draft protection (closing a dirty form asks first). Status and
 * priority are one-tap segmented rows instead of popovers; the two
 * pickers open as drill-in pages inside the sheet (Back returns to the
 * form). `recreateFrom` prefills the creation fields (UNI-57 Recreate).
 */
import { batch, createEffect, createMemo, createSignal, For, on, Show } from 'solid-js';
import { ArrowLeft, Check, Link2, Paperclip, Plus, Search, Tag, X } from 'lucide-solid';
import { api, describe, displayTitle, laneLabel, PRIORITIES, PRIORITY_LABEL } from '../../lib/board/api';
import { allLabels, depCandidates, hue, mentionAt, mentionCandidates } from '../../lib/board/view';
import { boardStore } from '../../lib/board/store';
import { boardUi } from '../../lib/board/ui';
import { boardActions } from '../../lib/board/actions';
import { formatBytes, namedFile, uploadAll } from '../../lib/board/attach';
import { createFileDrop, isSubmitChord } from '../../lib/board/fileDrop';
import { BoardSheet } from './BoardSheet';
import { PriorityGlyph, StatusGlyph } from './Glyphs';
import { BoardSwitch } from './FilterDisplaySheets';
import { setFieldValue } from '../../lib/fieldEdit';

type Page = 'form' | 'labels' | 'deps';

/** Focus a picker's search field once the page swap settled (the drawer's
 * focus trap re-homes focus when the tapped "Add …" button unmounts).
 * Not on touch screens: picking is mostly tapping, and a keyboard popping
 * up would cover half the list — tap the field to type or create. */
const focusSoon = (el: HTMLInputElement) => {
  if (typeof matchMedia === 'function' && matchMedia('(pointer: coarse)').matches) return;
  setTimeout(() => el.focus({ preventScroll: true }), 160);
};

/** Tapping a picker row must not blur the search field (keeps the soft keyboard up). */
const keepFocus = (event: PointerEvent | MouseEvent) => {
  if (document.activeElement instanceof HTMLInputElement) event.preventDefault();
};

export function NewTaskSheet(props: { onHaptic?: () => void }) {
  const store = boardStore();
  const ui = boardUi();
  const [title, setTitle] = createSignal('');
  const [body, setBody] = createSignal('');
  const [lane, setLane] = createSignal<'backlog' | 'todo'>('backlog');
  const [initialLane, setInitialLane] = createSignal<'backlog' | 'todo'>('backlog');
  const [priority, setPriority] = createSignal('none');
  const [after, setAfter] = createSignal<string[]>([]);
  const [labels, setLabels] = createSignal<string[]>([]);
  const [more, setMore] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | undefined>();
  const [confirmDiscard, setConfirmDiscard] = createSignal(false);
  const [page, setPage] = createSignal<Page>('form');
  const [mention, setMention] = createSignal<{ start: number; needle: string } | null>(null);
  /** Files picked before the task exists — uploaded right after creation (web parity). */
  const [pending, setPending] = createSignal<File[]>([]);
  let titleInput: HTMLTextAreaElement | undefined;
  /** The title wraps instead of scrolling sideways on narrow phones: grow to fit (max 4 lines). */
  const fitTitle = (el: HTMLTextAreaElement | undefined) => {
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight, 4 * 28 + 8)}px`;
  };
  // Re-fit when the title changes from outside the field (Create more clears it, Recreate prefills it).
  createEffect(on(title, () => queueMicrotask(() => fitTitle(titleInput))));
  let filePicker: HTMLInputElement | undefined;
  let bodyArea: HTMLTextAreaElement | undefined;

  const open = () => ui.newTask() !== null;
  const recreate = () => ui.newTask()?.recreateFrom;
  const valid = () => title().trim().length > 0 || body().trim().length > 0;
  /** A draft worth protecting: any content, choice or changed lane (web UNI-61). */
  const dirty = () =>
    title().length > 0 ||
    body().length > 0 ||
    after().length > 0 ||
    labels().length > 0 ||
    priority() !== 'none' ||
    lane() !== initialLane() ||
    pending().length > 0;

  createEffect(
    on(ui.newTask, (request) => {
      if (!request) return;
      batch(() => {
        const source = request.recreateFrom;
        if (source) {
          setLane('todo');
          setTitle(source.title);
          setBody(source.body ?? '');
          setPriority(source.priority && source.priority !== 'none' ? source.priority : 'none');
          setAfter([...(source.deps ?? [])]);
          setLabels([...(source.labels ?? [])]);
        } else {
          setLane(request.lane === 'todo' ? 'todo' : 'backlog');
          setTitle('');
          setBody('');
          setAfter([]);
          setLabels([]);
          setPriority('none');
        }
        setInitialLane(lane());
        setConfirmDiscard(false);
        setError(undefined);
        setPage('form');
        setMention(null);
        setPending([]);
      });
      setTimeout(() => titleInput?.focus({ preventScroll: true }), 80);
    }),
  );

  const close = () => {
    setConfirmDiscard(false);
    ui.closeNewTask();
  };
  /** Back / swipe / ✕: a picker page goes back to the form; a dirty form asks first. */
  const requestClose = () => {
    if (page() !== 'form') return setPage('form');
    if (confirmDiscard()) return close();
    if (dirty()) return setConfirmDiscard(true);
    close();
  };

  const bodyDrop = createFileDrop((files) => setPending((current) => [...current, ...files.map((file) => namedFile(file))]));

  async function submit() {
    const at = store.target();
    if (!at?.slug || !valid() || busy()) return;
    setBusy(true);
    setError(undefined);
    try {
      const created = await api.create(at.hostId, at.slug, {
        title: title().trim(),
        body: body().trim() || undefined,
        status: lane(),
        priority: priority(),
        after: after().length > 0 ? after() : undefined,
        labels: labels().length > 0 ? labels() : undefined,
      });
      store.applyTask(created);
      if (pending().length > 0) {
        const failed: string[] = [];
        const uploaded = await uploadAll(at.hostId, at.slug, created.id, pending(), (message) => failed.push(message));
        if (uploaded.length > 0) {
          const withFiles = [body().trim(), uploaded.map((item) => item.markdown).join('\n')].filter(Boolean).join('\n\n');
          store.applyTask(await api.edit(at.hostId, at.slug, created.id, { body: withFiles }));
        }
        if (failed.length > 0) ui.toast(`${created.id}: ${failed.length} file${failed.length === 1 ? '' : 's'} failed — ${failed[0]}`, 'warning');
        setPending([]);
      }
      void store.reload();
      props.onHaptic?.();
      ui.toast(`Created ${created.id}`, 'success', { label: 'Open', run: () => boardActions().openTask(store.taskById(created.id) ?? created) });
      if (more()) {
        batch(() => {
          setTitle('');
          setBody('');
          setLabels([]);
          setPending([]);
        });
        titleInput?.focus({ preventScroll: true });
      } else close();
    } catch (cause) {
      setError(describe(cause));
    } finally {
      setBusy(false);
    }
  }

  const toggleIn = (setter: (fn: (current: string[]) => string[]) => void, value: string) =>
    setter((current) => (current.includes(value) ? current.filter((item) => item !== value) : [...current, value]));

  function onBodyInput(event: InputEvent & { currentTarget: HTMLTextAreaElement }) {
    const area = event.currentTarget;
    setBody(area.value);
    fit(area);
    setMention(mentionAt(area.value, area.selectionStart ?? area.value.length));
  }
  function fit(area: HTMLTextAreaElement) {
    area.style.height = 'auto';
    area.style.height = `${Math.min(area.scrollHeight, 220)}px`;
  }
  function insertMention(id: string) {
    const current = mention();
    const area = bodyArea;
    if (!current || !area) return;
    const caret = area.selectionStart ?? area.value.length;
    const next = `${area.value.slice(0, current.start)}${id} ${area.value.slice(caret)}`;
    const position = current.start + id.length + 1;
    setFieldValue(area, next, position);
    setBody(next);
    area.focus();
    setMention(null);
    fit(area);
  }
  const mentionList = createMemo(() => {
    const current = mention();
    return current ? mentionCandidates(store.state.tasks, current.needle) : [];
  });

  const headerTitle = () =>
    page() === 'labels' ? 'Labels' : page() === 'deps' ? 'Runs after' : recreate() ? 'Recreate task' : 'New task';

  return (
    <BoardSheet
      open={open()}
      onClose={requestClose}
      label={headerTitle()}
      testId="new-task-sheet"
      guarded={dirty() && page() === 'form'}
      class="board-sheet-tall new-task-sheet"
      title={
        <span class="flex items-center gap-2 min-w-0">
          <Show when={page() !== 'form'}>
            <button type="button" class="ghost-icon-button -ml-2" aria-label="Back to the form" onClick={() => setPage('form')}>
              <ArrowLeft size={20} />
            </button>
          </Show>
          <span class="truncate">{headerTitle()}</span>
          <Show when={page() === 'form'}>
            <span class="new-task-project truncate">{store.currentProject()?.name ?? store.target()?.slug}</span>
          </Show>
        </span>
      }
      headerAction={
        <button type="button" class="ghost-icon-button" aria-label={page() === 'form' ? 'Close' : 'Done'} onClick={() => (page() === 'form' ? requestClose() : setPage('form'))}>
          <Show when={page() === 'form'} fallback={<Check size={20} />}>
            <X size={20} />
          </Show>
        </button>
      }
      footer={
        <Show
          when={page() === 'form'}
          fallback={
            <button type="button" class="pair-secondary" onClick={() => setPage('form')}>
              Done
            </button>
          }
        >
          <Show
            when={!confirmDiscard()}
            fallback={
              <div class="discard-strip" role="alertdialog" aria-label="Discard draft?">
                <span class="flex-1 text-[15px]">Discard this draft?</span>
                <button type="button" class="board-text-button" onClick={() => setConfirmDiscard(false)}>
                  Keep editing
                </button>
                <button type="button" class="discard-button" onClick={close}>
                  Discard
                </button>
              </div>
            }
          >
            <div class="new-task-foot">
              <div class="flex items-center gap-2 text-[14px] text-text-muted whitespace-nowrap">
                <BoardSwitch label="Create more" checked={more()} onChange={setMore} />
                <span>Create more</span>
              </div>
              <button type="button" class="pair-primary new-task-create" disabled={!valid() || busy()} onClick={() => void submit()}>
                {busy() ? 'Creating…' : 'Create task'}
              </button>
            </div>
          </Show>
        </Show>
      }
    >
      <Show when={page() === 'form'}>
        <Show when={recreate()}>
          {(source) => (
            <p class="recreate-banner">
              Recreating <b class="font-mono">{source().id}</b> — this makes a new task; notes, history and status stay behind.
            </p>
          )}
        </Show>
        <textarea
          rows={1}
          ref={titleInput}
          class="new-task-title"
          placeholder="Title (optional)"
          aria-label="Title (optional)"
          enterkeyhint="done"
          value={title()}
          onInput={(event) => {
            setTitle(event.currentTarget.value.replace(/\n/g, ' '));
            fitTitle(event.currentTarget);
          }}
          onKeyDown={(event) => {
            if (event.key === 'Enter') {
              event.preventDefault();
              void submit();
            }
          }}
        />
        <textarea
          ref={bodyArea}
          class="new-task-body"
          rows={3}
          placeholder="Add a description… (markdown, @ to mention a task)"
          aria-label="Description"
          value={body()}
          onInput={onBodyInput}
          classList={{ 'drop-over': bodyDrop.over() }}
          {...bodyDrop.handlers}
          onKeyDown={(event) => {
            if (isSubmitChord(event)) {
              event.preventDefault();
              void submit();
            }
          }}
          onPaste={(event) => {
            const files = [...(event.clipboardData?.files ?? [])];
            if (files.length === 0) return;
            event.preventDefault();
            setPending((current) => [...current, ...files.map((file) => namedFile(file))]);
          }}
          onBlur={() => setTimeout(() => setMention(null), 150)}
        />
        <Show when={mentionList().length > 0}>
          <div class="mention-strip" role="listbox" aria-label="Mention a task">
            <For each={mentionList()}>
              {(task) => (
                <button type="button" role="option" class="mention-chip" onPointerDown={(event) => event.preventDefault()} onMouseDown={(event) => event.preventDefault()} onClick={() => insertMention(task.id)}>
                  <StatusGlyph status={task.status} size={12} />
                  <span class="font-mono">{task.id}</span>
                  <span class="truncate">{displayTitle(task)}</span>
                </button>
              )}
            </For>
          </div>
        </Show>

        <div class="prop-row">
          <span class="prop-label" id="nt-status">Status</span>
          <div class="seg" role="radiogroup" aria-labelledby="nt-status">
            <For each={['backlog', 'todo'] as const}>
              {(status) => (
                <button type="button" role="radio" aria-checked={lane() === status} class="seg-option" onClick={() => setLane(status)}>
                  <StatusGlyph status={status} size={14} />
                  {laneLabel(status)}
                </button>
              )}
            </For>
          </div>
        </div>
        <div class="prop-row">
          <span class="prop-label" id="nt-priority">Priority</span>
          <div class="seg seg-5" role="radiogroup" aria-labelledby="nt-priority">
            <For each={[...PRIORITIES]}>
              {(option) => (
                <button
                  type="button"
                  role="radio"
                  aria-checked={priority() === option}
                  aria-label={PRIORITY_LABEL[option]}
                  class="seg-option"
                  onClick={() => setPriority(option)}
                >
                  <PriorityGlyph priority={option} size={14} />
                  {option === 'none' ? 'None' : PRIORITY_LABEL[option]}
                </button>
              )}
            </For>
          </div>
        </div>
        <div class="prop-row">
          <span class="prop-label">Labels</span>
          <div class="prop-chips">
            <For each={labels()}>
              {(label) => (
                <span class="btag label removable" style={{ '--label-hue': hue(label) }}>
                  <span>{label}</span>
                  <button type="button" aria-label={`Remove ${label}`} onClick={() => toggleIn(setLabels, label)}>
                    <X size={12} />
                  </button>
                </span>
              )}
            </For>
            <button type="button" class="prop-add" onClick={() => setPage('labels')}>
              <Tag size={14} /> {labels().length ? 'Edit' : 'Add labels'}
            </button>
          </div>
        </div>
        <div class="prop-row">
          <span class="prop-label">Runs after</span>
          <div class="prop-chips">
            <For each={after()}>
              {(id) => (
                <span class="btag dep removable">
                  <span class="font-mono">{id}</span>
                  <button type="button" aria-label={`Remove ${id}`} onClick={() => toggleIn(setAfter, id)}>
                    <X size={12} />
                  </button>
                </span>
              )}
            </For>
            <button type="button" class="prop-add" onClick={() => setPage('deps')}>
              <Link2 size={14} /> {after().length ? 'Edit' : 'Add dependency'}
            </button>
          </div>
        </div>
        <div class="prop-row">
          <span class="prop-label">Attachments</span>
          <div class="prop-chips">
            <For each={pending()}>
              {(file, index) => (
                <span class="btag removable attach-chip">
                  <Paperclip size={12} />
                  <span class="truncate">{file.name}</span>
                  <span class="text-text-dim">{formatBytes(file.size)}</span>
                  <button type="button" aria-label={`Remove ${file.name}`} onClick={() => setPending((current) => current.filter((_, at) => at !== index()))}>
                    <X size={12} />
                  </button>
                </span>
              )}
            </For>
            <button type="button" class="prop-add" onClick={() => filePicker?.click()}>
              <Paperclip size={14} /> Attach
            </button>
            <input
              ref={filePicker}
              type="file"
              multiple
              hidden
              aria-label="Attach files"
              onChange={(event) => {
                const files = [...(event.currentTarget.files ?? [])].map((file) => namedFile(file));
                if (files.length > 0) setPending((current) => [...current, ...files]);
                event.currentTarget.value = '';
              }}
            />
          </div>
        </div>
        <Show when={error()}>
          <p class="text-sm text-wm-red mt-3" role="alert">
            {error()}
          </p>
        </Show>
      </Show>

      <Show when={page() === 'labels'}>
        <LabelPicker
          selected={labels()}
          onToggle={(label) => toggleIn(setLabels, label)}
          onCreate={(raw) => {
            const label = raw.trim().replace(/,/g, '');
            if (label && !labels().includes(label)) setLabels((current) => [...current, label]);
          }}
        />
      </Show>
      <Show when={page() === 'deps'}>
        <DepPicker picked={after()} onToggle={(id) => toggleIn(setAfter, id)} />
      </Show>
    </BoardSheet>
  );
}

/** Searchable create/select label multiselect (web label-picker.tsx). */
export function LabelPicker(props: { selected: string[]; onToggle: (label: string) => void; onCreate: (label: string) => void }) {
  const store = boardStore();
  const [needle, setNeedle] = createSignal('');
  const known = () => allLabels(store.state.tasks);
  const options = createMemo(() => {
    const n = needle().trim().toLowerCase();
    return known().filter((label) => !n || label.toLowerCase().includes(n));
  });
  const typed = () => needle().trim().replace(/,/g, '');
  const exists = () =>
    options().some((label) => label.toLowerCase() === typed().toLowerCase()) ||
    props.selected.some((label) => label.toLowerCase() === typed().toLowerCase());
  const accept = () => {
    const label = typed();
    if (!label) return;
    if (exists()) props.onToggle(known().find((item) => item.toLowerCase() === label.toLowerCase()) ?? label);
    else props.onCreate(label);
    setNeedle('');
  };
  return (
    <div class="picker-page">
      <div class="search-field">
        <Search size={18} class="text-text-dim shrink-0" />
        <input
          ref={focusSoon}
          placeholder="Filter or create a label"
          aria-label="Filter labels"
          enterkeyhint="done"
          value={needle()}
          onInput={(event) => setNeedle(event.currentTarget.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter') {
              event.preventDefault();
              accept();
            }
          }}
        />
      </div>
      <Show when={typed() && !exists()}>
        <button type="button" class="search-row" onPointerDown={keepFocus} onMouseDown={keepFocus} onClick={accept}>
          <Plus size={18} class="board-accent" />
          <span class="min-w-0 flex-1 truncate">
            Create <b>{typed()}</b>
          </span>
        </button>
      </Show>
      <div class="search-results" role="listbox" aria-label="Labels">
        <For each={props.selected.filter((label) => !options().includes(label))}>
          {(label) => <CheckRow label={label} checked onClick={() => props.onToggle(label)} dot={hue(label)} />}
        </For>
        <For each={options()} fallback={<p class="text-sm text-text-dim py-3">No existing labels.</p>}>
          {(label) => <CheckRow label={label} checked={props.selected.includes(label)} onClick={() => props.onToggle(label)} dot={hue(label)} />}
        </For>
      </div>
    </div>
  );
}

function CheckRow(props: { label: string; checked: boolean; onClick: () => void; dot: string }) {
  return (
    <button type="button" role="option" aria-selected={props.checked} class="search-row" onPointerDown={keepFocus} onMouseDown={keepFocus} onClick={props.onClick}>
      <span class="label-dot" style={{ background: props.dot }} />
      <span class="min-w-0 flex-1 truncate">{props.label}</span>
      <span class="check-box" data-checked={props.checked ? '' : undefined}>
        <Show when={props.checked}>
          <Check size={14} />
        </Show>
      </span>
    </button>
  );
}

/** "Runs after" multiselect (web dep-picker.tsx): newest first, capped at 50. */
export function DepPicker(props: { picked: string[]; onToggle: (id: string) => void; selfId?: string }) {
  const store = boardStore();
  const [needle, setNeedle] = createSignal('');
  const list = () => depCandidates(store.state.tasks, needle(), new Set(), props.selfId);
  return (
    <div class="picker-page">
      <div class="search-field">
        <Search size={18} class="text-text-dim shrink-0" />
        <input
          ref={focusSoon}
          placeholder="Search tasks (id or title)"
          aria-label="Search tasks to depend on"
          value={needle()}
          onInput={(event) => setNeedle(event.currentTarget.value)}
        />
      </div>
      <div class="search-results" role="listbox" aria-label="Runs after">
        <For each={list().shown} fallback={<p class="text-sm text-text-dim py-3">No matching tasks.</p>}>
          {(task) => (
            <button
              type="button"
              role="option"
              aria-selected={props.picked.includes(task.id)}
              class="search-row"
              onPointerDown={keepFocus} onMouseDown={keepFocus}
              onClick={() => props.onToggle(task.id)}
            >
              <StatusGlyph status={task.status} size={16} />
              <span class="bcard-id shrink-0">{task.id}</span>
              <span class="min-w-0 flex-1 truncate" classList={{ derived: !task.title }}>
                {displayTitle(task)}
              </span>
              <span class="check-box" data-checked={props.picked.includes(task.id) ? '' : undefined}>
                <Show when={props.picked.includes(task.id)}>
                  <Check size={14} />
                </Show>
              </span>
            </button>
          )}
        </For>
        <Show when={list().more > 0}>
          <p class="text-xs text-text-dim py-2 text-center">{list().more} more — keep typing</p>
        </Show>
      </div>
    </div>
  );
}
