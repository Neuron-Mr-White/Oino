/**
 * Description editor (UNI-131). The web edits the body inline in a
 * growing textarea; on a phone that leaves a few lines between the
 * header and the keyboard. Here editing takes the whole task page: a
 * Write | Preview switch, a field that fills the space above the
 * keyboard, and a markdown key row docked on the keyboard (bold,
 * italic, code, list, checklist, link, `@` mention, attach photo/file)
 * so nobody has to hunt for `*` and `` ` `` on a soft keyboard.
 * Cancel with changes asks first (drafts are protected).
 */
import { createSignal, onCleanup, onMount, Show } from 'solid-js';
import { AtSign, Bold, Code, Heading2, Italic, Link2, List, ListChecks, Paperclip, Quote } from 'lucide-solid';
import type { Task } from '../../lib/board/api';
import { insertAtCursor } from '../../lib/board/attach';
import { uploadToTask } from '../../lib/board/taskActions';
import { createFileDrop, isSubmitChord } from '../../lib/board/fileDrop';
import { pushBack } from '../../lib/back';
import { createMention, MentionStrip } from './Mention';
import { Markdown } from './Markdown';
import { AttachMenu, createFilePickers } from './TaskComposer';
import { setFieldValue } from '../../lib/fieldEdit';

/** Wrap the selection (or insert a placeholder) with `before`/`after`; returns [text, selStart, selEnd]. */
export function wrapSelection(text: string, start: number, end: number, before: string, after = before, placeholder = 'text'): [string, number, number] {
  const selected = text.slice(start, end) || placeholder;
  const next = `${text.slice(0, start)}${before}${selected}${after}${text.slice(end)}`;
  return [next, start + before.length, start + before.length + selected.length];
}

/** Prefix every selected line (or the caret's line) with `prefix`. */
export function prefixLines(text: string, start: number, end: number, prefix: string): [string, number, number] {
  const lineStart = text.lastIndexOf('\n', start - 1) + 1;
  const lineEnd = text.indexOf('\n', end) === -1 ? text.length : text.indexOf('\n', end);
  const block = text.slice(lineStart, lineEnd);
  const lines = block.split('\n').map((line) => (line.startsWith(prefix) ? line : `${prefix}${line}`));
  const replaced = lines.join('\n');
  const next = `${text.slice(0, lineStart)}${replaced}${text.slice(lineEnd)}`;
  return [next, lineStart + replaced.length, lineStart + replaced.length];
}

