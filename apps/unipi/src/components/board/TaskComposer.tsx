/**
 * The note composer pinned at the bottom of the task page (web TaskPanel
 * composer, UNI-131): auto-growing field, `@` task mentions (chip strip
 * above the field), attachments from the camera, the photo library or
 * files (uploaded right away, their markdown inserted at the cursor, a
 * preview of attached images above the field), Send. It rides above the
 * soft keyboard with the page.
 */
import { createSignal, Show, type JSX } from 'solid-js';
import { Camera, FileUp, Image, Paperclip, SendHorizontal } from 'lucide-solid';
import type { Task } from '../../lib/board/api';
import { insertAtCursor } from '../../lib/board/attach';
import { addNote, uploadToTask } from '../../lib/board/taskActions';
import { createFileDrop, isSubmitChord } from '../../lib/board/fileDrop';
import { AnchoredMenu, MenuRow } from './AnchoredMenu';
import { createMention, MentionStrip } from './Mention';
import { Markdown } from './Markdown';
import { setFieldValue } from '../../lib/fieldEdit';

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

export function TaskComposer(props: {
  task: Task;
  value: string;
  onInput: (value: string) => void;
  uploading: number;
  setUploading: (fn: (count: number) => number) => void;
  onFocusChange?: (focused: boolean) => void;
}) {
  const [menu, setMenu] = createSignal<DOMRect | undefined>();
  const [sending, setSending] = createSignal(false);
  let area: HTMLTextAreaElement | undefined;
  const mention = createMention(() => area, (next) => props.onInput(next), () => props.task.id);

  const fit = () => {
    if (!area) return;
    area.style.height = 'auto';
    area.style.height = `${Math.min(area.scrollHeight, 132)}px`;
  };
  async function attach(files: File[]) {
    props.setUploading((count) => count + files.length);
    try {
      const done = await uploadToTask(props.task, files);
      if (done.length === 0) return;
      const snippet = done.map((item) => item.markdown).join('\n');
      const next = insertAtCursor(area, props.value, snippet);
      if (area) setFieldValue(area, next);
      props.onInput(next);
      queueMicrotask(fit);
    } finally {
      props.setUploading((count) => Math.max(0, count - files.length));
    }
  }
  const pickers = createFilePickers((files) => void attach(files), 'Attach to note');
  const drop = createFileDrop((files) => void attach(files));
  const canSend = () => !sending() && props.uploading === 0 && props.value.trim().length > 0;
  async function send() {
    if (!canSend()) return;
    setSending(true);
    const ok = await addNote(props.task, props.value);
    setSending(false);
    if (ok) {
      props.onInput('');
      // A Ctrl/⌘+Enter send leaves the caret right after an `@id` — the
      // mention strip must not outlive the note it was for.
      mention.close();
      queueMicrotask(fit);
    }
  }

  return (
    <footer class="task-composer" classList={{ 'drop-over': drop.over() }} {...drop.handlers}>
      <MentionStrip mention={mention} />
      <Show when={props.value.includes('att:')}>
        <Markdown text={props.value.split('\n').filter((line) => line.includes('att:')).join('\n')} class="composer-preview" />
      </Show>
      <div class="composer-row">
        <button
          type="button"
          class="composer-round"
          aria-label="Attach to note"
          aria-expanded={menu() !== undefined}
          onClick={(event) => setMenu(event.currentTarget.getBoundingClientRect())}
        >
          <Paperclip size={20} />
        </button>
        <div class="composer-field">
          <textarea
            ref={area}
            rows={1}
            value={props.value}
            placeholder={props.uploading > 0 ? `Uploading ${props.uploading}…` : 'Add a note… @ links a task'}
            aria-label="Add a note"
            enterkeyhint="enter"
            onInput={(event) => {
              props.onInput(event.currentTarget.value);
              fit();
              mention.update();
            }}
            onClick={() => mention.update()}
            onKeyDown={(event) => {
              if (isSubmitChord(event)) {
                event.preventDefault();
                void send();
              }
            }}
            onFocus={() => props.onFocusChange?.(true)}
            onBlur={() => {
              props.onFocusChange?.(false);
              setTimeout(() => mention.close(), 150);
            }}
            onPaste={(event) => {
              const files = [...(event.clipboardData?.files ?? [])];
              if (files.length === 0) return;
              event.preventDefault();
              void attach(files);
            }}
          />
        </div>
        <button
          type="button"
          class="composer-send"
          aria-label="Send note"
          disabled={!canSend()}
          onPointerDown={(event) => event.preventDefault()}
          onClick={() => void send()}
        >
          <SendHorizontal size={20} />
        </button>
      </div>
      {pickers.element}
      <Show when={menu()}>
        {(anchor) => <AttachMenu anchor={anchor()} onDismiss={() => setMenu(undefined)} onPick={pickers.pick} />}
      </Show>
    </footer>
  );
}
