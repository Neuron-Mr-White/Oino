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
import { createMemo, createSignal, onCleanup, onMount, Show } from 'solid-js';
import { AtSign, Bold, Code, Heading2, Italic, Link2, List, ListChecks, Paperclip, Quote } from 'lucide-solid';
import { api, filePath, type Task } from '../../lib/board/api';
import { insertAtCursor, MAX_UPLOAD_BYTES, namedFile, pendingMarkdown, resolvePending, stripPending } from '../../lib/board/attach';
import { attTarget, joinRefs, parseRef, pendingId, splitRefs, type DraftRef } from '../../lib/board/attachDraft';
import { boardStore } from '../../lib/board/store';
import { boardUi } from '../../lib/board/ui';
import { AttachChips } from './AttachChips';
import { uploadToTask } from '../../lib/board/taskActions';
import { isSubmitChord } from '../../lib/board/fileDrop';
import { createMediaIntake } from './mediaIntake';
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
  /** The description's words, and the attachment links closing it (chips under the field, never text — D2). */
  const parts = createMemo(() => splitRefs(props.value));
  const text = () => parts().text;
  const emit = (next: string) => props.onInput(joinRefs(next, parts().refs));
  const mention = createMention(() => area, emit, () => props.task.id);
  /**
   * Files picked or pasted here wait on this side until Save (P3-6): nothing is stored on the task before that,
   * so cancelling the edit leaves no orphan attachment behind. Their chips carry a `pending:<n>` link.
   */
  const [held, setHeld] = createSignal<File[]>([]);
  const heldIds = new WeakMap<File, string>();
  const heldUrls = new WeakMap<File, string>();
  let heldSeq = 0;
  const heldOf = (ref: DraftRef) => held().find((file) => heldIds.get(file) === pendingId(ref));
  onCleanup(() => held().forEach((file) => { const url = heldUrls.get(file); if (url) URL.revokeObjectURL?.(url); }));
  const thumb = (ref: DraftRef): Promise<string | undefined> | string | undefined => {
    const file = heldOf(ref);
    if (file) {
      if (typeof URL.createObjectURL !== 'function') return undefined;
      let url = heldUrls.get(file);
      if (!url) heldUrls.set(file, (url = URL.createObjectURL(file)));
      return url;
    }
    const at = boardStore().target();
    const stored = attTarget(ref.target);
    return at?.slug && stored ? api.fileUrl(at.hostId, filePath(at.slug, stored.task, stored.name)) : undefined;
  };
  const sizeOf = (ref: DraftRef): number | undefined => {
    const file = heldOf(ref);
    if (file) return file.size;
    const stored = attTarget(ref.target);
    return props.task.attachments?.find((item) => item.name === stored?.name)?.size;
  };
  const removeRef = (ref: DraftRef) => {
    const file = heldOf(ref);
    if (file) setHeld((current) => current.filter((item) => item !== file));
    props.onInput(joinRefs(text(), parts().refs.filter((item) => item !== ref)));
  };

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
    emit(next);
    area?.focus();
  };
  const sel = () => [area?.selectionStart ?? text().length, area?.selectionEnd ?? text().length] as const;
  const wrap = (before: string, after = before, placeholder?: string) => () => apply(wrapSelection(text(), ...sel(), before, after, placeholder));
  const prefix = (prefixText: string) => () => apply(prefixLines(text(), ...sel(), prefixText));
  const link = () => {
    const [start, end] = sel();
    const label = text().slice(start, end) || 'link';
    const next = `${text().slice(0, start)}[${label}](https://)${text().slice(end)}`;
    const urlAt = start + label.length + 3;
    apply([next, urlAt, urlAt + 8]);
  };
  const at = () => {
    const [start, end] = sel();
    const lead = start > 0 && !/\s/.test(text()[start - 1]!) ? ' ' : '';
    const next = `${text().slice(0, start)}${lead}@${text().slice(end)}`;
    const caret = start + lead.length + 1;
    apply([next, caret, caret]);
    mention.update();
  };
  function attach(list: File[]) {
    const ok = list.filter((file) => file.size <= MAX_UPLOAD_BYTES);
    if (ok.length < list.length) boardUi().toast(`${list.find((file) => file.size > MAX_UPLOAD_BYTES)!.name} is larger than 25 MB`, 'error');
    const named = ok.map((file) => namedFile(file));
    if (named.length === 0) return;
    for (const file of named) heldIds.set(file, String(++heldSeq));
    setHeld((current) => [...current, ...named]);
    props.onInput(joinRefs(text(), [...parts().refs, ...named.map((file) => parseRef(pendingMarkdown(file, heldIds.get(file)!))!)]));
  }
  /** Save: the held files are stored on the task first, then their links replace the placeholders. */
  async function save() {
    const files = held();
    if (files.length === 0) return props.onSave();
    props.setUploading((count) => count + files.length);
    try {
      const resolved: Array<{ id: string; markdown: string | null }> = [];
      for (const file of files) {
        // One at a time: a file that fails is dropped from the text (the uploader toasts why).
        const [item] = await uploadToTask(props.task, [file]);
        resolved.push({ id: heldIds.get(file)!, markdown: item?.markdown ?? null });
      }
      props.onInput(resolvePending(props.value, resolved));
      setHeld([]);
    } finally {
      props.setUploading((count) => Math.max(0, count - files.length));
    }
    props.onSave();
  }
  const pickers = createFilePickers((files) => void attach(files), 'Attach to description');
  const intake = createMediaIntake({
    attach,
    zone: () => area,
    pasteText: (pasted) => {
      const next = insertAtCursor(area, text(), pasted);
      if (area) setFieldValue(area, next);
      emit(next);
    },
  });
  const drop = intake.drop;
  /** Keys never take focus from the field (the keyboard stays up). */
  const keep = (event: Event) => event.preventDefault();

  return (
    <div class="body-editor" role="dialog" aria-label="Edit description" data-testid="body-editor">
      <header class="task-head">
        <button type="button" class="board-text-button muted" onClick={cancel}>
          Cancel
        </button>
        <span class="flex-1 text-center text-[16px] font-semibold">Description</span>
        <button type="button" class="board-text-button" disabled={props.busy || props.uploading > 0} onClick={() => void save()}>
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
            <Markdown text={stripPending(props.value)} empty="Nothing to preview yet." />
          </div>
        }
      >
        <textarea
          ref={area}
          class="editor-area"
          aria-label="Description (markdown)"
          placeholder="Describe the task… Markdown works: **bold**, - lists, - [ ] checklists, `code`. @ mentions a task."
          value={text()}
          onInput={(event) => {
            emit(event.currentTarget.value);
            mention.update();
          }}
          onClick={() => mention.update()}
          classList={{ 'drop-over': drop.over() }}
          {...drop.handlers}
          onKeyDown={(event) => {
            if (isSubmitChord(event) && !props.busy && props.uploading === 0) {
              event.preventDefault();
              void save();
            }
          }}
          onPaste={intake.onPaste}
        />
      </Show>
      <Show when={tab() === 'write'}>
        <div class="editor-chips">
          <AttachChips refs={parts().refs} thumb={thumb} size={sizeOf} onRemove={removeRef} />
        </div>
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
