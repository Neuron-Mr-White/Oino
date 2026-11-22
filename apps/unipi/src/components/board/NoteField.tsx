/**
 * The one note field of the Board and the Workbench: an auto-growing textarea
 * with `@` task mentions, an attach menu (camera / photos / files), OS file
 * drop and paste (uploaded at once, their markdown inserted at the cursor)
 * and a preview of what was attached (the review dock holds them until the reply is sent). TaskComposer, the review dock, the
 * move-note sheet and the escalation reply all use it; each passes where
 * its files go (`upload`) and how a thumbnail is fetched (`thumb`).
 */
import { createEffect, createMemo, createSignal, For, on, onCleanup, Show, type JSX } from 'solid-js';
import { Camera, FileUp, Image, Paperclip, X } from 'lucide-solid';
import { api, filePath, type Attachment, type Task } from '../../lib/board/api';
import { insertAtCursor, MAX_UPLOAD_BYTES } from '../../lib/board/attach';
import { attTarget, joinRefs, parseRef, splitRefs, type DraftRef } from '../../lib/board/attachDraft';
import { AttachChips } from './AttachChips';
import { boardStore } from '../../lib/board/store';
import { uploadToTask } from '../../lib/board/taskActions';
import { setFieldValue } from '../../lib/fieldEdit';
import { boardUi } from '../../lib/board/ui';
import { AnchoredMenu, MenuRow } from './AnchoredMenu';
import { createMediaIntake } from './mediaIntake';
import { createMention, MentionStrip } from './Mention';
import './note-field.css';

export type PickKind = 'camera' | 'photos' | 'files';

/** Three hidden inputs (camera capture, image library, any file) that must outlive the menu that triggers them. */
export function createFilePickers(onFiles: (files: File[]) => void, label = 'Attach') {
  const inputs: Partial<Record<PickKind, HTMLInputElement>> = {};
  const change = (event: Event & { currentTarget: HTMLInputElement }) => {
    const files = [...(event.currentTarget.files ?? [])];
    event.currentTarget.value = '';
    if (files.length > 0) onFiles(files);
  };
  const element: JSX.Element = (
    <span hidden>
      <input ref={(el) => (inputs.camera = el)} type="file" accept="image/*" capture="environment" aria-label={`${label}: take a photo`} onChange={change} />
      <input ref={(el) => (inputs.photos = el)} type="file" accept="image/*,video/*" multiple aria-label={`${label}: photos`} onChange={change} />
      <input ref={(el) => (inputs.files = el)} type="file" multiple aria-label={`${label}: files`} onChange={change} />
    </span>
  );
  return { element, pick: (kind: PickKind) => inputs[kind]?.click() };
}

export function AttachMenu(props: { anchor: DOMRect; onDismiss: () => void; onPick: (kind: PickKind) => void; alignEnd?: boolean }) {
  const pick = (kind: PickKind) => () => {
    props.onDismiss();
    props.onPick(kind);
  };
  return (
    <AnchoredMenu anchor={props.anchor} width={232} alignEnd={props.alignEnd} label="Attach" onDismiss={props.onDismiss}>
      <MenuRow icon={<Camera size={18} />} label="Take photo" onSelect={pick('camera')} />
      <MenuRow icon={<Image size={18} />} label="Photo library" onSelect={pick('photos')} />
      <MenuRow icon={<FileUp size={18} />} label="File" hint="25 MB max" onSelect={pick('files')} />
    </AnchoredMenu>
  );
}

/** Where a ticket's files go: the open board's project (the default for a task shown on the Board). */
export function boardTarget(task: Task): Pick<NoteTarget, 'upload' | 'thumb'> {
  return {
    upload: (files) => uploadToTask(task, files),
    thumb: async (item) => {
      const at = boardStore().target();
      return at?.slug ? api.fileUrl(at.hostId, filePath(at.slug, task.id, item.name)) : undefined;
    },
  };
}

export type NoteTarget = {
  /** Uploads to the ticket; resolves what was stored (failures are toasted by the uploader). */
  upload: (files: File[]) => Promise<Attachment[]>;
  /** A URL for an attachment's thumbnail. */
  thumb: (item: Attachment) => Promise<string | undefined>;
};

