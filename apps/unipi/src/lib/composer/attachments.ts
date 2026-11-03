/**
 * Composer attachments (WP-A13; Z-60 states, D-6 per-chat store).
 *
 * Two kinds, both **pre-uploaded on attach** and both living in the same
 * per-chat store — module level, keyed like `drafts.ts`, so a half-uploaded
 * or annotated attachment survives leaving the chat and coming back (D-6:
 * this replaces the Composer's local `images` signal; the store sits beside
 * the old `pendingFiles` one, which it subsumes):
 *
 *   image  shrunk + base64-encoded here (the prompt wire only carries inline
 *          image bytes), annotated in the image editor, then `ready`;
 *   file   uploaded over the blob channel with real progress, referenced in
 *          the prompt as `[file: <name>]@<host path>` (unipi's mention).
 *
 * State machine (ZCode `docs/ui/chat-file-attachments.md`, the Z-60 hybrid):
 *
 *   queued → uploading (%) → ready
 *                 │
 *                 ├─ transient failure → 500 ms, one automatic retry
 *                 └─ → failed → user retry (a fresh retry budget)
 *
 * Concurrency is at most 2 uploads in flight; progress is monotonic and
 * stops at 99 % until the upload commits; at most `MAX_ATTACHMENTS` (8)
 * attachments per chat; the composer's send is gated until every attachment
 * is `ready` (`attachmentsGate`).
 */
import { createStore, produce } from 'solid-js/store';
import { blobToBase64, shrinkImage } from '../imageShrink';
import { uploadPicked } from '../blob';
import type { OutboxImage } from '../session/types';

/** Z-60: at most 8 attachments per chat. */
export const MAX_ATTACHMENTS = 8;
/** Z-60: one automatic retry, then the user's manual retry. */
export const MAX_AUTO_RETRIES = 1;
/** Z-60: wait before the automatic retry. */
export const AUTO_RETRY_DELAY_MS = 500;
/** Two uploads in flight (ZCode: 上传并发上限为 2). */
export const MAX_UPLOADS = 2;
/** Progress stops here until the upload commits (ZCode). */
export const PROGRESS_CEILING = 0.99;

/** Cap for non-image "File" attachments (images go through the shrink path instead). */
export const MAX_GENERIC_FILE_BYTES = 1024 * 1024 * 1024;
const MAX_GENERIC_LABEL = '1 GB';

export type AttachmentStatus = 'queued' | 'uploading' | 'ready' | 'failed';

export type PendingAttachment = {
  id: string;
  kind: 'image' | 'file';
  name: string;
  mime: string;
  size: number;
  status: AttachmentStatus;
  /** 0…1, monotonic, ≤ `PROGRESS_CEILING` until committed then 1. */
  progress: number;
  /** `data:` URL — the image chip's thumbnail and, once ready, the payload that is sent. */
  preview?: string;
  /** A ready image's base64 bytes (the wire's `ImageContent.data`). */
  data?: string;
  /** A ready generic file's host path (the `[file: …]@path` mention). */
  path?: string;
  /** Why the last attempt failed (`status: 'failed'`). */
  error?: string;
  /** Automatic retries already spent (reset by a manual retry). */
  autoRetries: number;
  /** The picked bytes, kept only so a failed image/file can retry (never sent). */
  source?: Blob;
  /** Its inline chip was put in the draft (owner spec 2026-11-03). From then on only a chip in the
   *  text sends it: a deleted chip leaves it unsent (kept for Undo, pruned when room is needed). */
  placed?: boolean;
};

/** What the composer's send gate says about a chat right now. */
export type AttachmentsGate = { ok: boolean; uploading: number; failed: number };

let seq = 0;
const nextId = (kind: PendingAttachment['kind']) => `${kind}${Date.now().toString(36)}${(++seq).toString(36)}`;

/* ── the per-chat store (D-6) ────────────────────────────────────────── */

type ChatStore = { items: PendingAttachment[] };

const stores = new Map<string, ReturnType<typeof createStore<ChatStore>>>();
/** Upload/encode jobs, in arrival order (`MAX_UPLOADS` run at a time). */
const queue = new Set<string>();
const jobs = new Map<string, () => Promise<void>>();
let running = 0;
const retryTimers = new Map<string, ReturnType<typeof setTimeout>>();

