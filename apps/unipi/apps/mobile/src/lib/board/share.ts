/**
 * Getting things off the board on a phone (UNI-131): copy text, open
 * links, save/share attachment files. Tauri's clipboard plugin first
 * (navigator.clipboard is not always granted in the Android WebView),
 * the Web Share API for files where the WebView has it, a plain
 * download anchor in a browser.
 */
import { isTauri, isTauriDesktop, platform } from '../platform';

export async function copyText(text: string): Promise<boolean> {
  try {
    if (isTauri()) {
      const clipboard = await import('@tauri-apps/plugin-clipboard-manager');
      await clipboard.writeText(text);
      return true;
    }
  } catch {
    // fall through to the web API
  }
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

/**
 * Opens an external link outside the app; false when there's no handler
 * (caller copies it instead).
 *
 * UNI-224: in the app, `window.open` is NOT an external browser — the
 * Android WebView has no multi-window support, so it loaded the URL in the
 * app's own WebView, replacing the whole app (and Back then did nothing).
 * Android goes through `UniPiFiles.openUrl` (ACTION_VIEW), the desktop
 * through the core's `open_url` (xdg-open / open / start).
 */
export function openExternal(url: string): boolean {
  if (!/^(https?|mailto):/i.test(url)) return false;
  const native = (window as unknown as { UniPiFiles?: { openUrl?: (url: string) => string } }).UniPiFiles;
  if (native?.openUrl) {
    try {
      return (JSON.parse(native.openUrl(url)) as { ok?: boolean }).ok === true;
    } catch {
      return false;
    }
  }
  // iOS (UNI-263): the core opens Safari (UIApplication openURL).
  if (isTauriDesktop() || (isTauri() && platform().os === 'ios')) {
    void import('@tauri-apps/api/core').then(({ invoke }) => invoke('open_url', { url })).catch((error) => console.warn('[open_url]', error));
    return true;
  }
  // A phone build without the bridge method (old APK): window.open would
  // replace the app — copy the link instead.
  if (isTauri()) return false;
  try {
    const opened = window.open(url, '_blank', 'noopener,noreferrer');
    return opened !== null;
  } catch {
    return false;
  }
}

export type SaveResult = 'shared' | 'downloaded' | 'cancelled' | 'unsupported';

/** Shares (Android share sheet) or downloads a file; `unsupported` when the WebView can do neither. */
export async function saveFile(blob: Blob, name: string): Promise<SaveResult> {
  const file = new File([blob], name, { type: blob.type || 'application/octet-stream' });
  const nav = navigator as Navigator & { canShare?: (data: { files: File[] }) => boolean };
  if (typeof nav.share === 'function' && nav.canShare?.({ files: [file] })) {
    try {
      await nav.share({ files: [file], title: name });
      return 'shared';
    } catch (error) {
      if ((error as { name?: string })?.name === 'AbortError') return 'cancelled';
    }
  }
  // A blob download anchor works in a browser; the Android WebView ignores it.
  if (isTauri()) return 'unsupported';
  const url = URL.createObjectURL(file);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = name;
  anchor.rel = 'noopener';
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  setTimeout(() => URL.revokeObjectURL(url), 30_000);
  return 'downloaded';
}
