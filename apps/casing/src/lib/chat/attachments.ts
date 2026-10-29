/**
 * Composer attachments (UNI-157): images are shrunk then sent inline or
 * via blob-upload with the prompt (`resolvePromptImages`); other files
 * ("File" in the "+" menu) upload right away via the blob channel
 * (`uploadGenericFile`).
 *
 * UNI-203: a generic-file attach used to drop a raw `@<host blob path>`
 * straight into the draft text — unreadable, and easy to leave unsent (it
 * just sat there looking like any other typed text). Uploaded files are now
 * tracked as `PendingFile` chips (icon/name/size, spinner while uploading,
 * error state, ✕ to remove) instead; `formatFileMention` is what actually
 * reaches pi, appended to the outgoing text at send time (one per line, see
 * `appendFileMentions`): `[file: <name>]@<host path>` — the bracketed name
 * is a readable hint for the model (the host stores blobs by sha256 only,
 * so the original filename would otherwise be lost) and is also what the
 * bubble renderer (`fileMentionsOf`/Transcript.tsx) matches on to turn the
 * line into a chip instead of showing it as raw text.
 *
 * UNI-212: the chip now lives INLINE in the draft text, as a
 * `[file: <name>]@att:<id>` token at the caret where it was attached
 * (lib/chat/chips.ts); at send the token is swapped in place for the same
 * `[file: <name>]@<host path>` wire mention. `appendFileMentions` remains
 * for files not referenced by a token (an older draft).
 *
 * Pending files must survive leaving the chat without sending: they live in
 * a small per-chat (`hostId:pid`) module-level store (`pendingFilesStore`)
 * that outlives the Composer instance, mirroring how ChatView keeps drafts
 * per chat key.
 */
import { createStore } from 'solid-js/store';
import { blobToBase64, shrinkImage, INLINE_MAX_BYTES } from '../imageShrink';
import { blobTransport, uploadPicked } from '../blob';

/** Cap for non-image "File" attachments in chat (images go through the
 * shrink path instead). UNI-278: 1 GB (was 10 MB) — uploads stream now. */
export const MAX_GENERIC_FILE_BYTES = 1024 * 1024 * 1024;
const MAX_GENERIC_LABEL = '1 GB';

export type PendingImage = {
  id: string;
  name: string;
  mime: string;
  blob: Blob;
  /** Object URL for the chip thumbnail; caller revokes it when done. */
  previewUrl: string;
};

export type PromptImage = { mime: string; data: string } | { mime: string; path: string };

let seq = 0;
/** UNI-285: a pending image's id, minted up front so its draft chip token
 *  (`[image]@img:<id>`) can go in at the caret before the shrink finishes. */
export function newPendingImageId(): string {
  return `img${Date.now().toString(36)}${(++seq).toString(36)}`;
}

export async function addPendingImage(file: File, id: string = newPendingImageId()): Promise<PendingImage> {
  const shrunk = await shrinkImage(file);
  return {
    id,
    name: file.name,
    mime: shrunk.mime,
    blob: shrunk.blob,
    previewUrl: URL.createObjectURL(shrunk.blob),
  };
}

/** Turns pending images into the wire payload: small enough combined (≤
 * `INLINE_MAX_BYTES`) goes inline as base64; otherwise each image is
 * uploaded via the blob channel and referenced by its host path. Mixed
 * (some inline, some by path) is fine — the bridge accepts either per
 * image. */
export async function resolvePromptImages(hostId: string, images: PendingImage[]): Promise<PromptImage[]> {
  const total = images.reduce((sum, i) => sum + i.blob.size, 0);
  if (total <= INLINE_MAX_BYTES) {
    return Promise.all(
      images.map(async (i) => ({ mime: i.mime, data: await blobToBase64(i.blob) })),
    );
  }
  const out: PromptImage[] = [];
  for (const image of images) {
    const bytes = new Uint8Array(await image.blob.arrayBuffer());
    const result = await blobTransport().putBytes(hostId, bytes, image.mime, image.name);
    out.push({ mime: image.mime, path: result.path });
  }
  return out;
}

/** Uploads a non-image file (≤ `MAX_GENERIC_FILE_BYTES`) via the blob
 * channel and returns its host path, to be mentioned in the prompt as
 * `@<path>` so pi can read it straight off disk. */
export async function uploadGenericFile(hostId: string, file: File): Promise<string> {
  if (file.size > MAX_GENERIC_FILE_BYTES) {
    throw new Error(`${file.name} is larger than ${MAX_GENERIC_LABEL}`);
  }
  const result = await uploadPicked(hostId, file);
  return result.path;
}

// ── generic-file chips (UNI-203) ──────────────────────────────────────

export type PendingFile = {
  id: string;
  name: string;
  mime: string;
  size: number;
  status: 'uploading' | 'done' | 'error';
  /** Set once `status` is `'done'`. */
  path?: string;
  /** Set once `status` is `'error'`. */
  error?: string;
};

/** `[file: <name>]@<host path>` — one mention line appended to the
 * outgoing text per uploaded file (UNI-203). Bracketed name first so a
 * model reading raw text still sees a readable filename next to the path
 * it must use to read the file; `fileMentionsOf` below is the inverse. */
export function formatFileMention(file: Pick<PendingFile, 'name' | 'path'>): string {
  return `[file: ${file.name}]@${file.path ?? ''}`;
}

