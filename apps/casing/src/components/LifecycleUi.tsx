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
    <BoardSheet open={!!payload()} onClose={() => setPayload(undefined)} label="Quit UniPi?" testId="quit-sheet">
      <p class="preview-muted" style={{ margin: '0 0 12px' }} data-testid="quit-message">
        {payload()?.message ?? "Phones won't reach this PC until you open UniPi again."}
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
