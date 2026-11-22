/**
 * The reply dock of a ticket that waits for the owner (In Review, Blocked):
 * the shared note field and the buttons that act on it, pinned under the
 * scrolling ticket. Rework and Answer take the note as the move's comment, so
 * nothing opens a sheet. Opened from a Superagent queue, Mark done and Rework
 * wait out a 5 s undo window (lib/board/pendingMoves) and the next item opens
 * at once; on the Board they save immediately.
 *
 * Pasted or picked files wait in the dock (`lib/board/heldFiles`): they are stored on the ticket only when the
 * reply goes out (Comment, Rework, Answer, Mark done), so a removed chip or an unsent reply leaves no attachment.
 *
 * Keys (in the field): Ctrl/⌘+Enter comments, Ctrl/⌘+Shift+Enter reworks (or
 * answers), Esc leaves the field and cancels a pending empty Rework.
 */
import { createEffect, createSignal, on, onCleanup, Show } from 'solid-js';
import { Check, MessageSquare, RotateCcw } from 'lucide-solid';
import type { Task } from '../../lib/board/api';
import type { DraftRef } from '../../lib/board/attachDraft';
import { boardStore } from '../../lib/board/store';
import { advanceQueue, taskOrigin } from '../../lib/board/taskOrigin';
import { statusMoves } from '../../lib/board/report';
import { addNote, boardCtx, moveTask, uploadToTask } from '../../lib/board/taskActions';
import { noteKey, scheduleMove } from '../../lib/board/pendingMoves';
import { commitHeld, heldSize, heldThumb, holdFiles, releaseHeld } from '../../lib/board/heldFiles';
import { boardTarget, NoteField } from './NoteField';
import './review-dock.css';

/** How long the "press Rework again" hint stays armed. */
export const EMPTY_REWORK_MS = 5000;
export const EMPTY_REWORK_HINT = 'Add what should change, or press Rework again to send it back without a note.';

export function ReviewDock(props: {
  task: Task;
  kind: 'review' | 'blocked';
  note: string;
  onNote: (value: string) => void;
  uploading: number;
  setUploading: (fn: (count: number) => number) => void;
}) {
  const store = boardStore();
  const moves = () => statusMoves(store.state.rules, props.task);
  const [busy, setBusy] = createSignal(false);
  const [armed, setArmed] = createSignal(false);
  let area: HTMLTextAreaElement | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const disarm = () => { clearTimeout(timer); setArmed(false); };
  onCleanup(() => clearTimeout(timer));
  createEffect(on(() => props.task.id, disarm, { defer: true }));
  createEffect(() => { if (props.note.trim()) disarm(); });
  const blocked = () => props.kind === 'blocked';
  const waiting = () => busy() || props.uploading > 0;
  const heldKey = () => noteKey(store.target()?.slug ?? '', props.task.id);
  const deferred = {
    hold: (files: File[]) => holdFiles(heldKey(), files),
    thumb: (ref: DraftRef) => heldThumb(heldKey(), ref),
    size: (ref: DraftRef) => heldSize(heldKey(), ref),
  };
  /**
   * The reply as it goes out: the files its chips still name are stored on the ticket now (and only now).
   * A reply with no held file goes out as typed, at once (a string, not a promise).
   */
  function outgoing(text: string): string | Promise<string> {
    const key = heldKey();
    if (!/\(pending:/.test(text)) {
      releaseHeld(key);
      return text;
    }
    const task = props.task;
    props.setUploading((count) => count + 1);
    return commitHeld(key, text, async (file) => (await uploadToTask(task, [file]))[0])
      .then((stored) => {
        if (stored !== text) props.onNote(stored);
        return stored;
      })
      .finally(() => props.setUploading((count) => Math.max(0, count - 1)));
  }

  async function comment() {
    if (waiting() || !props.note.trim()) return;
    setBusy(true);
    const text = await outgoing(props.note);
    const ok = text.trim() ? await addNote(props.task, text) : false;
    setBusy(false);
    if (ok) props.onNote('');
    area?.focus();
  }
  async function move(to: 'done' | 'todo') {
    if (waiting()) return;
    if (to === 'todo' && !blocked() && !props.note.trim() && !armed()) {
      setArmed(true);
      area?.focus();
      clearTimeout(timer);
      timer = setTimeout(() => setArmed(false), EMPTY_REWORK_MS);
      return;
    }
    disarm();
    const out = outgoing(props.note);
    const text = typeof out === 'string' ? out : await out;
    const ctx = boardCtx();
    if (taskOrigin() && ctx) {
      scheduleMove({ hostId: ctx.hostId, slug: ctx.slug, task: props.task, to, note: text });
      await advanceQueue(props.task.id, { quiet: true });
      return;
    }
    setBusy(true);
    const ok = await moveTask(props.task, to, text.trim() || undefined, { confirmed: true });
    setBusy(false);
    if (ok) props.onNote('');
  }
  const leaveField = (event: KeyboardEvent) => {
    disarm();
    event.preventDefault();
    event.stopPropagation();
    const page = (event.currentTarget as HTMLElement).closest<HTMLElement>('.task-page');
    area?.blur();
    page?.focus({ preventScroll: true });
  };
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === 'Escape') return leaveField(event);
    if (event.key !== 'Enter' || !(event.ctrlKey || event.metaKey) || event.isComposing) return;
    event.preventDefault();
    if (event.shiftKey) void move('todo');
    else void comment();
  };

  return (
    <footer class="task-composer review-dock" data-kind={props.kind} aria-label={blocked() ? 'Answer' : 'Review reply'}>
      <NoteField
        {...boardTarget(props.task)}
        taskId={props.task.id}
        value={props.note}
        onInput={props.onNote}
        uploading={props.uploading}
        setUploading={props.setUploading}
        deferred={deferred}
        placeholder={blocked() ? 'Your answer — the agent sees it when it resumes. Paste or drop images' : 'What should change? Paste or drop images and files'}
        label="Add a note"
        fieldRef={(el) => (area = el)}
        onKeyDown={onKeyDown}
      />
      <Show when={armed()}>
        <p class="dock-hint" role="status" data-testid="rework-hint">{EMPTY_REWORK_HINT}</p>
      </Show>
      <div class="dock-actions">
        <button type="button" class="btn btn-ghost btn-sm dock-btn" data-kind="ghost" data-action="comment" title="Comment (Ctrl+Enter)" disabled={waiting() || !props.note.trim()} onClick={() => void comment()}>
          <MessageSquare size={16} /> Comment
        </button>
        <Show when={!blocked() && moves().includes('todo')}>
          <button type="button" class="btn btn-secondary btn-sm dock-btn" data-kind="secondary" data-action="rework" title="Rework (Ctrl+Shift+Enter)" disabled={waiting()} onClick={() => void move('todo')}>
            <RotateCcw size={16} /> Rework
          </button>
        </Show>
        <Show when={!blocked() && moves().includes('done')}>
          <button type="button" class="btn btn-primary btn-sm dock-btn" data-kind="primary" data-action="done" title="Mark done (D)" disabled={waiting()} onClick={() => void move('done')}>
            <Check size={17} /> Mark done
          </button>
        </Show>
        <Show when={blocked() && moves().includes('todo')}>
          <button type="button" class="btn btn-primary btn-sm dock-btn" data-kind="primary" data-action="answer" title="Answer & move to Todo (Ctrl+Shift+Enter)" disabled={waiting()} onClick={() => void move('todo')}>
            <RotateCcw size={16} /> Answer &amp; move to Todo
          </button>
        </Show>
      </div>
    </footer>
  );
}