const FILE_MENTION_RE = /\[file: ([^\]]*)\]@(\S+)/g;

/** One `[file: name]@path` mention found in a message's text (UNI-203),
 * for the bubble renderer to turn into a chip instead of raw text. */
export type FileMention = { name: string; path: string; start: number; end: number };

export function fileMentionsOf(text: string): FileMention[] {
  const out: FileMention[] = [];
  for (const m of text.matchAll(FILE_MENTION_RE)) {
    out.push({ name: m[1]!, path: m[2]!, start: m.index!, end: m.index! + m[0].length });
  }
  return out;
}

/** The text with every `[file: name]@path` mention (and the blank line(s)
 * that isolate them) stripped — what the bubble shows as its prose. */
export function stripFileMentions(text: string): string {
  return text.replace(FILE_MENTION_RE, '').replace(/\n{3,}/g, '\n\n').trim();
}

/** Appends one mention line per uploaded (`status: 'done'`) file to
 * `text`, blank-line separated — what actually reaches pi (UNI-203); files
 * still uploading or failed are never sent (the caller should block send
 * until uploads settle, or drop them). */
export function appendFileMentions(text: string, files: readonly PendingFile[]): string {
  const done = files.filter((f): f is PendingFile & { path: string } => f.status === 'done' && !!f.path);
  if (!done.length) return text;
  const mentions = done.map((f) => formatFileMention(f)).join('\n');
  return text.trim() ? `${text}\n\n${mentions}` : mentions;
}

/** Per-chat (`hostId:pid`) pending-file chips, kept outside the Composer
 * component so they survive leaving the chat without sending (UNI-203) —
 * ChatView already does the same for drafts, keyed the same way, but owns
 * that file; this is the attachments-only equivalent living here instead. */
const pendingFilesStore = new Map<string, ReturnType<typeof createStore<PendingFile[]>>>();

function storeFor(chatKey: string) {
  let entry = pendingFilesStore.get(chatKey);
  if (!entry) {
    entry = createStore<PendingFile[]>([]);
    pendingFilesStore.set(chatKey, entry);
  }
  return entry;
}

export function pendingFilesFor(chatKey: string): PendingFile[] {
  return storeFor(chatKey)[0];
}

export function addPendingFile(chatKey: string, file: Pick<PendingFile, 'id' | 'name' | 'mime' | 'size' | 'status'>): void {
  const [, setFiles] = storeFor(chatKey);
  setFiles((prev) => [...prev, { ...file }]);
}

export function updatePendingFile(chatKey: string, id: string, patch: Partial<PendingFile>): void {
  const [, setFiles] = storeFor(chatKey);
  setFiles((prev) => prev.map((f) => (f.id === id ? { ...f, ...patch } : f)));
}

export function removePendingFile(chatKey: string, id: string): void {
  const [, setFiles] = storeFor(chatKey);
  setFiles((prev) => prev.filter((f) => f.id !== id));
}

/** Clears every pending-file chip for a chat (after a successful send). */
export function clearPendingFiles(chatKey: string): void {
  const [, setFiles] = storeFor(chatKey);
  setFiles([]);
}

let fileSeq = 0;
/** A fresh pending-file id (also what an inline `[file: …]@att:<id>`
 * draft token points at, UNI-212). */
export function newPendingFileId(): string {
  return `file${Date.now().toString(36)}${(++fileSeq).toString(36)}`;
}

/** The pending file `id` in a chat (UNI-212: inline chips look theirs up). */
export function pendingFileById(chatKey: string, id: string): PendingFile | undefined {
  return storeFor(chatKey)[0].find((f) => f.id === id);
}

/** Uploads `file` right away, tracking it as a chip (`pendingFilesFor`)
 * through the upload — `uploading` → `done`/`error` — instead of inserting
 * raw text into the draft (UNI-203). UNI-212: the caller may pass the id
 * (its inline draft token already points at it). */
export async function attachGenericFile(chatKey: string, hostId: string, file: File, id: string = newPendingFileId()): Promise<void> {
  addPendingFile(chatKey, { id, name: file.name, mime: file.type || 'application/octet-stream', size: file.size, status: 'uploading' });
  try {
    const path = await uploadGenericFile(hostId, file);
    updatePendingFile(chatKey, id, { status: 'done', path });
  } catch (error) {
    updatePendingFile(chatKey, id, { status: 'error', error: error instanceof Error ? error.message : String(error) });
  }
}

/** UNI-256: a local file the desktop app got from the OS (dropped / copied in
 * the file manager): uploaded straight from its path (the core streams it,
 * no bytes through the UI), tracked as a chip like `attachGenericFile`. */
export async function attachLocalFile(
  chatKey: string,
  hostId: string,
  file: { path: string; name: string; size: number; mime: string },
  id: string = newPendingFileId(),
): Promise<void> {
  addPendingFile(chatKey, { id, name: file.name, mime: file.mime, size: file.size, status: 'uploading' });
  try {
    if (file.size > MAX_GENERIC_FILE_BYTES) throw new Error(`${file.name} is larger than ${MAX_GENERIC_LABEL}`);
    const result = await blobTransport().putFile(hostId, file.path, file.mime, file.name);
    updatePendingFile(chatKey, id, { status: 'done', path: result.path });
  } catch (error) {
    updatePendingFile(chatKey, id, { status: 'error', error: error instanceof Error ? error.message : String((error as { message?: string })?.message ?? error) });
  }
}
