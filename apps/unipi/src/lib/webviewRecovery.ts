/**
 * E-13: when the OS kills the page's web process the Tauri core reloads the
 * window (src-tauri/src/webview_recover.rs). The fresh page asks once whether
 * that just happened and says so, instead of looking like a silent restart.
 */
import { invoke } from '@tauri-apps/api/core';
import { toasts } from './toasts';
import { appLog } from './appLog';

export async function announceWebviewRecovery(
  take: () => Promise<string | null | undefined> = () => invoke<string | null>('take_webview_recovery'),
): Promise<boolean> {
  try {
    const reason = await take();
    if (!reason) return false;
    appLog('warn', 'webview', `reloaded after the web process ended: ${reason}`);
    toasts().push(`The view was reloaded because ${reason}. Your chats are unchanged.`, 'info');
    return true;
  } catch {
    return false; // web build / older core: no such command
  }
}
