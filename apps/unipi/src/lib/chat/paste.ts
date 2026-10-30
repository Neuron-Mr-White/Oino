/**
 * UNI-256: what a paste into the composer carries, and local files handed
 * over by the OS (dropped on the desktop window, copied in the file manager).
 *
 * WebKitGTK (the Linux desktop app) hands the page an EMPTY paste event for
 * a screenshot on the clipboard, and for files copied in the file manager
 * a `text/uri-list` type whose data reads back as "" (or nothing at all for
 * `x-special/gnome-copied-files`). Chromium/WebView2 give `File`s. So:
 *
 *  - `files`  → attach them (images inline, others as file chips);
 *  - `native` → ask the desktop core (`clipboard_files`) for the clipboard's
 *               files / image; if it finds none, paste `text` (if any);
 *  - `text`   → plain-text paste, as before;
 *  - `none`   → nothing to do.
 */

export type PasteData = Pick<DataTransfer, 'types' | 'getData'> & { files?: ArrayLike<File> | null };

export type PasteKind =
  | { kind: 'files'; files: File[] }
  | { kind: 'native'; text: string }
  | { kind: 'text'; text: string }
  | { kind: 'none' };

const FILE_TYPES = ['text/uri-list', 'x-special/gnome-copied-files'];

/** `nativeFallback`: the desktop core can read the clipboard itself (Tauri desktop). */
export function classifyPaste(dt: PasteData | null | undefined, nativeFallback: boolean): PasteKind {
  if (!dt) return nativeFallback ? { kind: 'native', text: '' } : { kind: 'none' };
  const files = Array.from(dt.files ?? []);
  const text = (dt.getData('text/plain') ?? '').replace(/\r\n?/g, '\n');
  if (files.length) return { kind: 'files', files };
  const types = Array.from(dt.types ?? []);
  // Copied files: the uri list may be readable (some engines) — only then do we know it's
  // local files; otherwise the core reads it.
  const hasFileList = types.some((t) => FILE_TYPES.includes(t));
  if (hasFileList && nativeFallback) return { kind: 'native', text };
  if (text) return { kind: 'text', text };
  // An empty event (no text, no files): a clipboard image WebKitGTK didn't expose.
  return nativeFallback ? { kind: 'native', text: '' } : { kind: 'none' };
}

/** A file the desktop core described (dropped / pasted from the OS). */
export type LocalFile = { path: string; name: string; size: number; isDir: boolean };

const MIME: Record<string, string> = {
  png: 'image/png',
  jpg: 'image/jpeg',
  jpeg: 'image/jpeg',
  gif: 'image/gif',
  webp: 'image/webp',
  bmp: 'image/bmp',
  avif: 'image/avif',
  heic: 'image/heic',
  svg: 'image/svg+xml',
  pdf: 'application/pdf',
  txt: 'text/plain',
  md: 'text/markdown',
  log: 'text/plain',
  csv: 'text/csv',
  json: 'application/json',
  html: 'text/html',
  zip: 'application/zip',
  mp4: 'video/mp4',
  mov: 'video/quicktime',
  webm: 'video/webm',
  mp3: 'audio/mpeg',
};

/** MIME type by file extension (the OS gives no type for a path). */
export function mimeForName(name: string): string {
  const ext = /\.([a-z0-9]+)$/i.exec(name)?.[1]?.toLowerCase();
  return (ext && MIME[ext]) || 'application/octet-stream';
}

/** Images the composer can shrink and send inline (SVG goes as a file, as for drops). */
export function isInlineImage(name: string): boolean {
  const mime = mimeForName(name);
  return mime.startsWith('image/') && mime !== 'image/svg+xml' && mime !== 'image/heic';
}

export type LocalPlan = {
  /** Read these (bytes) and attach them like dropped files. */
  read: LocalFile[];
  /** Can't attach: shown as failed chips with the reason. */
  refused: Array<{ file: LocalFile; reason: string }>;
};

/** Which local files to read and attach, which to refuse (folders, too big). */
export function planLocalFiles(files: readonly LocalFile[], maxGenericBytes: number): LocalPlan {
  const plan: LocalPlan = { read: [], refused: [] };
  for (const f of files) {
    if (f.isDir) plan.refused.push({ file: f, reason: 'Folders can’t be attached — drop the files inside' });
    else if (!isInlineImage(f.name) && f.size > maxGenericBytes) plan.refused.push({ file: f, reason: `${f.name} is larger than ${Math.round(maxGenericBytes / 1048576)} MB` });
    else plan.read.push(f);
  }
  return plan;
}

/** Maps a drop position (Tauri: physical px, window client area) to CSS px. */
export function dropPoint(position: { x: number; y: number }, scale: number): { x: number; y: number } {
  const s = scale > 0 ? scale : 1;
  return { x: position.x / s, y: position.y / s };
}
