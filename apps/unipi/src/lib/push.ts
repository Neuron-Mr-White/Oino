/**
 * Files pushed from the PC (UNI-214): `unipi-host push app.apk` on the PC →
 * an "Incoming from <PC>" sheet here → download with progress (`push_fetch`,
 * the core verifies size + sha256) → APK: the system installer (Android asks
 * to confirm); other files: Downloads / Open with… (Android) or Save to… /
 * Open (desktop). The answer goes back with `push_done` so the CLI prints it.
 *
 * Learning about pushes: a long-poll `push_list{wait}` per paired PC while the
 * app is visible (instant), and the host's encrypted ntfy/UnifiedPush
 * notification when the app isn't open (the tap opens the app, the poll then
 * finds it).
 *
 * The UI never calls `invoke()` directly: `pushTransport()` is the seam
 * (tests inject a fake), the Android bits go through `window.UniPiUpdate` /
 * `window.UniPiFiles` (UpdateBridge.kt / FilesBridge.kt).
 */
import { createRoot, createSignal } from 'solid-js';
import { Channel } from '@tauri-apps/api/core';
import { appClient, hostClient } from './host/factory';
import { AppErrorSchema, PushListDataSchema, type AppError, type PushItem, type PushListData, type PushOutcome } from '@unipi/protocol';
import { toAppError } from '@unipi/protocol';
import { can, isTauri, isTauriDesktop, isTauriMobile } from './platform';

export const APK_MIME = 'application/vnd.android.package-archive';
/** What the PC owner runs to switch push on (shown in help). */
export const ENABLE_COMMAND = 'unipi-host push enable';

export type FetchProgress = { done: number; total: number };

export interface PushTransport {
  list(hostId: string, wait: number): Promise<PushListData>;
  fetch(hostId: string, item: PushItem, onProgress: (p: FetchProgress) => void): Promise<{ localPath: string }>;
  done(hostId: string, id: string, outcome: PushOutcome, detail?: string): Promise<void>;
  /** Desktop: native Save dialog; `null` = cancelled. */
  saveAs(localPath: string, name: string): Promise<string | null>;
  /** Desktop: open with the default app. */
  open(localPath: string): Promise<void>;
}

const tauriTransport: PushTransport = {
  async list(hostId, wait) {
    return PushListDataSchema.parse(await hostClient(hostId).request('push_list', { wait }));
  },
  async fetch(hostId, item, onProgress) {
    const channel = new Channel<{ kind: 'progress'; sent: number; total: number }>();
    channel.onmessage = (m) => onProgress({ done: m.sent, total: m.total });
    return (await hostClient(hostId).request('push_fetch', {
      id: item.id,
      path: item.path,
      sha256: item.sha256,
      size: item.size,
      name: item.name,
      onProgress: channel,
    })) as { localPath: string };
  },
  async done(hostId, id, outcome, detail) {
    await hostClient(hostId).request('push_done', { id, outcome, detail });
  },
  async saveAs(localPath, name) {
    return (await appClient().request('push_save_as', { src: localPath, name })) as string | null;
  },
  async open(localPath) {
    await appClient().request('push_open', { path: localPath });
  },
};

let override: PushTransport | undefined;
export function setPushTransport(t: PushTransport | undefined): void {
  override = t;
}
export function pushTransport(): PushTransport {
  return override ?? tauriTransport;
}

// ------------------------------------------------------------- native bits

type UpdateNative = { info(): string; installFile?: (path: string) => string; openInstallSettings(): void };
type FilesNative = { saveFile?: (path: string, name: string, mime: string) => string; openFile?: (path: string, name: string, mime: string) => string };
const updateNative = () => (window as unknown as { UniPiUpdate?: UpdateNative }).UniPiUpdate;
const filesNative = () => (window as unknown as { UniPiFiles?: FilesNative }).UniPiFiles;

/** Android: may UniPi install apps ("install unknown apps")? `null` when unknown. */
export function canInstallApps(): boolean | null {
  const n = updateNative();
  if (!n) return null;
  try {
    return Boolean((JSON.parse(n.info()) as { canInstall?: boolean }).canInstall);
  } catch {
    return null;
  }
}

export function openInstallSettings(): void {
  updateNative()?.openInstallSettings();
}

export const isApk = (item: Pick<PushItem, 'name' | 'mime'>) => item.mime === APK_MIME || /\.apk$/i.test(item.name);

// ------------------------------------------------------------- state

export type PushAction = 'install' | 'save' | 'open';

/** One incoming file and where its sheet is. */
export type SheetState =
  | { kind: 'offer' }
  | { kind: 'downloading'; action: PushAction; done: number; total: number }
  | { kind: 'permission'; localPath: string }
  | { kind: 'verify_failed'; action: PushAction; message: string }
  | { kind: 'error'; action: PushAction; message: string; hint?: string }
  | { kind: 'done'; outcome: PushOutcome; message: string };

