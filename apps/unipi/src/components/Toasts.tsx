/**
 * The app-wide toast stack (WP-A19, Z-85): mounted once by the shell on
 * every surface. One action per toast (Undo / Open…), identical toasts fold
 * ×N, hovering pauses the timer, dismissal exits through a 200 ms fade
 * (none under reduced motion). The board's own toasts land here too.
 */
import { For, Show } from 'solid-js';
import { AlertCircle, CheckCircle2, Info, TriangleAlert, Undo2, X } from 'lucide-solid';
import { toasts, type ToastKind } from '../lib/toasts';
import './toasts.css';

const ICON: Record<ToastKind, typeof Info> = { info: Info, success: CheckCircle2, warning: TriangleAlert, error: AlertCircle };

export function Toasts() {
  const stack = toasts();
  return (
    <div class="app-toasts" aria-live="polite" data-testid="app-toasts">
      <For each={stack.list()}>
        {(toast) => {
          const Icon = ICON[toast.kind];
          return (
            <div
              class="app-toast"
              data-kind={toast.kind}
              data-leaving={toast.leaving ? '' : undefined}
              role={toast.kind === 'error' ? 'alert' : 'status'}
              onPointerEnter={() => stack.pause(toast.id)}
              onPointerLeave={() => stack.resume(toast.id)}
            >
              <Icon size={18} class="app-toast-icon shrink-0" />
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
                    class="app-toast-action"
                    onClick={() => {
                      stack.dismiss(toast.id);
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
              <button type="button" class="app-toast-close" aria-label="Dismiss" onClick={() => stack.dismiss(toast.id)}>
                <X size={16} />
              </button>
            </div>
          );
        }}
      </For>
    </div>
  );
}