function storeFor(chatKey: string) {
  let entry = stores.get(chatKey);
  if (!entry) {
    entry = createStore<ChatStore>({ items: [] });
    stores.set(chatKey, entry);
  }
  return entry;
}

/** A chat's pending attachments, in attach order (images and files together). */
export function pendingAttachmentsFor(chatKey: string): PendingAttachment[] {
  return storeFor(chatKey)[0].items;
}

/** Patches one attachment in place — the item object stays the same, so a
 *  `<For>` over the store does not re-create the chip on every progress tick. */
function update(chatKey: string, id: string, patch: Partial<PendingAttachment>): void {
  const [, set] = storeFor(chatKey);
  set(
    produce((state: ChatStore) => {
      const item = state.items.find((a) => a.id === id);
      if (item) Object.assign(item, patch);
    }),
  );
}

/** Marks attachments whose inline chip is now in the draft. */
export function markPlaced(chatKey: string, ids: readonly string[]): void {
  if (!ids.length) return;
  const [, set] = storeFor(chatKey);
  set(
    produce((state: ChatStore) => {
      for (const item of state.items) if (ids.includes(item.id)) item.placed = true;
    }),
  );
}

/** Drops placed attachments no chip names any more (their chip was deleted), making room. */
export function prunePlaced(chatKey: string, referenced: ReadonlySet<string>): number {
  const gone = pendingAttachmentsFor(chatKey).filter((a) => a.placed && !referenced.has(a.id));
  for (const a of gone) removeAttachment(chatKey, a.id);
  return gone.length;
}

/** The attachment `id` in a chat. */
export function attachmentById(chatKey: string, id: string): PendingAttachment | undefined {
  return storeFor(chatKey)[0].items.find((a) => a.id === id);
}

function push(chatKey: string, attachment: PendingAttachment): void {
  const [, set] = storeFor(chatKey);
  set('items', (prev) => [...prev, attachment]);
}

export function removeAttachment(chatKey: string, id: string): void {
  const timer = retryTimers.get(id);
  if (timer) {
    clearTimeout(timer);
    retryTimers.delete(id);
  }
  queue.delete(id);
  jobs.delete(id);
  const [, set] = storeFor(chatKey);
  set('items', (prev) => prev.filter((a) => a.id !== id));
}

/** Clears a chat's attachments (after a send). */
export function clearAttachments(chatKey: string): void {
  for (const a of [...pendingAttachmentsFor(chatKey)]) removeAttachment(chatKey, a.id);
  const [, set] = storeFor(chatKey);
  set('items', []);
}

/* ── the upload pump ─────────────────────────────────────────────────── */

function pump(): void {
  while (running < MAX_UPLOADS) {
    const next = [...queue].find((id) => jobs.has(id));
    if (!next) return;
    queue.delete(next);
    const job = jobs.get(next)!;
    running += 1;
    void job()
      .catch(() => undefined)
      .finally(() => {
        running -= 1;
        jobs.delete(next);
        pump();
      });
  }
}

/** Queues a job for an attachment (only `MAX_UPLOADS` run at once). */
function schedule(chatKey: string, id: string, job: () => Promise<void>): void {
  jobs.set(id, job);
  queue.add(id);
  update(chatKey, id, { status: 'queued', progress: 0, error: undefined });
  pump();
}

/** A failed job: one automatic retry after 500 ms, then `failed` (manual). */
function failed(chatKey: string, id: string, message: string, retry: () => Promise<void>): void {
  const attachment = attachmentById(chatKey, id);
  if (!attachment) return;
  if (attachment.autoRetries < MAX_AUTO_RETRIES) {
    update(chatKey, id, { status: 'queued', autoRetries: attachment.autoRetries + 1, error: message });
    const timer = setTimeout(() => {
      retryTimers.delete(id);
      schedule(chatKey, id, retry);
    }, AUTO_RETRY_DELAY_MS);
    retryTimers.set(id, timer);
    return;
  }
  update(chatKey, id, { status: 'failed', error: message });
}

const describe = (error: unknown): string =>
  (error as { message?: unknown } | null)?.message !== undefined ? String((error as { message: unknown }).message) : String(error);

