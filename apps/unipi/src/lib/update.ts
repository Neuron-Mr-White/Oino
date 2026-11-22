/**
 * In-app updates. One feed (FEED_URL, written by scripts/release.sh) says
 * which build is newest; the Android shell (UpdateBridge.kt, window.UniPiUpdate)
 * reports the installed build, downloads the APK (SHA-256 checked) and opens
 * the system installer. Nothing here needs editing per release: publishing a
 * new feed is the whole release.
 *
 * iOS (UNI-263): the ad hoc build can't update itself. The feed's `app.ios`
 * block says which version is out; the app only shows "open
 * oino.dev/download on this iPhone" (`iosUpdate`).
 */
import { createSignal } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { platform } from './platform';

export const FEED_URL = 'https://oino.dev/app/android/latest.json';
/** Check again after this long (also on app resume). */
const RECHECK_MS = 6 * 60 * 60 * 1000;

export type AppRelease = { version: string; build: number; url: string; sha256: string; size?: number; notes?: string; date?: string };
export type IosRelease = { version: string; url: string; page: string; size?: number };
export type Feed = { app?: AppRelease; host?: { version: string }; ios?: IosRelease };

type Native = {
  info(): string;
  download(url: string, sha256: string, build: number): void;
  install(): string;
  openInstallSettings(): void;
};

export type UpdateState =
  | { kind: 'idle' }
  | { kind: 'checking' }
  | { kind: 'current' }
  | { kind: 'available'; release: AppRelease }
  | { kind: 'downloading'; release: AppRelease; done: number; total: number }
  | { kind: 'ready'; release: AppRelease }
  | { kind: 'error'; message: string; release?: AppRelease };

export type Installed = { versionName: string; versionCode: number; canInstall: boolean };

const native = (): Native | undefined => (window as unknown as { UniPiUpdate?: Native }).UniPiUpdate;

/** The installed build, or null outside the Android app (browser, desktop). */
export function installed(): Installed | null {
  const n = native();
  if (!n) return null;
  try {
    return JSON.parse(n.info()) as Installed;
  } catch {
    return null;
  }
}

const OURS = /^https:\/\/oino\.dev\//;

/** `app.ios` (UNI-263), or undefined when absent/malformed. */
export function parseIos(app: Record<string, unknown> | undefined): IosRelease | undefined {
  const ios = app?.ios as Record<string, unknown> | undefined;
  if (!ios || typeof ios !== 'object') return undefined;
  const ok = typeof ios.version === 'string' && /^\d+\.\d+\.\d+$/.test(ios.version) && typeof ios.url === 'string' && OURS.test(ios.url) && typeof ios.page === 'string' && OURS.test(ios.page);
  return ok ? { version: ios.version as string, url: ios.url as string, page: ios.page as string, size: typeof ios.size === 'number' ? ios.size : undefined } : undefined;
}

/** Strictly newer X.Y.Z? */
export function newerVersion(latest: string, current: string): boolean {
  const p = (v: string) => v.split(/[-+]/)[0].split('.').map((n) => Number.parseInt(n, 10));
  const [a, b] = [p(latest), p(current)];
  if (a.length !== 3 || b.length !== 3 || [...a, ...b].some((n) => !Number.isFinite(n))) return false;
  for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i] > b[i];
  return false;
}

/** Validates a feed; anything malformed means "no update". */
export function parseFeed(value: unknown): Feed | null {
  if (!value || typeof value !== 'object') return null;
  const app = (value as { app?: unknown }).app as Record<string, unknown> | undefined;
  if (!app) return { host: (value as Feed).host };
  const ios = parseIos(app);
  const ok =
    typeof app.version === 'string' &&
    Number.isInteger(app.build) &&
    typeof app.url === 'string' &&
    /^https:\/\/oino\.dev\//.test(app.url) &&
    typeof app.sha256 === 'string' &&
    /^[0-9a-f]{64}$/i.test(app.sha256);
  if (!ok) return ios ? { host: (value as Feed).host, ios } : null;
  return { app: app as unknown as AppRelease, host: (value as Feed).host, ...(ios ? { ios } : {}) };
}

