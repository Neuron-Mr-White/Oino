/**
 * Desktop app self-update (UNI-263). The core (src-tauri app_update.rs, the
 * Tauri updater plugin) reads https://unipi.nrn.one/desktop/latest.json and
 * verifies the download's signature; this module decides *when* to ask:
 *
 * - checks a few seconds after start and every RECHECK_MS (plus Settings →
 *   About → Check for updates),
 * - never installs on its own: a newer version opens a prompt, and only
 *   "Update and restart" installs it (user decision: update when the user
 *   agrees). "Later" hides the prompt for that version until the next start;
 *   the card in Settings stays,
 * - .deb / .rpm / tar.gz installs (`kind: 'manual'`) can't replace
 *   themselves: they get "Download" (the download page) instead.
 */
import { Channel, invoke } from '@tauri-apps/api/core';
import { createSignal } from 'solid-js';
import { z } from 'zod';
import { can } from './platform';

export const RECHECK_MS = 4 * 60 * 60 * 1000;
export const FIRST_CHECK_MS = 8000;

export const UpdateInfoSchema = z.object({
  current: z.string(),
  kind: z.enum(['in_app', 'manual']),
  version: z.string().optional(),
  notes: z.string().optional(),
  date: z.string().optional(),
  downloadPage: z.string(),
});
export type UpdateInfo = z.infer<typeof UpdateInfoSchema>;

export type DesktopUpdateState =
  | { kind: 'idle' }
  | { kind: 'checking' }
  | { kind: 'current'; info: UpdateInfo }
  | { kind: 'available'; info: UpdateInfo }
  | { kind: 'downloading'; info: UpdateInfo; done: number; total?: number }
  | { kind: 'installing'; info: UpdateInfo }
  | { kind: 'error'; message: string; info?: UpdateInfo };

type Progress = { kind: 'progress'; done: number; total?: number | null } | { kind: 'verifying' } | { kind: 'installing' };

/** The core calls (swapped in tests / captures). */
export interface DesktopUpdateTransport {
  check(): Promise<unknown>;
  install(expect: string, onProgress: (p: Progress) => void): Promise<void>;
}

const tauriTransport: DesktopUpdateTransport = {
  check: () => invoke('app_update_check'),
  install: (expect, onProgress) => {
    const channel = new Channel<Progress>();
    channel.onmessage = onProgress;
    return invoke('app_update_install', { expect, onProgress: channel });
  },
};
let transport = tauriTransport;
export function setDesktopUpdateTransport(t: DesktopUpdateTransport | undefined): void {
  transport = t ?? tauriTransport;
}

const [state, setState] = createSignal<DesktopUpdateState>({ kind: 'idle' });
export const desktopUpdateState = state;
/** The version whose prompt the user closed with "Later" (this run only). */
const [dismissed, setDismissed] = createSignal<string | undefined>();
export const dismissedVersion = dismissed;

let lastCheck = 0;

function errorText(error: unknown): string {
  if (error && typeof error === 'object' && 'message' in error) return String((error as { message: unknown }).message);
  return String(error);
}

/** Feed check. Quiet unless `force` (Settings button): a failed background check leaves the state alone. */
export async function checkDesktopUpdate(force = false, now = Date.now()): Promise<void> {
  const s = state();
  if (s.kind === 'downloading' || s.kind === 'installing') return;
  if (!force && now - lastCheck < RECHECK_MS) return;
  lastCheck = now;
  if (force) setState({ kind: 'checking' });
  try {
    const parsed = UpdateInfoSchema.safeParse(await transport.check());
    if (!parsed.success) throw new Error('Unexpected answer from the update check');
    const info = parsed.data;
    setState(info.version ? { kind: 'available', info } : { kind: 'current', info });
  } catch (error) {
    if (force) setState({ kind: 'error', message: errorText(error) });
    else if (s.kind === 'checking') setState({ kind: 'idle' });
  }
}

/** The user said yes. Download (signature-checked in the core), install, relaunch. */
export async function installDesktopUpdate(): Promise<void> {
  const s = state();
  if (s.kind !== 'available' && !(s.kind === 'error' && s.info?.version)) return;
  const info = s.info!;
  if (info.kind !== 'in_app' || !info.version) return;
  setState({ kind: 'downloading', info, done: 0 });
  try {
    await transport.install(info.version, (p) => {
      if (p.kind === 'progress') setState({ kind: 'downloading', info, done: p.done, total: p.total ?? undefined });
      else setState({ kind: 'installing', info });
    });
    // The core relaunches the app; reaching here means it didn't.
    setState({ kind: 'installing', info });
  } catch (error) {
    setState({ kind: 'error', message: errorText(error), info });
  }
}

/** "Later": hide the prompt for this version (Settings still shows it). */
export function dismissDesktopUpdate(): void {
  const s = state();
  if ('info' in s && s.info?.version) setDismissed(s.info.version);
}

/** Should the prompt be open? */
export function promptOpen(s: DesktopUpdateState = state(), later = dismissed()): boolean {
  if (s.kind === 'downloading' || s.kind === 'installing') return true;
  if (s.kind === 'error') return !!s.info?.version && s.info.version !== later;
  return s.kind === 'available' && s.info.version !== later;
}

/** Desktop app only: first check soon after start, then every RECHECK_MS. */
export function watchDesktopUpdates(): () => void {
  if (!can('desktopUpdater')) return () => {};
  const first = setTimeout(() => void checkDesktopUpdate(), FIRST_CHECK_MS);
  const timer = setInterval(() => void checkDesktopUpdate(), RECHECK_MS + 1000);
  return () => {
    clearTimeout(first);
    clearInterval(timer);
  };
}

/** Test hook. */
export function resetDesktopUpdate(): void {
  setState({ kind: 'idle' });
  setDismissed(undefined);
  lastCheck = 0;
}
