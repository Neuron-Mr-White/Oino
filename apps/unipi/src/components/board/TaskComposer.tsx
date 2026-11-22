/**
 * The note composer pinned at the bottom of the task page (web TaskPanel
 * composer, UNI-131): the shared NoteField (auto-growing field, `@` task
 * mentions, attachments from the camera, the photo library or files — uploaded
 * right away, their markdown inserted at the cursor, a preview above the
 * field) plus Send. It rides above the soft keyboard with the page.
 */
import { createSignal } from 'solid-js';
import { SendHorizontal } from 'lucide-solid';
import type { Task } from '../../lib/board/api';
import { addNote } from '../../lib/board/taskActions';
import { isSubmitChord } from '../../lib/board/fileDrop';
import { boardTarget, NoteField } from './NoteField';

export { AttachMenu, createFilePickers, type PickKind } from './NoteField';

export function TaskComposer(props: {
  task: Task;
  value: string;
  onInput: (value: string) => void;
  uploading: number;
  setUploading: (fn: (count: number) => number) => void;
  onFocusChange?: (focused: boolean) => void;
}) {
  const [sending, setSending] = createSignal(false);
  const canSend = () => !sending() && props.uploading === 0 && props.value.trim().length > 0;
  async function send() {
    if (!canSend()) return;
    setSending(true);
    const ok = await addNote(props.task, props.value);
    setSending(false);
    if (ok) props.onInput('');
  }

  return (
    <footer class="task-composer">
      <NoteField
        {...boardTarget(props.task)}
        taskId={props.task.id}
        value={props.value}
        onInput={props.onInput}
        uploading={props.uploading}
        setUploading={props.setUploading}
        placeholder="Add a note… @ links a task"
        label="Add a note"
        onFocusChange={props.onFocusChange}
        onKeyDown={(event) => {
          if (isSubmitChord(event)) {
            event.preventDefault();
            void send();
          }
        }}
        trailing={
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
        }
      />
    </footer>
  );
}
