import { invoke } from '@tauri-apps/api/core';
import { isTauriDesktop } from '../platform';

/** "Restart agent host" (fix3-launch `restart_host`): the desktop app only. */
export function canRestartHost(): boolean {
  return isTauriDesktop();
}

/** Stops/replaces a host that will not answer and starts or adopts a fresh one. */
export async function restartAgentHost(): Promise<boolean> {
  try {
    await invoke('restart_host', {});
    return true;
  } catch {
    return false;
  }
}