/** A file picked but not yet stored: its own preview (an object URL) and a way to take it back out. */
function HeldPreview(props: { file: File; onRemove: () => void }) {
  const image = props.file.type.startsWith('image/');
  const url = image && typeof URL.createObjectURL === 'function' ? URL.createObjectURL(props.file) : undefined;
  onCleanup(() => { if (url) URL.revokeObjectURL?.(url); });
  return (
    <span class="note-held" data-held>
      <Show when={url} fallback={<span class="note-chip" title={props.file.name}><Paperclip size={13} /><span class="truncate">{props.file.name}</span></span>}>
        <span class="note-thumb" title={props.file.name}><img src={url} alt={props.file.name} /></span>
      </Show>
      <button type="button" class="note-held-remove" aria-label={`Remove ${props.file.name}`} onClick={props.onRemove}><X size={12} /></button>
    </span>
  );
}

export function NoteField(props: NoteTarget & {
  /** The ticket the note belongs to (`@` never offers it). */
  taskId: string;
  value: string;
  onInput: (value: string) => void;
  uploading: number;
  setUploading: (fn: (count: number) => number) => void;
  placeholder: string;
  label: string;
  attachLabel?: string;
  rows?: number;
  disabled?: boolean;
  /** Runs first; `preventDefault()` keeps the field's own handling out. */
  onKeyDown?: (event: KeyboardEvent) => void;
  onFocusChange?: (focused: boolean) => void;
  /** The textarea, for callers that focus it. */
  fieldRef?: (el: HTMLTextAreaElement) => void;
  /** Right of the field (a send button). */
  trailing?: JSX.Element;
  /**
   * Keep picked files on this side instead of uploading them at once (a sheet that can be cancelled): they are
   * shown with a remove button, and whoever sends the note attaches them with it. Nothing is stored before that.
   */
  held?: { files: () => File[]; set: (files: File[]) => void };
  /**
   * Keep picked files on this side too, but as the chips of the note itself (a `pending:<n>` link each):
   * a reply that may never be sent (the review dock) stores nothing until whoever sends it commits the
   * files the text still names (`lib/board/heldFiles`). Removing a chip then leaves nothing behind.
   */
  deferred?: { hold: (files: File[]) => DraftRef[]; thumb: (ref: DraftRef) => string | undefined; size: (ref: DraftRef) => number | undefined };
  class?: string;
}) {
  const [menu, setMenu] = createSignal<DOMRect | undefined>();
  const [uploaded, setUploaded] = createSignal<Attachment[]>([]);
  let area: HTMLTextAreaElement | undefined;
  /** The note's words, and the attachment links closing it (chips above the field, never text — P3-1). */
  const parts = createMemo(() => splitRefs(props.value));
  const text = () => parts().text;
  const emit = (next: string) => props.onInput(joinRefs(next, parts().refs));
  const mention = createMention(() => area, emit, () => props.taskId);
  const fit = () => {
    if (!area) return;
    area.style.height = 'auto';
    area.style.height = `${Math.min(area.scrollHeight, 132)}px`;
  };
  createEffect(on(() => props.value, (value) => {
    // A send clears the note: the strip must not outlive an `@id` it was for.
    if (!value) mention.close();
    queueMicrotask(fit);
  }));
  async function attach(files: File[]) {
    if (props.held) {
      const ok = files.filter((file) => file.size <= MAX_UPLOAD_BYTES);
      if (ok.length < files.length) boardUi().toast(`${files.find((file) => file.size > MAX_UPLOAD_BYTES)!.name} is larger than 25 MB`, 'error');
      if (ok.length > 0) props.held.set([...props.held.files(), ...ok]);
      return;
    }
    if (props.deferred) {
      const ok = files.filter((file) => file.size <= MAX_UPLOAD_BYTES);
      if (ok.length < files.length) boardUi().toast(`${files.find((file) => file.size > MAX_UPLOAD_BYTES)!.name} is larger than 25 MB`, 'error');
      if (ok.length > 0) props.onInput(joinRefs(text(), [...parts().refs, ...props.deferred.hold(ok)]));
      return;
    }
    props.setUploading((count) => count + files.length);
    try {
      const done = await props.upload(files);
      if (done.length === 0) return;
      const added = done.map((item) => parseRef(item.markdown)).filter((ref): ref is DraftRef => !!ref);
      props.onInput(added.length === done.length ? joinRefs(text(), [...parts().refs, ...added]) : insertAtCursor(area, props.value, done.map((item) => item.markdown).join('\n')));
      setUploaded((old) => [...old, ...done]);
      queueMicrotask(fit);
    } finally {
      props.setUploading((count) => Math.max(0, count - files.length));
    }
  }
  const pickers = createFilePickers((files) => void attach(files), props.attachLabel ?? 'Attach to note');
  let wrap: HTMLDivElement | undefined;
  const intake = createMediaIntake({
    attach,
    zone: () => wrap,
    disabled: () => !!props.disabled,
    pasteText: (pasted) => {
      if (!area) return;
      const next = insertAtCursor(area, text(), pasted);
      setFieldValue(area, next);
      emit(next);
    },
  });
  const drop = intake.drop;
  /** What a link names: the upload this note made, else the stored file the link points at. */
  const itemOf = (ref: DraftRef): Attachment => {
    const made = uploaded().find((item) => item.markdown === ref.markdown);
    if (made) return made;
    const name = attTarget(ref.target)?.name ?? ref.name;
    return { name, original: ref.name, ref: ref.target, path: '', size: 0, mime: '', kind: ref.image ? 'image' : 'file', markdown: ref.markdown } as Attachment;
  };
  return (
    <div ref={wrap} class={`note-field${props.class ? ` ${props.class}` : ''}`} classList={{ 'drop-over': drop.over() }} {...drop.handlers}>
      <MentionStrip mention={mention} />
      <Show when={props.held && props.held.files().length > 0}>
        <div class="note-previews composer-preview" aria-label="Attached to this note">
          <For each={props.held!.files()}>{(file) => <HeldPreview file={file} onRemove={() => props.held!.set(props.held!.files().filter((f) => f !== file))} />}</For>
        </div>
      </Show>
      <AttachChips
        refs={parts().refs}
        thumb={(ref) => props.deferred?.thumb(ref) ?? props.thumb(itemOf(ref))}
        size={(ref) => props.deferred?.size(ref) ?? uploaded().find((item) => item.markdown === ref.markdown)?.size}
        onRemove={(ref) => props.onInput(joinRefs(text(), parts().refs.filter((item) => item !== ref)))}
      />
      <div class="composer-row">
        <button type="button" class="composer-round" aria-label={props.attachLabel ?? 'Attach to note'} aria-expanded={menu() !== undefined} disabled={props.disabled} onClick={(event) => setMenu(event.currentTarget.getBoundingClientRect())}>
          <Paperclip size={20} />
        </button>
        <div class="composer-field">
          <textarea
            ref={(el) => { area = el; props.fieldRef?.(el); }}
            rows={props.rows ?? 1}
            value={text()}
            disabled={props.disabled}
            placeholder={props.uploading > 0 ? `Uploading ${props.uploading}…` : props.placeholder}
            aria-label={props.label}
            enterkeyhint="enter"
            onInput={(event) => {
              emit(event.currentTarget.value);
              fit();
              mention.update();
            }}
            onClick={() => mention.update()}
            onKeyDown={(event) => props.onKeyDown?.(event)}
            onFocus={() => props.onFocusChange?.(true)}
            onBlur={() => {
              props.onFocusChange?.(false);
              setTimeout(() => mention.close(), 150);
            }}
            onPaste={intake.onPaste}
          />
        </div>
        {props.trailing}
      </div>
      {pickers.element}
      <Show when={menu()}>{(anchor) => <AttachMenu anchor={anchor()} onDismiss={() => setMenu(undefined)} onPick={pickers.pick} />}</Show>
    </div>
  );
}
