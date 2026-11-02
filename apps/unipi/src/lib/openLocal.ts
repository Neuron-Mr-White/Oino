/**
 * Files on the user's own PC (owner5 #23): the desktop app can reveal them
 * in the file manager, open them with the default app or an editor found on
 * the PC. A remote host (or the phone) keeps Download.
 */
import { invoke } from '@tauri-apps/api/core';
import { LOCAL_HOST } from './host/client';
import { isTauriDesktop } from './platform';

export type OpenTarget = { id: string; name: string };

/** Is this file on the machine the app itself runs on? */
export function isOwnPcFile(hostId: string): boolean {
  return hostId === LOCAL_HOST && isTauriDesktop();
}

let editors: Promise<OpenTarget[]> | undefined;
/** Editors detected on this PC (PATH); cached for the session. */
export function localEditors(): Promise<OpenTarget[]> {
  editors ??= invoke<OpenTarget[]>('local_open_targets').catch(() => []);
  return editors;
}

/** `how`: `reveal` | `default` | an editor id. */
export async function localOpen(path: string, how: string): Promise<{ ok: true } | { ok: false; message: string }> {
  try {
    await invoke('local_open', { path, how });
    return { ok: true };
  } catch (e) {
    return { ok: false, message: (e as { message?: string })?.message ?? 'Could not open this file' };
  }
}

/** Native folder dialog (desktop, own PC). `null` = cancelled. */
export async function pickFolderNative(start?: string): Promise<string | null> {
  return (await invoke('pick_folder', { start })) as string | null;
}
