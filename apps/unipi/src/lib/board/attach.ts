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

// ─── files picked before the task exists (new-task sheet, OIN-95) ───────────

/**
 * A file picked into a description before its task exists gets a placeholder reference
 * (`![shot.png](pending:3)` / `[log.txt](pending:4)`) at the cursor; once it is uploaded the real
 * `att:` markdown takes its place. `id` is unique per picked file, so two screenshots with one name stay apart.
 */
export function pendingMarkdown(file: Pick<File, 'name' | 'type'>, id: string): string {
  const label = file.name.replace(/[[\]]/g, '');
  return `${file.type.startsWith('image/') ? '!' : ''}[${label}](pending:${id})`;
}

const placeholder = (id: string) => `!?\\[[^\\]]*\\]\\(pending:${id.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\)`;

/** Replaces each placeholder with its stored markdown (`null`: the upload failed or was dropped — the placeholder's own line goes). */
export function resolvePending(body: string, resolved: Array<{ id: string; markdown: string | null }>): string {
  let out = body;
  for (const { id, markdown } of resolved) {
    if (markdown === null) {
      out = out.replace(new RegExp(`(^|\n)[ \t]*${placeholder(id)}[ \t]*(?=\n|$)`, 'g'), '');
      out = out.replace(new RegExp(placeholder(id), 'g'), '');
    } else {
      out = out.replace(new RegExp(placeholder(id), 'g'), () => markdown);
    }
  }
  return out.trim();
}

/** The text without any placeholder (what the task is created with before its files are stored). */
export function stripPending(body: string): string {
  return resolvePending(body, [...body.matchAll(/\(pending:([^)]*)\)/g)].map((m) => ({ id: m[1]!, markdown: null })));
}
