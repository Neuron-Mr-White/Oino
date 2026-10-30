/**
 * The dialog panel of the bottom dock (WP-A9, Z-46; app spec §3.11):
 * pending pi/Oino dialogs dock **above** the composer, so typing, steering
 * and Stop keep working while a question waits (this supersedes ui-protocol
 * §3.2 "dialogs take over the composer slot").
 *
 * Dialogs queue in **arrival order** with a "1 of N" pager; on the phone the
 * panel collapses to a one-line "Question waiting" bar (tap it to expand).
 * The body is the moved `ui/dialogs/DialogCard` for pi's kinds and
 * `ApprovalPanel` for `approval`; WP-U10 widens the body set behind this same
 * frame.
 */
import { createEffect, createMemo, createSignal, Show } from 'solid-js';
import { ChevronDown, ChevronLeft, ChevronRight, ChevronUp, MessageCircleQuestion } from 'lucide-solid';
import { DialogCard } from '../../ui/dialogs/DialogCard';
import { piDialog } from '../../ui/dialogs/adapter';
import type { UiRequest } from '../../ui/types';
import { ApprovalPanel } from './ApprovalPanel';

/** Kinds A9 renders itself; U10 adds the rest (`multiSelect`, `form`, `custom` nodes, …). */
const PI_KINDS = new Set(['select', 'confirm', 'input', 'editor']);

export function DialogPanel(props: {
  /** Open dialogs, in arrival order (the registry keeps that order). */
  dialogs: readonly UiRequest[];
  /** Sends one answer (`undefined`/`null` = cancelled). */
  onAnswer: (id: string, value: unknown) => void;
  /** Phone: collapse to a one-line bar (Z-46). */
  phone?: boolean;
  onHaptic?: () => void;
}) {
  const [page, setPage] = createSignal(0);
  const [collapsed, setCollapsed] = createSignal(false);
  const count = () => props.dialogs.length;
  const index = () => Math.max(0, Math.min(page(), count() - 1));
  // A dialog closing under the pager must not leave it pointing past the end.
  createEffect(() => {
    if (page() > count() - 1) setPage(Math.max(0, count() - 1));
  });
  const shown = createMemo(() => props.dialogs[index()]);
  const step = (by: number) => setPage(Math.max(0, Math.min(count() - 1, index() + by)));

  const answer = (id: string) => (value: unknown) => props.onAnswer(id, value);
  const body = () => {
    const dialog = shown();
    if (!dialog) return undefined;
    if (dialog.method === 'approval') return <ApprovalPanel dialog={dialog} onAnswer={answer(dialog.id)} />;
    if (PI_KINDS.has(dialog.method) || dialog.method === 'custom' || Array.isArray(dialog.questions)) {
      return <DialogCard dialog={piDialog(dialog)} onAnswer={answer(dialog.id)} onHaptic={() => props.onHaptic?.()} />;
    }
    return (
      <div class="dlg dlg-unsupported" data-testid="dialog-unsupported">
        <div class="dlg-head">
          <span class="dlg-eyebrow">{dialog.method}</span>
        </div>
        <p class="dlg-message">This dialog kind needs a newer UniPi build — answer it in the terminal for now.</p>
      </div>
    );
  };

  return (
    <Show when={shown()}>
      <div class="dock-dialog" data-testid="dialog-panel" data-collapsed={props.phone && collapsed() ? '' : undefined}>
        <Show
          when={props.phone && collapsed()}
          fallback={
            <Show when={count() > 1 || props.phone}>
            <div class="dock-dialog-bar">
              <Show when={count() > 1}>
                <button type="button" class="dock-dialog-page" aria-label="Previous dialog" disabled={index() === 0} onClick={() => step(-1)}>
                  <ChevronLeft size={16} />
                </button>
                <span class="dock-dialog-count" data-testid="dialog-count">
                  {index() + 1} of {count()}
                </span>
                <button
                  type="button"
                  class="dock-dialog-page"
                  aria-label="Next dialog"
                  disabled={index() >= count() - 1}
                  onClick={() => step(1)}
                >
                  <ChevronRight size={16} />
                </button>
              </Show>
              <span class="flex-1" />
              <Show when={props.phone}>
                <button type="button" class="dock-dialog-collapse" aria-expanded="true" onClick={() => setCollapsed(true)} data-testid="dialog-collapse">
                  Hide <ChevronDown size={15} />
                </button>
              </Show>
            </div>
            </Show>
          }
        >
          <button
            type="button"
            class="dock-dialog-waiting"
            aria-expanded="false"
            onClick={() => setCollapsed(false)}
            data-testid="dialog-collapse"
          >
            <MessageCircleQuestion size={16} class="dock-dialog-waiting-icon" />
            <span class="dock-dialog-waiting-label">Question waiting</span>
            <Show when={count() > 1}>
              <span class="dock-dialog-count">{index() + 1} of {count()}</span>
            </Show>
            <span class="flex-1" />
            <ChevronUp size={16} />
          </button>
        </Show>
        <Show when={!props.phone || !collapsed()}>
          <div class="dock-dialog-body" data-testid="dialog-body">
            {body()}
          </div>
        </Show>
      </div>
    </Show>
  );
}
