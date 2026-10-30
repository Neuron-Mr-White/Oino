/**
 * Full-screen QR scan UI (DESIGN §3.2) drawn over the camera preview.
 *
 * The barcode plugin runs `scan({ windowed: true })`: its camera
 * preview sits *behind* the WebView and the WebView is made
 * transparent, so everything here is our own HTML — a dimmed scrim
 * with a square cut-out, corner brackets, a hint and a Close button.
 * While it is mounted, `html.qr-scanning` makes the page backgrounds
 * transparent and hides the rest of the app (see style.css).
 *
 * Android Back cancels (`onBackButtonPress`), as does Close.
 */
import { can, isTauri } from '../lib/platform';
import { onCleanup, onMount } from 'solid-js';
import { X } from 'lucide-solid';

export const SCANNING_CLASS = 'qr-scanning';

export function QrScanner(props: { onClose: () => void }) {
  onMount(() => {
    document.documentElement.classList.add(SCANNING_CLASS);
    let unregister: (() => void) | undefined;
    let disposed = false;
    if (isTauri() && can('backButton')) {
      void (async () => {
        try {
          const { onBackButtonPress } = await import('@tauri-apps/api/app');
          const listener = await onBackButtonPress(() => props.onClose());
          if (disposed) void listener.unregister();
          else unregister = () => void listener.unregister();
        } catch (e) {
          console.warn('[scan] back button listener unavailable', e);
        }
      })();
    }
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') props.onClose();
    };
    window.addEventListener('keydown', onKey);
    onCleanup(() => {
      disposed = true;
      document.documentElement.classList.remove(SCANNING_CLASS);
      window.removeEventListener('keydown', onKey);
      unregister?.();
    });
  });

  return (
    <div class="qr-scan" role="dialog" aria-modal="true" aria-label="Scan pairing QR code">
      <div class="qr-scan-top">
        <p class="qr-scan-title">Scan pairing code</p>
        <p class="qr-scan-hint">
          Run <code>unipi-host pair</code> on your computer and fit the code inside the frame.
        </p>
      </div>
      <div class="qr-scan-window" data-testid="qr-viewfinder">
        <span class="qr-corner qr-corner-tl" />
        <span class="qr-corner qr-corner-tr" />
        <span class="qr-corner qr-corner-bl" />
        <span class="qr-corner qr-corner-br" />
        <span class="qr-scan-line" />
      </div>
      <div class="qr-scan-bottom">
        <button type="button" class="qr-scan-close" onClick={() => props.onClose()}>
          <X size={18} /> Close
        </button>
      </div>
    </div>
  );
}