export type Incoming = { hostId: string; hostName: string; item: PushItem; state: SheetState };

export const incomingKey = (hostId: string, id: string) => `${hostId}:${id}`;

/** Which buttons the offer shows, primary first (pure, tested). */
export function offerActions(item: PushItem, mode: 'android' | 'desktop' | 'web'): PushAction[] {
  if (mode === 'web') return [];
  if (mode === 'desktop') return ['save', 'open'];
  return isApk(item) ? ['install', 'save'] : ['open', 'save'];
}

export const ACTION_LABEL: Record<PushAction, string> = { install: 'Install', save: 'Save', open: 'Open' };

export function platformMode(): 'android' | 'desktop' | 'web' {
  if (isTauriMobile()) return 'android';
  if (isTauriDesktop()) return 'desktop';
  return 'web';
}

/** Human size: 12.0 MB / 640 KB / 12 B. */
export function humanSize(bytes: number): string {
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
  if (bytes >= 1024 ** 2) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${bytes} B`;
}

/** Maps a fetch failure to the sheet (checksum vs. everything else). */
export function failureState(action: PushAction, error: AppError): SheetState {
  const detail = error.detail as { reason?: string } | undefined;
  if (error.code === 'E_BLOB_REJECTED' && detail?.reason === 'checksum') {
    return { kind: 'verify_failed', action, message: error.message };
  }
  if (error.code === 'E_UNAUTHORIZED' || error.code === 'E_NOT_FOUND') {
    return {
      kind: 'error',
      action,
      message: 'The PC no longer offers this file.',
      hint: 'It was cancelled, moved, or push was switched off on the PC. Push it again.',
    };
  }
  return { kind: 'error', action, message: error.message, hint: error.hint };
}

function createPushStore() {
  const [items, setItems] = createSignal<Incoming[]>([]);
  /** Answered here (push_done may still be in flight / offline): never re-offered. */
  const answered = new Set<string>();
  const set = (key: string, state: SheetState): void => {
    setItems((list) => list.map((i) => (incomingKey(i.hostId, i.item.id) === key ? { ...i, state } : i)));
  };

  /** Replace one PC's pending list (poll result); keeps local sheet state. */
  function setHost(hostId: string, hostName: string, pushes: readonly PushItem[]): void {
    setItems((list) => {
      const others = list.filter((i) => i.hostId !== hostId);
      const mine = list.filter((i) => i.hostId === hostId);
      const fresh = pushes
        .filter((p) => !answered.has(incomingKey(hostId, p.id)))
        .map((p) => mine.find((m) => m.item.id === p.id) ?? { hostId, hostName, item: p, state: { kind: 'offer' } as SheetState });
      // Items finishing locally (done) stay until the user closes them.
      const local = mine.filter((m) => m.state.kind === 'done' && !fresh.some((f) => f.item.id === m.item.id));
      return [...others, ...fresh, ...local];
    });
  }

  function remove(key: string): void {
    setItems((list) => list.filter((i) => incomingKey(i.hostId, i.item.id) !== key));
  }

  return { items, set, setHost, remove, answered };
}

let store: ReturnType<typeof createPushStore> | undefined;
export function pushStore() {
  if (!store) store = createRoot(() => createPushStore());
  return store;
}
/** Tests: a fresh store. */
export function resetPushStore(): void {
  store = undefined;
}

/** The sheet shows the oldest incoming file first. */
export function currentIncoming(): Incoming | undefined {
  return pushStore().items()[0];
}

async function answer(inc: Incoming, outcome: PushOutcome, detail?: string): Promise<void> {
  const key = incomingKey(inc.hostId, inc.item.id);
  pushStore().answered.add(key);
  try {
    await pushTransport().done(inc.hostId, inc.item.id, outcome, detail);
  } catch {
    // Offline: the PC keeps it pending (24 h) — harmless, we won't re-offer it.
  }
}

const errorOf = (e: unknown): AppError => {
  const parsed = AppErrorSchema.safeParse(e);
  return parsed.success ? parsed.data : toAppError(e);
};

/** Starts an action on an incoming file: download → verify → hand off. */
export async function runAction(inc: Incoming, action: PushAction): Promise<void> {
  const s = pushStore();
  const key = incomingKey(inc.hostId, inc.item.id);
  s.set(key, { kind: 'downloading', action, done: 0, total: inc.item.size });
  let localPath: string;
  try {
    ({ localPath } = await pushTransport().fetch(inc.hostId, inc.item, (p) => s.set(key, { kind: 'downloading', action, done: p.done, total: p.total || inc.item.size })));
  } catch (error) {
    s.set(key, failureState(action, errorOf(error)));
    return;
  }
  await handOff(inc, action, localPath);
}

/**
 * After a verified download: install / save / open. The `done` state is set
 * BEFORE telling the PC (`answer`): once the PC drops the push, the next poll
 * removes every non-`done` row, which used to swallow the "Saved to …" line.
 */
export async function handOff(inc: Incoming, action: PushAction, localPath: string): Promise<void> {
  const s = pushStore();
  const key = incomingKey(inc.hostId, inc.item.id);
  const { name, mime } = inc.item;
  const fail = (message: string, hint?: string) => s.set(key, { kind: 'error', action, message, hint });
  try {
    if (action === 'install') {
      const n = updateNative();
      if (!n?.installFile) return fail('This version of UniPi can’t install pushed apps.', 'Update UniPi (Settings → About), then push again.');
      if (canInstallApps() === false) return s.set(key, { kind: 'permission', localPath });
      const r = n.installFile(localPath);
      if (r === 'permission') return s.set(key, { kind: 'permission', localPath });
      if (r !== 'started') return fail(`Couldn’t open the installer (${r}).`);
      s.set(key, { kind: 'done', outcome: 'installed', message: 'Android’s installer is open — confirm the install there.' });
      return await answer(inc, 'installed', 'system installer opened');
    }
    if (isTauriDesktop()) {
      if (action === 'save') {
        const saved = await pushTransport().saveAs(localPath, name);
        if (saved === null) return s.set(key, { kind: 'offer' });
        s.set(key, { kind: 'done', outcome: 'saved', message: `Saved to ${saved}` });
        return await answer(inc, 'saved', saved);
      }
      await pushTransport().open(localPath);
      s.set(key, { kind: 'done', outcome: 'opened', message: `Opened ${name}` });
      return await answer(inc, 'opened');
    }
    const f = filesNative();
    const op = action === 'save' ? f?.saveFile : f?.openFile;
    if (!f || !op) return fail('This version of UniPi can’t receive files.', 'Update UniPi (Settings → About), then push again.');
    const parsed = JSON.parse(op.call(f, localPath, name, mime)) as { ok: boolean; error?: string; uri?: string };
    if (!parsed.ok) return fail(parsed.error ?? 'It didn’t work.');
    if (action === 'save') {
      s.set(key, { kind: 'done', outcome: 'saved', message: `Saved to Downloads/unipi/${name}` });
      return await answer(inc, 'saved', 'Downloads/unipi');
    }
    s.set(key, { kind: 'done', outcome: 'opened', message: `Opened ${name}` });
    return await answer(inc, 'opened');
  } catch (error) {
    fail(errorOf(error).message);
  }
}

/** Permission screen → back from Settings, or "Install" tapped again. */
export async function retryInstall(inc: Incoming): Promise<void> {
  if (inc.state.kind !== 'permission') return;
  if (canInstallApps() === false) return;
  await handOff(inc, 'install', inc.state.localPath);
}

/** Dismiss (or give up after a failure). Closes the sheet. */
export async function dismiss(inc: Incoming): Promise<void> {
  const key = incomingKey(inc.hostId, inc.item.id);
  const failed = inc.state.kind === 'verify_failed' || inc.state.kind === 'error';
  pushStore().remove(key);
  if (inc.state.kind === 'done') return;
  await answer(inc, failed ? 'failed' : 'dismissed', failed ? (inc.state as { message: string }).message : undefined);
}

// ------------------------------------------------------------- poller

/** Whether this runtime can receive pushes at all. */
export function canReceivePushes(): boolean {
  return isTauri() && can('blob');
}

/**
 * Long-polls every paired PC while the app is visible. `getHosts` is
 * re-read every round. Returns a stop function.
 */
export function startPushPoll(getHosts: () => readonly { id: string; name: string }[], waitSeconds = 25): () => void {
  if (!canReceivePushes()) return () => {};
  let stopped = false;
  const running = new Map<string, boolean>();
  const backoff = new Map<string, number>();
  const loop = async (hostId: string) => {
    if (running.get(hostId)) return;
    running.set(hostId, true);
    try {
      while (!stopped && !document.hidden) {
        const host = getHosts().find((h) => h.id === hostId);
        if (!host) break;
        try {
          const data = await pushTransport().list(hostId, waitSeconds);
          backoff.delete(hostId);
          pushStore().setHost(hostId, data.host_name || host.name, data.enabled ? data.pushes : []);
          // Push off on that PC: nothing will come; check again in a while.
          if (!data.enabled) await sleep(60_000);
        } catch {
          const wait = Math.min(60_000, (backoff.get(hostId) ?? 2_000) * 2);
          backoff.set(hostId, wait);
          await sleep(wait);
        }
      }
    } finally {
      running.set(hostId, false);
    }
  };
  const kick = () => {
    if (document.hidden || stopped) return;
    for (const h of getHosts()) void loop(h.id);
  };
  kick();
  const timer = setInterval(kick, 5_000);
  const onVisible = () => kick();
  document.addEventListener('visibilitychange', onVisible);
  return () => {
    stopped = true;
    clearInterval(timer);
    document.removeEventListener('visibilitychange', onVisible);
  };
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
