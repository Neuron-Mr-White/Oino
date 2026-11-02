/**
 * Getting host files onto the phone / computer or into other apps
 * (UNI-169, UNI-225).
 *
 * Download / Open with… stream the file into the app cache first
 * (`fs_download`: chunked, with progress, no size cap), then hand the
 * cached file off — the same path pushed files take (UNI-214):
 *  - Android: `window.UniPiFiles.saveFile` (→ Download/unipi via
 *    MediaStore) / `openFile` (FileProvider + "Open with…" chooser).
 *  - Desktop: the native Save dialog (`push_save_as`) / the default app
 *    (`push_open`).
 * It used to go through `blob_get_bytes`, the previewer's in-memory read
 * (25 MB cap), so a Download of a bigger file only ever said "That file is
 * too large to preview inline". An older Android bridge without
 * `saveFile`/`openFile` still gets the in-memory base64 path.
 */
import { Channel, invoke } from '@tauri-apps/api/core';
import { blobTransport } from './blob';
import { isTauri, isTauriDesktop } from './platform';

type NativeFilesBridge = {
  saveDownload(name: string, mime: string, base64: string): string;
  openWith(name: string, mime: string, base64: string): string;
  saveFile?: (path: string, name: string, mime: string) => string;
  openFile?: (path: string, name: string, mime: string) => string;
};

function bridge(): NativeFilesBridge | undefined {
  return (window as unknown as { UniPiFiles?: NativeFilesBridge }).UniPiFiles;
}

export type TransferOutcome = 'saved' | 'opened' | 'cancelled' | 'unsupported' | 'failed';

export type TransferResult = { outcome: TransferOutcome; detail?: string };

export type TransferProgress = { done: number; total: number };

/** The core side of a streamed download (tests inject a fake). */
export interface DownloadTransport {
  /** Streams a host file into the app cache; resolves with its local path. */
  fetch(hostId: string, path: string, onProgress?: (p: TransferProgress) => void): Promise<{ localPath: string; size: number; mime: string }>;
  /** Desktop: native Save dialog + copy; `null` when cancelled. */
  saveAs(localPath: string, name: string): Promise<string | null>;
  /** Desktop: open with the default app. */
  open(localPath: string): Promise<void>;
}

const tauriDownload: DownloadTransport = {
  async fetch(hostId, path, onProgress) {
    const channel = new Channel<{ kind: 'progress'; sent: number; total: number }>();
    if (onProgress) channel.onmessage = (m) => onProgress({ done: m.sent, total: m.total });
    return (await invoke('fs_download', { hostId, path, onProgress: channel })) as { localPath: string; size: number; mime: string };
  },
  async saveAs(localPath, name) {
    return (await invoke('push_save_as', { src: localPath, name })) as string | null;
  },
  async open(localPath) {
    await invoke('push_open', { path: localPath });
  },
};

let override: DownloadTransport | undefined;
/** Tests inject a fake. */
export function setDownloadTransport(transport: DownloadTransport | undefined): void {
  override = transport;
}
function downloads(): DownloadTransport | undefined {
  return override ?? (isTauri() ? tauriDownload : undefined);
}

function toBase64(bytes: Uint8Array): string {
  let binary = '';
  const CHUNK = 0x8000; // 32k chars per push, avoids arg-count blowups
  for (let i = 0; i < bytes.length; i += CHUNK) {
    binary += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  }
  return btoa(binary);
}

const baseName = (path: string) => path.split('/').pop() || path;
const message = (error: unknown) => (error as { message?: string })?.message ?? String(error);

/** Old APKs (bridge without saveFile/openFile): the in-memory base64 path. */
async function legacy(native: NativeFilesBridge, hostId: string, path: string, op: 'saveDownload' | 'openWith'): Promise<TransferResult> {
  const got = await blobTransport().getBytes(hostId, { path });
  const parsed = JSON.parse(native[op](baseName(path), got.mime, toBase64(got.bytes))) as { ok: boolean; error?: string; uri?: string };
  return parsed.ok ? { outcome: op === 'saveDownload' ? 'saved' : 'opened', detail: parsed.uri } : { outcome: 'failed', detail: parsed.error };
}

