/**
 * Rename dialog (Z-15, WP-A5): the menu's "Rename…" row and the header ⋯
 * "Rename…" action both land here — one input, Enter saves. A manual name wins
 * over auto-rename: the host sets the card's name (`session_rename`) and the
 * app locks it (`session_meta_set{title_locked}`), so the utility extension's
 * auto-rename skips this session from now on.
 *
 * Desktop: centred modal dialog; phone: the standard bottom sheet
 * (`BoardSheet`), like the Session modal.
 */
import { createEffect, createSignal, onCleanup, onMount, Show } from 'solid-js';
import { X } from 'lucide-solid';
import { BoardSheet } from '../board/BoardSheet';
import { layout } from '../../lib/layout';
import { wrapTab } from '../../lib/focusTrap';
import { renameSession } from '../../lib/session/actions';
import { pushBack } from '../../lib/back';

export type RenameTarget = { hostId: string; sessionId: string; name: string };

const [target, setTarget] = createSignal<RenameTarget>();

/** What the dialog is renaming (`undefined` = closed). */
export const renameDialog = target;
export function openRenameDialog(next: RenameTarget): void {
  setTarget(next);
}
export function closeRenameDialog(): void {
  setTarget(undefined);
}

/** Longest name the host takes (`SessionName`, 256). */
export const NAME_MAX = 256;

export function RenameDialog() {
  const [value, setValue] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  const close = () => !busy() && closeRenameDialog();
  // Opening resets the field to the current name (a second Rename… of the same
  // session must not keep the previous edit).
  createEffect(() => {
    const t = target();
    if (t) setValue(t.name);
  });
  return (
    <Show when={target()}>
      {(t) => {
        const save = async () => {
          const name = value().trim();
          if (!name || busy()) return;
          setBusy(true);
          await renameSession(t().hostId, t().sessionId, name, { notify: true });
          setBusy(false);
          closeRenameDialog();
        };
        return (
          <Show when={layout().desktop()} fallback={
            <BoardSheet open onClose={close} label="Rename" title="Rename" testId="rename-sheet">
              <RenameForm value={value()} setValue={setValue} busy={busy()} onSave={save} onCancel={close} />
            </BoardSheet>
          }>
            <BackClose onClose={close} />
            <div class="sm-overlay" role="presentation" onClick={close} data-testid="rename-overlay">
              <div
                class="sm-dialog rename-dialog"
                role="dialog"
                aria-modal="true"
                aria-label="Rename session"
                tabindex="-1"
                ref={(el) => queueMicrotask(() => el.focus({ preventScroll: true }))}
                onKeyDown={(e: KeyboardEvent) => {
                  if (wrapTab(e, e.currentTarget as HTMLElement)) e.stopPropagation();
                  if (e.key === 'Escape') close();
                }}
                onClick={(e) => e.stopPropagation()}
              >
                <div class="sm-head">
                  <span class="sm-title">
                    <h2>Rename</h2>
                  </span>
                  <button type="button" class="ghost-icon-button" aria-label="Close" onClick={close} data-testid="rename-close">
                    <X size={20} />
                  </button>
                </div>
                <RenameForm value={value()} setValue={setValue} busy={busy()} onSave={save} onCancel={close} />
              </div>
            </div>
          </Show>
        );
      }}
    </Show>
  );
}

/** Android Back closes the dialog while it is open (not the app under it). */
function BackClose(props: { onClose: () => void }) {
  onMount(() => onCleanup(pushBack(props.onClose)));
  return null;
}

function RenameForm(props: { value: string; setValue: (v: string) => void; busy: boolean; onSave: () => void; onCancel: () => void }) {
  let input: HTMLInputElement | undefined;
  onMount(() =>
    queueMicrotask(() => {
      input?.focus({ preventScroll: true });
      input?.select();
    }),
  );
  return (
    <form
      class="rename-form"
      onSubmit={(e) => {
        e.preventDefault();
        props.onSave();
      }}
      data-testid="rename-form"
    >
      <input
        ref={input}
        class="rename-input"
        type="text"
        value={props.value}
        maxlength={NAME_MAX}
        aria-label="Session name"
        data-testid="rename-input"
        onInput={(e) => props.setValue(e.currentTarget.value)}
        onKeyDown={(e) => e.key === 'Escape' && props.onCancel()}
      />
      <div class="rename-actions">
        <button type="button" class="btn ghost" onClick={props.onCancel} data-testid="rename-cancel">
          Cancel
        </button>
        <button type="submit" class="btn" disabled={!props.value.trim() || props.busy} data-testid="rename-save">
          Rename
        </button>
      </div>
    </form>
  );
}