/* ── images: shrink + base64 (no transport, still a job so the states hold) ── */

const isImage = (file: File): boolean => file.type.startsWith('image/') || /\.(png|jpe?g|gif|webp|bmp|avif)$/i.test(file.name);

/** Shrinks an image file and encodes it; what a picked image runs before `ready`. */
async function encodeImage(file: Blob, onProgress?: (fraction: number) => void): Promise<{ mime: string; data: string; preview: string; size: number }> {
  onProgress?.(0.15);
  const shrunk = await shrinkImage(file);
  onProgress?.(0.6);
  const data = await blobToBase64(shrunk.blob);
  onProgress?.(1);
  const preview = `data:${shrunk.mime};base64,${data}`;
  return { mime: shrunk.mime, data, preview, size: shrunk.blob.size };
}

function imageJob(chatKey: string, id: string, source: Blob, name?: string): () => Promise<void> {
  return async () => {
    update(chatKey, id, { status: 'uploading', progress: 0.01 });
    try {
      const encoded = await encodeImage(source, (f) => update(chatKey, id, { progress: Math.min(Math.max(f, 0.01), PROGRESS_CEILING) }));
      update(chatKey, id, { status: 'ready', progress: 1, ...encoded, ...(name ? { name } : {}) });
    } catch (error) {
      failed(chatKey, id, describe(error), imageJob(chatKey, id, source, name));
    }
  };
}

/* ── files: the blob channel, with progress ──────────────────────────── */

function fileJob(chatKey: string, hostId: string, id: string, file: File): () => Promise<void> {
  return async () => {
    update(chatKey, id, { status: 'uploading', progress: 0.01 });
    try {
      if (file.size > MAX_GENERIC_FILE_BYTES) throw new Error(`${file.name} is larger than ${MAX_GENERIC_LABEL}`);
      const result = await uploadPicked(hostId, file, (p) => {
        const total = Math.max(p.total, 1);
        update(chatKey, id, { progress: Math.min(Math.max(p.sent / total, 0.01), PROGRESS_CEILING) });
      });
      update(chatKey, id, { status: 'ready', progress: 1, path: result.path });
    } catch (error) {
      failed(chatKey, id, describe(error), fileJob(chatKey, hostId, id, file));
    }
  };
}

/* ── public API ──────────────────────────────────────────────────────── */

export type AttachResult = { added: PendingAttachment[]; refused: File[]; overLimit: number };

/** How many more attachments this chat can take. */
export function attachmentRoom(chatKey: string): number {
  return Math.max(0, MAX_ATTACHMENTS - pendingAttachmentsFor(chatKey).length);
}

/** Attaches picked images (shrunk, annotated elsewhere) and files (uploaded). */
export function attach(chatKey: string, hostId: string, files: readonly File[]): AttachResult {
  const added: PendingAttachment[] = [];
  const refused: File[] = [];
  let overLimit = 0;
  for (const file of files) {
    if (pendingAttachmentsFor(chatKey).length >= MAX_ATTACHMENTS) {
      overLimit += 1;
      continue;
    }
    if (isImage(file)) {
      const id = nextId('image');
      const attachment: PendingAttachment = {
        id,
        kind: 'image',
        name: file.name || `image ${id.slice(-3)}`,
        mime: file.type || 'image/png',
        size: file.size,
        status: 'queued',
        progress: 0,
        autoRetries: 0,
        source: file,
      };
      push(chatKey, attachment);
      added.push(attachment);
      schedule(chatKey, id, imageJob(chatKey, id, file));
    } else if (file.size > MAX_GENERIC_FILE_BYTES) {
      const id = nextId('file');
      const attachment: PendingAttachment = {
        id,
        kind: 'file',
        name: file.name,
        mime: file.type || 'application/octet-stream',
        size: file.size,
        status: 'failed',
        progress: 0,
        autoRetries: MAX_AUTO_RETRIES,
        error: `${file.name} is larger than ${MAX_GENERIC_LABEL}`,
      };
      push(chatKey, attachment);
      added.push(attachment);
      refused.push(file);
    } else {
      const id = nextId('file');
      const attachment: PendingAttachment = {
        id,
        kind: 'file',
        name: file.name,
        mime: file.type || 'application/octet-stream',
        size: file.size,
        status: 'queued',
        progress: 0,
        autoRetries: 0,
        source: file,
      };
      push(chatKey, attachment);
      added.push(attachment);
      schedule(chatKey, id, fileJob(chatKey, hostId, id, file));
    }
  }
  return { added, refused, overLimit };
}

