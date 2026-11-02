/**
 * The app-wide toast stack (WP-A19, Z-85): mounted once by the shell on
 * every surface. Compact (≤ 360 px, 13 px text), stacked bottom-right of the
 * conversation area above the composer so it never covers the input. One
 * optional action per toast (Undo / Open…), identical toasts fold ×N, click
 * or ✕ dismisses, hovering pauses the timer, and at most 3 show at once.
 */
import { createEffect, For, onCleanup, Show } from 'solid-js';
import { AlertCircle, CheckCircle2, Info, TriangleAlert, Undo2, X } from 'lucide-solid';
import { toasts, type ToastKind } from '../lib/toasts';
import './toasts.css';

const ICON: Record<ToastKind, typeof Info> = { info: Info, success: CheckCircle2, warning: TriangleAlert, error: AlertCircle };

/** P2-04 (§3.2): on the phone, a bottom sheet / full-screen layer owns the bottom edge — the stack moves to the top. */
export function phoneSheetOpen(doc: Document = document): boolean {
  if (doc.documentElement.dataset.shell !== 'phone') return false;
  return !!doc.querySelector('.sheet, .ui-phone-pane, .browser-sheet');
}

/** Computes right/bottom offsets so toasts stay bottom-right of `.conv` and above the composer. */
function measureToastOffsets(): { right: number; bottom: number; top?: boolean } {
  if (typeof window === 'undefined' || typeof document === 'undefined') return { right: 16, bottom: 96 };
  const vw = window.innerWidth || 1440;
  const vh = window.innerHeight || 900;
  const phone = document.documentElement.dataset.shell === 'phone';
  const edgePad = phone ? 12 : 16;
  if (phoneSheetOpen()) return { right: edgePad, bottom: 0, top: true };

  const conv = document.querySelector<HTMLElement>('.ws-layer .conv, .conv');
  const convRect = conv?.getBoundingClientRect();
  const right =
    convRect && convRect.width > 0
      ? Math.max(edgePad, Math.round(vw - convRect.right + edgePad))
      : edgePad;

  const dock = document.querySelector<HTMLElement>('.ws-layer .dock, .conv .dock');
  const dockComposer = dock?.querySelector<HTMLElement>('.tl-composer') ?? dock;
  if (dockComposer) {
    const r = dockComposer.getBoundingClientRect();
    const dockRect = dock?.getBoundingClientRect();
    if (r.height > 0 && r.top < vh) {
      const top = dockRect && dockRect.height > 0 && dockRect.top > 180 ? Math.min(dockRect.top, r.top) : r.top;
      return { right, bottom: Math.max(edgePad, Math.round(vh - top + 10)) };
    }
  }

  const draftComposer = document.querySelector<HTMLElement>('.draft-composer .tl-composer, .composer-card');
  if (draftComposer) {
    const r = draftComposer.getBoundingClientRect();
    if (r.height > 0 && r.top < vh) {
      if (vh - r.bottom < 160) {
        return { right, bottom: Math.max(edgePad, Math.round(vh - r.top + 10)) };
      }
      return { right, bottom: phone ? 76 : 36 };
    }
  }

  return { right, bottom: phone ? 76 : 20 };
}

export function Toasts() {
  const stack = toasts();
  let rootEl: HTMLDivElement | undefined;

  // Only observe layout while at least one toast is shown (zero cost while idle/streaming).
  createEffect(() => {
    if (stack.list().length === 0 || !rootEl || typeof window === 'undefined') return;
    const el = rootEl;
    const sync = () => {
      const { right, bottom, top } = measureToastOffsets();
      el.style.setProperty('--toast-right', `${right}px`);
      el.style.setProperty('--toast-bottom', `${bottom}px`);
      if (top) el.dataset.pos = 'top';
      else delete el.dataset.pos;
    };
    sync();
    window.addEventListener('resize', sync, { passive: true });
    window.addEventListener('hashchange', sync);
    const ro = typeof ResizeObserver !== 'undefined' ? new ResizeObserver(sync) : undefined;
    const observeTargets = () => {
      if (!ro) return;
      ro.disconnect();
      for (const target of document.querySelectorAll('.conv, .dock, .tl-composer, .draft-composer, .composer-card')) {
        ro.observe(target);
      }
    };
    observeTargets();
    const mo =
      typeof MutationObserver !== 'undefined'
        ? new MutationObserver(() => {
            observeTargets();
            sync();
          })
        : undefined;
    mo?.observe(document.body, { childList: true, subtree: true });
    onCleanup(() => {
      window.removeEventListener('resize', sync);
      window.removeEventListener('hashchange', sync);
      mo?.disconnect();
      ro?.disconnect();
    });
  });

  return (
    <div ref={rootEl} class="app-toasts" data-testid="app-toasts">
      <For each={stack.list()}>
        {(toast) => {
          const Icon = ICON[toast.kind];
          return (
            <div
              class="app-toast"
              data-kind={toast.kind}
              data-leaving={toast.leaving ? '' : undefined}
              role={toast.kind === 'error' ? 'alert' : 'status'}
              onClick={() => stack.dismiss(toast.id)}
              onPointerEnter={() => stack.pause(toast.id)}
              onPointerLeave={() => stack.resume(toast.id)}
            >
              <Icon size={15} class="app-toast-icon shrink-0" />
              <span class="app-toast-msg">
                {toast.message}
                <Show when={(toast.count ?? 1) > 1}>
                  <span class="app-toast-count"> ×{toast.count}</span>
                </Show>
              </span>
              <Show when={toast.action}>
                {(action) => (
                  <button
                    type="button"
                    class="app-toast-action"
                    onClick={(e) => {
                      e.stopPropagation();
                      stack.dismiss(toast.id);
                      void action().run();
                    }}
                  >
                    <Show when={action().label === 'Undo'}>
                      <Undo2 size={13} />
                    </Show>
                    {action().label}
                  </button>
                )}
              </Show>
              <button
                type="button"
                class="app-toast-close"
                aria-label="Dismiss"
                onClick={(e) => {
                  e.stopPropagation();
                  stack.dismiss(toast.id);
                }}
              >
                <X size={13} />
              </button>
            </div>
          );
        }}
      </For>
    </div>
  );
}
