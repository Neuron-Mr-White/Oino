/** Board toasts / snackbars (UNI-130): bottom-left beside the FAB, one action (Undo, Open…). */
import { For, Show } from 'solid-js';
import { AlertCircle, CheckCircle2, Info, TriangleAlert, Undo2, X } from 'lucide-solid';
import { boardUi, type ToastKind } from '../../lib/board/ui';

const ICON: Record<ToastKind, typeof Info> = { info: Info, success: CheckCircle2, warning: TriangleAlert, error: AlertCircle };

export function BoardToasts() {
  const ui = boardUi();
  return (
    <div class="board-toasts" aria-live="polite">
      <For each={ui.toasts()}>
        {(toast) => {
          const Icon = ICON[toast.kind];
          return (
            <div class="board-toast" data-kind={toast.kind} role={toast.kind === 'error' ? 'alert' : 'status'}>
              <Icon size={18} class="board-toast-icon shrink-0" />
              <span class="min-w-0 flex-1">
                {toast.message}
                <Show when={(toast.count ?? 1) > 1}>
                  <span class="text-text-dim"> ×{toast.count}</span>
                </Show>
              </span>
              <Show when={toast.action}>
                {(action) => (
                  <button
                    type="button"
                    class="board-toast-action"
                    onClick={() => {
                      ui.dismissToast(toast.id);
                      void action().run();
                    }}
                  >
                    <Show when={action().label === 'Undo'}>
                      <Undo2 size={15} />
                    </Show>
                    {action().label}
                  </button>
                )}
              </Show>
              <button type="button" class="board-toast-close" aria-label="Dismiss" onClick={() => ui.dismissToast(toast.id)}>
                <X size={16} />
              </button>
            </div>
          );
        }}
      </For>
    </div>
  );
}
