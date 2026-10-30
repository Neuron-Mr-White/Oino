/**
 * Sheets around moves (UNI-131):
 *  - CommentSheet — the web's CommentDialog: "from → to", why a note is
 *    needed (the daemon's hint), a note field with `@` mentions, Move.
 *    Also used for the optional notes the phone offers on its quick
 *    actions (rework note from In Review, the answer when unblocking):
 *    then Move works with an empty note too.
 *  - ConfirmSheet — bulk / irreversible actions (Done all, Archive all,
 *    and a menu move into a final lane) ask first: one mis-tap on a
 *    phone must not archive a lane or cancel a task.
 */
import { createEffect, createSignal, on, Show } from 'solid-js';
import { ArrowRight } from 'lucide-solid';
import { laneLabel } from '../../lib/board/api';
import { moveLabel } from './TaskMenus';
import { boardUi } from '../../lib/board/ui';
import { moveTask } from '../../lib/board/taskActions';
import { BoardSheet } from './BoardSheet';
import { StatusGlyph } from './Glyphs';
import { createMention, MentionStrip } from './Mention';
import { isSubmitChord } from '../../lib/board/fileDrop';

const PLACEHOLDER: Record<string, string> = {
  'in_review>todo': 'What should change? (optional) — the agent reads this when it picks the task up again',
  'blocked>todo': 'Your answer (optional) — the agent sees it when it resumes',
  'in_progress>todo': 'e.g. The session crashed overnight; picking it up fresh.',
  'in_progress>backlog': 'e.g. Parking this until the API settles.',
};

export function CommentSheet() {
  const ui = boardUi();
  const [text, setText] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | undefined>();
  let area: HTMLTextAreaElement | undefined;
  const request = () => ui.comment();
  const mention = createMention(() => area, setText, () => request()?.task.id);
  createEffect(
    on(request, (current) => {
      setText('');
      setBusy(false);
      setError(undefined);
      if (current) setTimeout(() => area?.focus({ preventScroll: true }), 120);
    }),
  );
  const close = () => ui.setComment(null);
  const canMove = () => !busy() && (request()?.optional || text().trim().length > 0);
  async function submit() {
    const current = request();
    if (!current || !canMove()) return;
    setBusy(true);
    setError(undefined);
    const moved = await moveTask(current.task, current.to, text().trim() || undefined, { onError: setError, confirmed: true, after: current.after });
    setBusy(false);
    // A refusal keeps the sheet and the typed note (the error is a toast);
    // a "needs a note after all" refusal swaps in a new request.
    if (moved) close();
  }
  const key = () => `${request()?.task.status}>${request()?.to}`;
  return (
    <BoardSheet
      open={request() !== null}
      onClose={close}
      label={request()?.optional ? 'Add a note' : 'Needs a note'}
      testId="comment-sheet"
      class="comment-sheet"
      guarded={text().trim().length > 0}
      title={
        <span class="flex items-center gap-2 min-w-0">
          <span class="crumb-pill font-mono">{request()?.task.id}</span>
          <span class="truncate">{request()?.optional ? 'Add a note' : 'Needs a note'}</span>
        </span>
      }
      footer={
        <div class="flex gap-3">
          <button type="button" class="pair-secondary flex-1" onClick={close}>
            Cancel
          </button>
          <button type="button" class="pair-primary flex-[1.4]" disabled={!canMove()} onClick={() => void submit()}>
            {busy() ? 'Moving…' : request()?.optional && !text().trim() ? 'Move without note' : moveLabel(request()?.task.status ?? '', request()?.to ?? '').replace(' (rework)', '')}
          </button>
        </div>
      }
    >
      <div class="move-route" aria-label={`${laneLabel(request()?.task.status ?? '')} to ${laneLabel(request()?.to ?? '')}`}>
        <span class="move-lane">
          <StatusGlyph status={request()?.task.status ?? 'backlog'} size={15} />
          {laneLabel(request()?.task.status ?? '')}
        </span>
        <ArrowRight size={16} class="text-text-dim" />
        <span class="move-lane">
          <StatusGlyph status={request()?.to ?? 'backlog'} size={15} />
          {laneLabel(request()?.to ?? '')}
        </span>
      </div>
      <p class="move-why">
        <Show
          when={!request()?.optional}
          fallback={<>A note is optional here. Whoever picks the task up next sees it.</>}
        >
          This move needs a note — <b>{request()?.hint ?? 'note'}</b> — so whoever picks the task up next knows why.
        </Show>
      </p>
      <Show when={error()}>
        <p class="sheet-error" role="alert">
          {error()}
        </p>
      </Show>
      <MentionStrip mention={mention} />
      <textarea
        ref={area}
        class="sheet-textarea"
        rows={4}
        aria-label={request()?.optional ? 'Note (optional)' : 'Note'}
        placeholder={PLACEHOLDER[key()] ?? 'e.g. The retry test still fails on CI — needs the fixture fixed first.'}
        value={text()}
        onInput={(event) => {
          setText(event.currentTarget.value);
          mention.update();
        }}
        onClick={() => mention.update()}
        onKeyDown={(event) => {
          if (isSubmitChord(event)) {
            event.preventDefault();
            void submit();
          }
        }}
        onBlur={() => setTimeout(() => mention.close(), 150)}
      />
    </BoardSheet>
  );
}

export function ConfirmSheet() {
  const ui = boardUi();
  const [busy, setBusy] = createSignal(false);
  const request = () => ui.confirm();
  const close = () => !busy() && ui.setConfirm(null);
  async function run() {
    const current = request();
    if (!current || busy()) return;
    setBusy(true);
    try {
      await current.run();
    } finally {
      setBusy(false);
      ui.setConfirm(null);
    }
  }
  return (
    <BoardSheet
      open={request() !== null}
      onClose={close}
      label={request()?.title ?? 'Confirm'}
      testId="confirm-sheet"
      footer={
        <div class="flex gap-3">
          <button type="button" class="pair-secondary flex-1" onClick={close}>
            {request()?.dismiss ?? 'Cancel'}
          </button>
          <button
            type="button"
            class={`flex-[1.4] ${request()?.danger ? 'danger-primary' : 'pair-primary'}`}
            disabled={busy()}
            onClick={() => void run()}
          >
            {busy() ? 'Working…' : request()?.confirm}
          </button>
        </div>
      }
    >
      <Show when={request()?.detail}>
        <p class="move-why mt-0">{request()?.detail}</p>
      </Show>
    </BoardSheet>
  );
}
