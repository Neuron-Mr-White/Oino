/** UNI-237 (A6) UI: the real-Quit confirm sheet and "Start at login".
 *
 * Closing the window needs no UI — the Rust side hides it to the tray
 * (decision 2). Quitting does: when the host is the app's own child and
 * phones are paired, the tray/menu/Ctrl+Q path emits
 * `unipi-quit-confirm` and this sheet asks.
 */
import { createSignal, onCleanup, onMount } from 'solid-js';
import { listen } from '@tauri-apps/api/event';
import { call } from '../lib/ipc';
import { can } from '../lib/platform';
import { BoardSheet } from './board/BoardSheet';
import { z } from 'zod';

const QuitPayload = z.object({ message: z.string(), paired: z.number().default(0) });

/** The tray's event, and the capture-only window event that drives it. */
const QUIT_EVENT = 'unipi-quit-confirm';
const QUIT_TEST_EVENT = 'unipi-quit-confirm-test';

/** The "quit UniPi?" sheet; mounted once from the desktop shell. */
export function QuitRunner() {
  if (!can('localSharing')) return null;
  const [payload, setPayload] = createSignal<z.infer<typeof QuitPayload> | undefined>();
  onMount(() => {
    // The Rust side emits this only when quitting really needs asking.
    void listen<{ message: string; paired: number }>(QUIT_EVENT, (event) => setPayload(QuitPayload.parse(event.payload)));
    // Capture hook: the same listener, driven from a Playwright script.
    const drive = (event: Event) => setPayload(QuitPayload.parse((event as CustomEvent<{ message: string; paired: number }>).detail));
    window.addEventListener(QUIT_TEST_EVENT, drive);
    onCleanup(() => window.removeEventListener(QUIT_TEST_EVENT, drive));
  });
  return (
    <BoardSheet open={!!payload()} onClose={() => setPayload(undefined)} label="Quit Unipi?" testId="quit-sheet">
      <p class="preview-muted" style={{ margin: '0 0 12px' }} data-testid="quit-message">
        {payload()?.message ?? "Phones won't reach this PC until you open Unipi again."}
      </p>
      <div class="share-request-actions">
        <button type="button" class="btn btn-danger" data-testid="quit-confirm" onClick={() => void call('quit_now', undefined, z.null())}>
          Quit
        </button>
        <button type="button" class="btn btn-secondary" data-testid="quit-keep" onClick={() => setPayload(undefined)}>
          Keep in tray
        </button>
      </div>
    </BoardSheet>
  );
}

/** "Start at login" (tauri-plugin-autostart): starts hidden in the tray. */
export function useAutostart() {
  const [on, setOn] = createSignal(false);
  const refresh = async () => {
    const r = await call('autostart_state', undefined, z.boolean().default(false));
    if (r.ok) setOn(r.value);
  };
  onMount(() => void refresh());
  const set = async (next: boolean) => {
    setOn(next);
    const r = await call('autostart_set', { enabled: next }, z.boolean());
    if (!r.ok) void refresh();
  };
  onCleanup(() => undefined);
  return { on, set, refresh };
}

/** v2 §5.4 (WP-A24, D-20): the ask-once close dialog. The Rust side emits
 * `unipi-close-ask` when `closeAction` is still `ask` and the window closes;
 * the answer ("Minimize to tray" default, or Quit) may be remembered, which
 * is exactly what Settings → General changes later. */
const CLOSE_ASK_EVENT = 'unipi-close-ask';
const CLOSE_ASK_TEST_EVENT = 'unipi-close-ask-test';

export function CloseAskRunner() {
  if (!can('localSharing')) return null;
  const [open, setOpen] = createSignal(false);
  const [remember, setRemember] = createSignal(true);
  const [message, setMessage] = createSignal('');
  onMount(() => {
    void listen<{ message: string }>(CLOSE_ASK_EVENT, (event) => {
      setMessage(event.payload.message);
      setOpen(true);
    });
    // Capture hook: the same listener, driven from a browser script.
    const drive = (event: Event) => {
      setMessage((event as CustomEvent<{ message: string }>).detail.message);
      setOpen(true);
    };
    window.addEventListener(CLOSE_ASK_TEST_EVENT, drive);
    onCleanup(() => window.removeEventListener(CLOSE_ASK_TEST_EVENT, drive));
  });
  const answer = (action: 'tray' | 'quit') => {
    setOpen(false);
    void call('close_answer', { action, remember: remember() }, z.null());
  };
  return (
    <BoardSheet open={open()} onClose={() => answer('tray')} label="Close Unipi?" testId="close-ask-sheet">
      <p class="preview-muted" style={{ margin: '0 0 12px' }} data-testid="close-ask-message">
        {message() || "Close Unipi, or keep it running in the tray so phones and automations keep working?"}
      </p>
      <label class="flex items-center gap-2 text-sm text-text-muted mb-3" data-testid="close-ask-remember">
        <input type="checkbox" checked={remember()} onChange={(e) => setRemember(e.currentTarget.checked)} />
        Remember my choice
      </label>
      <div class="share-request-actions">
        <button type="button" class="btn btn-primary" data-testid="close-ask-tray" onClick={() => answer('tray')}>
          Minimize to tray
        </button>
        <button type="button" class="btn btn-secondary" data-testid="close-ask-quit" onClick={() => answer('quit')}>
          Quit
        </button>
      </div>
    </BoardSheet>
  );
}
