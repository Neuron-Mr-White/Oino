/**
 * Getting files into a task (ported from the kanboard web UI's attach.ts,
 * UNI-130): readable names for pasted/camera images, sequential uploads
 * through the board transport, insert-at-cursor. UNI-131 reuses these
 * for notes and the description editor.
 */
import { api, describe, type Attachment } from './api';

/** The daemon refuses bigger uploads with 413; check before sending the bytes over the relay. */
export const MAX_UPLOAD_BYTES = 25 * 1024 * 1024;

export function namedFile(file: File, now = new Date()): File {
  if (file.name && file.name !== 'image.png' && file.name !== 'image.jpg') return file;
  const ext = (file.type.split('/')[1] ?? 'bin').replace('jpeg', 'jpg').replace(/\+.*/, '');
  const stamp = now.toISOString().slice(0, 16).replace('T', '-').replace(':', '');
  return new File([file], `${file.type.startsWith('image/') ? 'screenshot' : 'file'}-${stamp}.${ext}`, { type: file.type });
}

/** Uploads files one by one; failures are reported, the rest still upload. */
export async function uploadAll(
  hostId: string,
  slug: string,
  taskId: string,
  files: File[],
  onError: (message: string) => void,
): Promise<Attachment[]> {
  const done: Attachment[] = [];
  for (const raw of files) {
    const file = namedFile(raw);
    if (file.size > MAX_UPLOAD_BYTES) {
      onError(`${file.name} is larger than 25 MB`);
      continue;
    }
    try {
      done.push(await api.uploadFile(hostId, slug, taskId, file));
    } catch (error) {
      onError(describe(error));
    }
  }
  return done;
}

/** Inserts `snippet` at the textarea's cursor on its own line; returns the new value. */
export function insertAtCursor(area: HTMLTextAreaElement | undefined, current: string, snippet: string): string {
  if (!area) return current ? `${current}\n${snippet}` : snippet;
  const start = area.selectionStart ?? current.length;
  const end = area.selectionEnd ?? current.length;
  const before = current.slice(0, start);
  const after = current.slice(end);
  const lead = before.length > 0 && !before.endsWith('\n') ? '\n' : '';
  const trail = after.length > 0 && !after.startsWith('\n') ? '\n' : '';
  return `${before}${lead}${snippet}${trail}${after}`;
}

export function formatBytes(size: number): string {
  if (size < 1024) return `${size} B`;
  if (size < 1024 * 1024) return `${Math.round(size / 1024)} KB`;
  return `${(size / (1024 * 1024)).toFixed(1)} MB`;
}
