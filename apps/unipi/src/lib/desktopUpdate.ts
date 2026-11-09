/**
 * Desktop app self-update (UNI-263). The core (src-tauri app_update.rs, the
 * Tauri updater plugin) reads https://oino.dev/app/desktop/latest.json and
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
const DISMISS_KEY = 'unipi.desktopUpdate.dismissed.latest';
export const SNOOZE_MS = 24 * 60 * 60 * 1000;
type Dismissal = { version: string; until?: number };
function readDismissal(): Dismissal | undefined {
  try { const value = JSON.parse(localStorage.getItem(DISMISS_KEY) ?? 'null'); return value && typeof value.version === 'string' ? value : undefined; } catch { return undefined; }
}
const [dismissal, setDismissal] = createSignal<Dismissal | undefined>(readDismissal());
const dismissed = () => dismissal()?.version;
function saveDismissal(value?: Dismissal): void {
  setDismissal(value);
  try { if (value) localStorage.setItem(DISMISS_KEY, JSON.stringify(value)); else localStorage.removeItem(DISMISS_KEY); } catch { /* storage unavailable */ }
}
/** Semver precedence; invalid versions are never offered. */
export function compareVersions(a: string, b: string): number | undefined {
  const parse = (v: string) => /^(?:v)?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/.exec(v);
  const x = parse(a), y = parse(b);
  if (!x || !y || [x[4], y[4]].some(pre => pre?.split('.').some(id => /^\d+$/.test(id) && id.length > 1 && id[0] === '0'))) return undefined;
  for (let i = 1; i <= 3; i++) { const d = BigInt(x[i]) - BigInt(y[i]); if (d) return d > 0 ? 1 : -1; }
  if (!x[4] || !y[4]) return x[4] ? -1 : y[4] ? 1 : 0;
  const xp = x[4].split('.'), yp = y[4].split('.');
  for (let i = 0; i < Math.max(xp.length, yp.length); i++) {
    if (xp[i] === undefined) return -1; if (yp[i] === undefined) return 1;
    const l = xp[i], r = yp[i]; if (l === r) continue;
    const ln = /^\d+$/.test(l), rn = /^\d+$/.test(r);
    if ((ln && l.length > 1 && l[0] === '0') || (rn && r.length > 1 && r[0] === '0')) return undefined;
    if (ln && rn) return BigInt(l) > BigInt(r) ? 1 : -1;
    if (ln !== rn) return ln ? -1 : 1;
    return l > r ? 1 : -1;
  }
  return 0;
}
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
    const comparison = info.version ? compareVersions(info.version, info.current) : 0;
    if (comparison === undefined) throw new Error('Invalid update version');
    if (comparison > 0) setState({ kind: 'available', info });
    else { saveDismissal(); try { localStorage.removeItem('unipi.updateBanner.dismissed'); } catch { /* unavailable */ } setState({ kind: 'current', info: { ...info, version: undefined } }); }
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
export function dismissDesktopUpdate(skip = false): void {
  const s = state();
  if ('info' in s && s.info?.version) saveDismissal({ version: s.info.version, ...(skip ? {} : { until: Date.now() + SNOOZE_MS }) });
}

/** Should the prompt be open? */
export function promptOpen(s: DesktopUpdateState = state(), later = dismissal()): boolean {
  const hidden = (version?: string) => !!version && !!later && later.version === version && (later.until === undefined || Date.now() < later.until);
  if (s.kind === 'downloading' || s.kind === 'installing') return true;
  if (s.kind === 'error') return !!s.info?.version && !hidden(s.info.version);
  return s.kind === 'available' && !hidden(s.info.version);
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
  setDismissal(readDismissal());
  lastCheck = 0;
}
