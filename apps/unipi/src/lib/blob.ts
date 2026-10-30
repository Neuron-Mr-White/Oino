/**
 * Blob channel (UNI-157 "media + attachments"): upload/download bytes to a
 * paired PC's `unipi-host` over iroh (`blob_put`/`blob_get`,
 * `crates/proto/src/blob.rs`). Mirrors `src/lib/board/transport.ts`'s
 * seam: the UI never calls `invoke()` directly.
 *
 * Outside Tauri (vite dev in a browser) there is no iroh endpoint, so
 * `devBlobTransport` always rejects; blob features only run in the app.
 */
import { Channel, invoke } from '@tauri-apps/api/core';
import { isTauri } from './platform';

export type BlobProgress = { sent: number; total: number };

export type BlobPutResult = { sha256: string; size: number; deduped: boolean; path: string };
export type BlobGetResult = { size: number; mime: string };
export type BlobGetBytesResult = { mime: string; bytes: Uint8Array };

export interface BlobTransport {
  /** Uploads a local file's bytes; skips the body when the host already
   * has that sha256 (dedup). */
  putFile(hostId: string, path: string, mime: string, name?: string, onProgress?: (p: BlobProgress) => void): Promise<BlobPutResult>;
  /** Uploads in-memory bytes (the composer's shrunk image blobs: no temp
   * file needed); same dedup behaviour as `putFile`. */
  putBytes(hostId: string, bytes: Uint8Array, mime: string, name?: string, onProgress?: (p: BlobProgress) => void): Promise<BlobPutResult>;
  /** UNI-278: a picked `File` of any size — streamed into the app cache in
   * chunks (never whole in memory or as one IPC message), then uploaded from
   * there. Progress covers both halves (staging ≈ first 30%). */
  putPicked?(hostId: string, file: File, onProgress?: (p: BlobProgress) => void): Promise<BlobPutResult>;
  /** Downloads a blob (by sha256) or an allow-listed host path to `destPath`. */
  getFile(
    hostId: string,
    ref: { sha256: string } | { path: string },
    destPath: string,
    onProgress?: (p: BlobProgress) => void,
  ): Promise<BlobGetResult>;
  /** Downloads a blob (by sha256) or an allow-listed path straight into
   * memory (the previewer: no temp file, no extra round trip to re-read it). */
  getBytes(hostId: string, ref: { sha256: string } | { path: string }): Promise<BlobGetBytesResult>;
}

/** 4 MiB per IPC call: measured on a Redmi Note 7 (2019), bigger chunks gained nothing. */
const STAGE_CHUNK = 4 * 1024 * 1024;
/** Share of the progress bar given to staging (the copy into the app cache). */
const STAGE_SHARE = 0.3;

/** Native base64 of a Blob (FileReader data URL) — ~10× faster than a JS loop on a phone. */
function blobBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const url = String(reader.result);
      resolve(url.slice(url.indexOf(',') + 1));
    };
    reader.onerror = () => reject(reader.error ?? new Error('could not read the file'));
    reader.readAsDataURL(blob);
  });
}

/** Bytes kept in memory below this; streamed above (UNI-278). */
export const STREAM_UPLOAD_ABOVE = 8 * 1024 * 1024;

/** Upload a picked file: small ones as bytes (one call), big ones streamed. */
export async function uploadPicked(hostId: string, file: File, onProgress?: (p: BlobProgress) => void): Promise<BlobPutResult> {
  const t = blobTransport();
  if (file.size > STREAM_UPLOAD_ABOVE && t.putPicked) return t.putPicked(hostId, file, onProgress);
  const bytes = new Uint8Array(await file.arrayBuffer());
  return t.putBytes(hostId, bytes, file.type || 'application/octet-stream', file.name, onProgress);
}

function notTauri(): never {
  throw new Error('Blobs need the app (no iroh endpoint in a browser).');
}

export const devBlobTransport: BlobTransport = {
  async putFile() {
    notTauri();
  },
  async putBytes() {
    notTauri();
  },
  async getFile() {
    notTauri();
  },
  async getBytes() {
    notTauri();
  },
};

export const tauriBlobTransport: BlobTransport = {
  async putFile(hostId, path, mime, name, onProgress) {
    const channel = new Channel<{ kind: 'progress'; sent: number; total: number }>();
    if (onProgress) channel.onmessage = (m) => onProgress({ sent: m.sent, total: m.total });
    return (await invoke('blob_put_file', { hostId, path, mime, name, onProgress: channel })) as BlobPutResult;
  },
  async putBytes(hostId, bytes, mime, name, onProgress) {
    const channel = new Channel<{ kind: 'progress'; sent: number; total: number }>();
    if (onProgress) channel.onmessage = (m) => onProgress({ sent: m.sent, total: m.total });
    return (await invoke('blob_put_bytes', { hostId, bytes: Array.from(bytes), mime, name, onProgress: channel })) as BlobPutResult;
  },
  async putPicked(hostId, file, onProgress) {
    const staged = (await invoke('upload_stage_begin', { name: file.name || 'upload.bin' })) as string;
    try {
      const total = Math.max(file.size, 1);
      for (let at = 0; at < file.size; at += STAGE_CHUNK) {
        const piece = file.slice(at, at + STAGE_CHUNK);
        await invoke('upload_stage_chunk', { path: staged, data: await blobBase64(piece) });
        onProgress?.({ sent: Math.round(((at + piece.size) / total) * file.size * STAGE_SHARE), total: file.size });
      }
      return await this.putFile(hostId, staged, file.type || 'application/octet-stream', file.name, (p) =>
        onProgress?.({ sent: Math.round(file.size * STAGE_SHARE + p.sent * (1 - STAGE_SHARE)), total: file.size }),
      );
    } finally {
      void invoke('upload_stage_drop', { path: staged }).catch(() => {});
    }
  },
  async getFile(hostId, ref, destPath, onProgress) {
    const channel = new Channel<{ kind: 'progress'; sent: number; total: number }>();
    if (onProgress) channel.onmessage = (m) => onProgress({ sent: m.sent, total: m.total });
    return (await invoke('blob_get_file', {
      hostId,
      sha256: 'sha256' in ref ? ref.sha256 : undefined,
      path: 'path' in ref ? ref.path : undefined,
      destPath,
      onProgress: channel,
    })) as BlobGetResult;
  },
  async getBytes(hostId, ref) {
    const result = (await invoke('blob_get_bytes', {
      hostId,
      sha256: 'sha256' in ref ? ref.sha256 : undefined,
      path: 'path' in ref ? ref.path : undefined,
    })) as { mime: string; bytes: number[] };
    return { mime: result.mime, bytes: Uint8Array.from(result.bytes) };
  },
};

let override: BlobTransport | undefined;
/** Tests inject a fake transport. */
export function setBlobTransport(transport: BlobTransport | undefined): void {
  override = transport;
}
export function blobTransport(): BlobTransport {
  return override ?? (isTauri() ? tauriBlobTransport : devBlobTransport);
}