/**
 * Manual retry (Z-60): a fresh retry budget. Re-runs the same job when the
 * picked bytes are still held (an image, or a file whose upload failed);
 * without them the chip asks to pick the file again.
 */
export function retryAttachment(chatKey: string, hostId: string, id: string): void {
  const attachment = attachmentById(chatKey, id);
  if (!attachment || attachment.status !== 'failed') return;
  const source = attachment.source;
  if (!source) {
    update(chatKey, id, { status: 'failed', error: 'Pick the file again to retry' });
    return;
  }
  update(chatKey, id, { autoRetries: 0, error: undefined });
  schedule(chatKey, id, attachment.kind === 'image' ? imageJob(chatKey, id, source, attachment.name) : fileJob(chatKey, hostId, id, source as File));
}

/** Replaces an image's bytes — the image editor's result (D-5). Keeps the id,
 *  so a chip's place and the draft's order survive an annotation. */
export function replaceImage(chatKey: string, id: string, blob: Blob, name?: string): void {
  const attachment = attachmentById(chatKey, id);
  if (!attachment || attachment.kind !== 'image') return;
  update(chatKey, id, {
    mime: blob.type || attachment.mime,
    size: blob.size,
    status: 'queued',
    progress: 0,
    data: undefined,
    preview: undefined,
    error: undefined,
    source: blob,
  });
  schedule(chatKey, id, imageJob(chatKey, id, blob, name ?? attachment.name));
}

/** Adds an already-encoded image (the editor's output, or a message being
 *  edited back into the composer) as a ready attachment. */
export function addReadyImage(
  chatKey: string,
  image: { name?: string; mime: string; data: string },
): PendingAttachment | undefined {
  if (pendingAttachmentsFor(chatKey).length >= MAX_ATTACHMENTS) return undefined;
  const id = nextId('image');
  const attachment: PendingAttachment = {
    id,
    kind: 'image',
    name: image.name ?? `image ${id.slice(-3)}.${(image.mime.split('/')[1] ?? 'png').replace(/[^a-z0-9]/gi, '') || 'png'}`,
    mime: image.mime,
    size: Math.round((image.data.length * 3) / 4),
    status: 'ready',
    progress: 1,
    data: image.data,
    preview: `data:${image.mime};base64,${image.data}`,
    autoRetries: 0,
  };
  push(chatKey, attachment);
  return attachment;
}

/** Replaces every pending image with already-encoded ones (Edit / tree jump). */
export function setImages(chatKey: string, images: readonly { mimeType: string; data: string }[]): string[] {
  for (const a of pendingAttachmentsFor(chatKey)) if (a.kind === 'image') removeAttachment(chatKey, a.id);
  const ids: string[] = [];
  for (const image of images) {
    const added = addReadyImage(chatKey, { mime: image.mimeType, data: image.data });
    if (added) ids.push(added.id);
  }
  return ids;
}

/** The chat's ready images, in attach order — what the prompt carries. */
export function readyImages(chatKey: string): OutboxImage[] {
  return pendingAttachmentsFor(chatKey)
    .filter((a) => a.kind === 'image' && a.status === 'ready' && a.data)
    .map((a) => ({ mimeType: a.mime, data: a.data! }));
}

/** True while nothing is blocking a send; a failed attachment blocks it too. With `referenced`
 *  (the ids the draft's chips name), a placed attachment whose chip was deleted no longer counts. */
export function attachmentsGate(chatKey: string, referenced?: ReadonlySet<string>): AttachmentsGate {
  const list = pendingAttachmentsFor(chatKey).filter((a) => !referenced || !a.placed || referenced.has(a.id));
  const uploading = list.filter((a) => a.status === 'queued' || a.status === 'uploading').length;
  const failedCount = list.filter((a) => a.status === 'failed').length;
  return { ok: uploading === 0 && failedCount === 0, uploading, failed: failedCount };
}