export function BodyEditor(props: {
  task: Task;
  value: string;
  onInput: (value: string) => void;
  dirty: boolean;
  busy: boolean;
  onSave: () => void;
  onCancel: () => void;
  uploading: number;
  setUploading: (fn: (count: number) => number) => void;
}) {
  const [tab, setTab] = createSignal<'write' | 'preview'>('write');
  const [confirm, setConfirm] = createSignal(false);
  const [menu, setMenu] = createSignal<DOMRect | undefined>();
  let area: HTMLTextAreaElement | undefined;
  const mention = createMention(() => area, props.onInput, () => props.task.id);

  const cancel = () => {
    if (confirm() || !props.dirty) return props.onCancel();
    setConfirm(true);
  };
  onMount(() => {
    onCleanup(pushBack(cancel));
    setTimeout(() => {
      area?.focus({ preventScroll: true });
      const end = area?.value.length ?? 0;
      area?.setSelectionRange(end, end);
    }, 120);
  });

  const apply = ([next, start, end]: [string, number, number]) => {
    // UNI-270: toolbar edits are undoable steps of the field's own history.
    if (area) setFieldValue(area, next, start, end);
    props.onInput(next);
    area?.focus();
  };
  const sel = () => [area?.selectionStart ?? props.value.length, area?.selectionEnd ?? props.value.length] as const;
  const wrap = (before: string, after = before, placeholder?: string) => () => apply(wrapSelection(props.value, ...sel(), before, after, placeholder));
  const prefix = (text: string) => () => apply(prefixLines(props.value, ...sel(), text));
  const link = () => {
    const [start, end] = sel();
    const label = props.value.slice(start, end) || 'link';
    const next = `${props.value.slice(0, start)}[${label}](https://)${props.value.slice(end)}`;
    const urlAt = start + label.length + 3;
    apply([next, urlAt, urlAt + 8]);
  };
  const at = () => {
    const [start, end] = sel();
    const lead = start > 0 && !/\s/.test(props.value[start - 1]!) ? ' ' : '';
    const next = `${props.value.slice(0, start)}${lead}@${props.value.slice(end)}`;
    const caret = start + lead.length + 1;
    apply([next, caret, caret]);
    mention.update();
  };
  async function attach(files: File[]) {
    props.setUploading((count) => count + files.length);
    try {
      const done = await uploadToTask(props.task, files);
      if (done.length > 0) {
        const next = insertAtCursor(area, props.value, done.map((item) => item.markdown).join('\n'));
        if (area) setFieldValue(area, next);
        props.onInput(next);
      }
    } finally {
      props.setUploading((count) => Math.max(0, count - files.length));
    }
  }
  const pickers = createFilePickers((files) => void attach(files), 'Attach to description');
  const drop = createFileDrop((files) => void attach(files));
  /** Keys never take focus from the field (the keyboard stays up). */
  const keep = (event: Event) => event.preventDefault();

  return (
    <div class="body-editor" role="dialog" aria-label="Edit description" data-testid="body-editor">
      <header class="task-head">
        <button type="button" class="board-text-button muted" onClick={cancel}>
          Cancel
        </button>
        <span class="flex-1 text-center text-[16px] font-semibold">Description</span>
        <button type="button" class="board-text-button" disabled={props.busy || props.uploading > 0} onClick={() => props.onSave()}>
          {props.busy ? 'Saving…' : 'Save'}
        </button>
      </header>
      <Show when={confirm()}>
        <div class="discard-strip page-strip" role="alertdialog" aria-label="Discard changes?">
          <span class="flex-1 text-[15px]">Discard your changes?</span>
          <button type="button" class="board-text-button" onClick={() => setConfirm(false)}>
            Keep editing
          </button>
          <button type="button" class="discard-button" onClick={() => props.onCancel()}>
            Discard
          </button>
        </div>
      </Show>
      <div class="editor-tabs">
        <div class="seg" role="radiogroup" aria-label="Editor mode">
          <button type="button" role="radio" class="seg-option" aria-checked={tab() === 'write'} onClick={() => setTab('write')}>
            Write
          </button>
          <button type="button" role="radio" class="seg-option" aria-checked={tab() === 'preview'} onClick={() => setTab('preview')}>
            Preview
          </button>
        </div>
        <Show when={props.uploading > 0}>
          <span class="uploading">
            <span class="spinner" /> Uploading {props.uploading}…
          </span>
        </Show>
      </div>
      <Show
        when={tab() === 'write'}
        fallback={
          <div class="editor-preview">
            <Markdown text={props.value} empty="Nothing to preview yet." />
          </div>
        }
      >
        <textarea
          ref={area}
          class="editor-area"
          aria-label="Description (markdown)"
          placeholder="Describe the task… Markdown works: **bold**, - lists, - [ ] checklists, `code`. @ mentions a task."
          value={props.value}
          onInput={(event) => {
            props.onInput(event.currentTarget.value);
            mention.update();
          }}
          onClick={() => mention.update()}
          classList={{ 'drop-over': drop.over() }}
          {...drop.handlers}
          onKeyDown={(event) => {
            if (isSubmitChord(event) && !props.busy && props.uploading === 0) {
              event.preventDefault();
              props.onSave();
            }
          }}
          onPaste={(event) => {
            const files = [...(event.clipboardData?.files ?? [])];
            if (files.length === 0) return;
            event.preventDefault();
            void attach(files);
          }}
        />
      </Show>
      <div class="editor-keys-wrap">
        <MentionStrip mention={mention} />
        <div class="editor-keys" role="toolbar" aria-label="Formatting">
          <button type="button" class="editor-key" aria-label="Bold" onPointerDown={keep} onMouseDown={keep} onClick={wrap('**', '**', 'bold')} disabled={tab() !== 'write'}>
            <Bold size={18} />
          </button>
          <button type="button" class="editor-key" aria-label="Italic" onPointerDown={keep} onMouseDown={keep} onClick={wrap('_', '_', 'italic')} disabled={tab() !== 'write'}>
            <Italic size={18} />
          </button>
          <button type="button" class="editor-key" aria-label="Heading" onPointerDown={keep} onMouseDown={keep} onClick={prefix('## ')} disabled={tab() !== 'write'}>
            <Heading2 size={18} />
          </button>
          <button type="button" class="editor-key" aria-label="Code" onPointerDown={keep} onMouseDown={keep} onClick={wrap('`', '`', 'code')} disabled={tab() !== 'write'}>
            <Code size={18} />
          </button>
          <button type="button" class="editor-key" aria-label="Bulleted list" onPointerDown={keep} onMouseDown={keep} onClick={prefix('- ')} disabled={tab() !== 'write'}>
            <List size={18} />
          </button>
          <button type="button" class="editor-key" aria-label="Checklist" onPointerDown={keep} onMouseDown={keep} onClick={prefix('- [ ] ')} disabled={tab() !== 'write'}>
            <ListChecks size={18} />
          </button>
          <button type="button" class="editor-key" aria-label="Quote" onPointerDown={keep} onMouseDown={keep} onClick={prefix('> ')} disabled={tab() !== 'write'}>
            <Quote size={18} />
          </button>
          <button type="button" class="editor-key" aria-label="Link" onPointerDown={keep} onMouseDown={keep} onClick={link} disabled={tab() !== 'write'}>
            <Link2 size={18} />
          </button>
          <button type="button" class="editor-key" aria-label="Mention a task" onPointerDown={keep} onMouseDown={keep} onClick={at} disabled={tab() !== 'write'}>
            <AtSign size={18} />
          </button>
          <button
            type="button"
            class="editor-key"
            aria-label="Attach to description"
            onClick={(event) => setMenu(event.currentTarget.getBoundingClientRect())}
          >
            <Paperclip size={18} />
          </button>
        </div>
      </div>
      {pickers.element}
      <Show when={menu()}>
        {(anchor) => <AttachMenu anchor={anchor()} alignEnd onDismiss={() => setMenu(undefined)} onPick={pickers.pick} />}
      </Show>
    </div>
  );
}