/** Newer only when the feed's build is strictly higher than the installed one. */
export function isNewer(feed: Feed | null, current: Installed | null): AppRelease | null {
  if (!feed?.app || !current) return null;
  return feed.app.build > current.versionCode ? feed.app : null;
}

const [state, setState] = createSignal<UpdateState>({ kind: 'idle' });
export const updateState = state;
let lastCheck = 0;

/** Fetches the feed and updates `updateState`. Quiet: network trouble just means no banner. */
export async function checkForUpdate(force = false): Promise<void> {
  const cur = installed();
  if (!cur) return;
  const s = state();
  if (s.kind === 'downloading' || s.kind === 'ready') return;
  if (!force && Date.now() - lastCheck < RECHECK_MS) return;
  lastCheck = Date.now();
  if (force) setState({ kind: 'checking' });
  try {
    const res = await fetch(`${FEED_URL}?t=${Date.now()}`, { cache: 'no-store' });
    if (!res.ok) throw new Error(`Update check failed (HTTP ${res.status})`);
    const release = isNewer(parseFeed(await res.json()), cur);
    setState(release ? { kind: 'available', release } : { kind: 'current' });
  } catch (error) {
    setState(force ? { kind: 'error', message: String((error as Error).message ?? error) } : { kind: 'idle' });
  }
}

export function startDownload(release: AppRelease): void {
  const n = native();
  if (!n) return;
  setState({ kind: 'downloading', release, done: 0, total: release.size ?? 0 });
  n.download(release.url, release.sha256, release.build);
}

/** Opens the system installer (or Android's "install unknown apps" switch the first time). */
export function installNow(): 'started' | 'permission' | 'missing' {
  const n = native();
  if (!n) return 'missing';
  const r = n.install() as 'started' | 'permission' | 'missing';
  if (r === 'missing') {
    const s = state();
    if (s.kind === 'ready') setState({ kind: 'available', release: s.release });
  }
  return r;
}

/** Events from UpdateBridge.kt. */
(window as unknown as { __unipiUpdateEvent?: (e: Record<string, unknown>) => void }).__unipiUpdateEvent = (e) => {
  const s = state();
  const release = 'release' in s ? s.release : undefined;
  if (!release) return;
  if (e.kind === 'progress') setState({ kind: 'downloading', release, done: Number(e.done) || 0, total: Number(e.total) || release.size || 0 });
  else if (e.kind === 'ready') {
    setState({ kind: 'ready', release });
    installNow();
  } else if (e.kind === 'error') setState({ kind: 'error', message: String(e.message ?? 'Update failed'), release });
};

// ── iOS: notice only ──────────────────────────────────────────────────

const [ios, setIos] = createSignal<{ current?: string; release?: IosRelease; error?: string; checking?: boolean }>({});
export const iosUpdate = ios;
let lastIosCheck = 0;

/** iOS: is a newer ad hoc build out? Quiet unless `force`. */
export async function checkIosUpdate(force = false, current?: string): Promise<void> {
  if (!force && Date.now() - lastIosCheck < RECHECK_MS) return;
  lastIosCheck = Date.now();
  if (force) setIos((s) => ({ ...s, checking: true, error: undefined }));
  try {
    const cur = current ?? ((await invoke('app_info')) as { version: string }).version;
    const res = await fetch(`${FEED_URL}?t=${Date.now()}`, { cache: 'no-store' });
    if (!res.ok) throw new Error(`Update check failed (HTTP ${res.status})`);
    const rel = parseFeed(await res.json())?.ios;
    setIos({ current: cur, release: rel && newerVersion(rel.version, cur) ? rel : undefined });
  } catch (error) {
    setIos((s) => ({ ...s, checking: false, error: force ? String((error as Error).message ?? error) : undefined }));
  }
}

/** Checks at start and whenever the app comes back to the foreground. */
export function watchForUpdates(): void {
  if (platform().os === 'ios' && platform().tauri) {
    setTimeout(() => void checkIosUpdate(), 4000);
    document.addEventListener('visibilitychange', () => {
      if (!document.hidden) void checkIosUpdate();
    });
    return;
  }
  if (!installed()) return;
  setTimeout(() => void checkForUpdate(), 4000);
  document.addEventListener('visibilitychange', () => {
    if (!document.hidden) void checkForUpdate();
  });
}