/** The composer's send gate (Z-60): everything attached is ready. */
export function attachmentsReady(chatKey: string, referenced?: ReadonlySet<string>): boolean {
  return attachmentsGate(chatKey, referenced).ok;
}

/** `[file: <name>]@<host path>` — one mention line per uploaded file. Bracketed
 *  name first so a model reading raw text still sees a readable filename next
 *  to the path it must use. */
export function formatFileMention(file: { name: string; path?: string }): string {
  return `[file: ${file.name}]@${file.path ?? ''}`;
}

/** One `[file: name]@path` mention found in a message's text, for the bubble
 *  renderer to turn into a chip instead of raw text. */
export type FileMention = { name: string; path: string; start: number; end: number };

const FILE_MENTION_RE = /\[file: ([^\]]*)\]@(\S+)/g;

export function fileMentionsOf(text: string): FileMention[] {
  const out: FileMention[] = [];
  for (const m of text.matchAll(FILE_MENTION_RE)) {
    out.push({ name: m[1]!, path: m[2]!, start: m.index!, end: m.index! + m[0].length });
  }
  return out;
}

/** The text with every `[file: name]@path` mention (and the blank line(s) that
 *  isolate them) stripped — what a bubble shows as its prose. */
export function stripFileMentions(text: string): string {
  return text.replace(FILE_MENTION_RE, '').replace(/\n{3,}/g, '\n\n').trim();
}

/** What sends a file: `ready` here, and the legacy `done` the draft-token
 *  helpers (`lib/composer/chips.ts`) still spell it. */
type SendableStatus = AttachmentStatus | 'done';

/** Appends a mention line per sendable file to `text`, blank-line separated —
 *  what actually reaches pi; files still uploading or failed never are. */
export function appendFileMentions(text: string, files: readonly { name: string; status: SendableStatus; path?: string; [extra: string]: unknown }[]): string {
  const done = files.filter((f): f is { name: string; status: SendableStatus; path: string } => (f.status === 'ready' || f.status === 'done') && !!f.path);
  if (!done.length) return text;
  const mentions = done.map((f) => formatFileMention(f)).join('\n');
  return text.trim() ? `${text}\n\n${mentions}` : mentions;
}

/** The full prompt text for a chat's draft: file mentions appended. */
export function withAttachmentMentions(chatKey: string, text: string): string {
  return appendFileMentions(text, pendingAttachmentsFor(chatKey));
}

/* ── capture/test seeding ───────────────────────────────────────────── */

/**
 * Adds an attachment in a given state **without** running an upload — the
 * capture scenarios use it to show all four Z-60 states side by side, and
 * tests use it to set up a chip. The state machine itself is exercised
 * through `attach`/`retryAttachment`.
 */
export function seedAttachment(
  chatKey: string,
  attachment: Omit<PendingAttachment, 'id' | 'autoRetries'> & { id?: string },
): PendingAttachment | undefined {
  if (pendingAttachmentsFor(chatKey).length >= MAX_ATTACHMENTS) return undefined;
  const id = attachment.id ?? nextId(attachment.kind);
  const full: PendingAttachment = {
    ...attachment,
    id,
    autoRetries: attachment.status === 'failed' ? MAX_AUTO_RETRIES : 0,
    preview: attachment.preview ?? (attachment.data ? `data:${attachment.mime};base64,${attachment.data}` : undefined),
  };
  push(chatKey, full);
  return full;
}

/* ── tests ───────────────────────────────────────────────────────────── */

/** Test hook: await every scheduled job's queue to drain. */
export async function drainAttachments(chatKey: string): Promise<void> {
  for (let i = 0; i < 200; i++) {
    const pending = pendingAttachmentsFor(chatKey).some((a) => a.status === 'queued' || a.status === 'uploading');
    if (!pending && !retryTimers.size && !running) return;
    await new Promise((r) => setTimeout(r, 5));
  }
}

/** Test hook: drop every per-chat store and queued job. */
export function resetAttachments(): void {
  for (const timer of retryTimers.values()) clearTimeout(timer);
  retryTimers.clear();
  queue.clear();
  jobs.clear();
  running = 0;
  stores.clear();
}