async function transfer(hostId: string, path: string, op: 'save' | 'open', onProgress?: (p: TransferProgress) => void): Promise<TransferResult> {
  const name = baseName(path);
  const dl = downloads();
  try {
    if (isTauriDesktop()) {
      if (!dl) return { outcome: 'unsupported' };
      const got = await dl.fetch(hostId, path, onProgress);
      if (op === 'open') {
        await dl.open(got.localPath);
        return { outcome: 'opened' };
      }
      const saved = await dl.saveAs(got.localPath, name);
      return saved === null ? { outcome: 'cancelled' } : { outcome: 'saved', detail: saved };
    }
    const native = bridge();
    if (!native) return { outcome: 'unsupported' };
    const handOff = op === 'save' ? native.saveFile : native.openFile;
    if (!handOff || !dl) return await legacy(native, hostId, path, op === 'save' ? 'saveDownload' : 'openWith');
    const got = await dl.fetch(hostId, path, onProgress);
    const parsed = JSON.parse(handOff.call(native, got.localPath, name, got.mime)) as { ok: boolean; error?: string; uri?: string };
    if (!parsed.ok) return { outcome: 'failed', detail: parsed.error };
    return { outcome: op === 'save' ? 'saved' : 'opened', detail: parsed.uri };
  } catch (error) {
    return { outcome: 'failed', detail: message(error) };
  }
}

/** Saves a host file to the phone's Downloads (Download/unipi) or, on the desktop, where the Save dialog says. */
export function downloadToDevice(hostId: string, path: string, onProgress?: (p: TransferProgress) => void): Promise<TransferResult> {
  return transfer(hostId, path, 'save', onProgress);
}

/** Hands a host file to another app (system "Open with…" chooser / desktop default app). */
export function openWithExternal(hostId: string, path: string, onProgress?: (p: TransferProgress) => void): Promise<TransferResult> {
  return transfer(hostId, path, 'open', onProgress);
}

/** "Downloading… 42%" / "Downloading… 12.0 of 80.0 MB" for a toast. */
export function progressLabel(verb: string, p: TransferProgress | undefined): string {
  if (!p || p.total <= 0) return `${verb}…`;
  const pct = Math.min(100, Math.floor((p.done / p.total) * 100));
  const mb = (n: number) => (n / 1024 ** 2).toFixed(1);
  return p.total >= 1024 ** 2 ? `${verb}… ${pct}% · ${mb(p.done)} of ${mb(p.total)} MB` : `${verb}… ${pct}%`;
}

/** The toast line for a finished transfer (one wording for Files and the previewer). */
export function transferMessage(result: TransferResult, name: string, desktop: boolean): string {
  switch (result.outcome) {
    case 'saved':
      return desktop && result.detail ? `Saved to ${result.detail}` : `Saved to Download/unipi/${name}`;
    case 'opened':
      return `Opened ${name}`;
    case 'cancelled':
      return 'Download cancelled';
    case 'unsupported':
      return 'Needs the Unipi app';
    default:
      return result.detail ?? 'That did not work';
  }
}

/**
 * Saves bytes made on the phone (a rendered diagram, UNI-182) to Downloads.
 * Native bridge first; else the Web Share / download path; `unsupported`
 * when this WebView can do neither (old APK without the bridge, no share).
 */
export async function saveBytesToDevice(name: string, mime: string, bytes: Uint8Array): Promise<TransferResult> {
  const native = bridge();
  if (native) {
    try {
      const parsed = JSON.parse(native.saveDownload(name, mime, toBase64(bytes))) as { ok: boolean; error?: string; uri?: string };
      return parsed.ok ? { outcome: 'saved', detail: parsed.uri } : { outcome: 'failed', detail: parsed.error };
    } catch (error) {
      return { outcome: 'failed', detail: message(error) };
    }
  }
  const { saveFile } = await import('./board/share');
  const result = await saveFile(new Blob([bytes as BlobPart], { type: mime }), name);
  if (result === 'shared' || result === 'downloaded') return { outcome: 'saved' };
  if (result === 'cancelled') return { outcome: 'failed', detail: 'Cancelled' };
  return { outcome: 'unsupported' };
}
