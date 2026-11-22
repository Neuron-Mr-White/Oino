/**
 * Previewer routing (UNI-157): what kind of full-screen viewer a mime
 * type / file extension gets. Pure, no Solid — the component picks a
 * renderer from this.
 */

export type MediaKind = 'image' | 'pdf' | 'audio' | 'video' | 'markdown' | 'code' | 'log' | 'text' | 'other';

const EXT_KIND: Record<string, MediaKind> = {
  md: 'markdown', markdown: 'markdown',
  log: 'log', logs: 'log',
  txt: 'text', text: 'text',
};

/** No in-app renderer worth building (UNI-169): docx and friends open
 * in other apps via "Open with…" / land in Downloads via "Download" —
 * anything here renders the previewer's no-preview card instead of
 * garbage text. */
const OPEN_WITH_EXTS = new Set([
  // office
  'doc', 'docx', 'xls', 'xlsx', 'ppt', 'pptx', 'odt', 'ods', 'odp', 'rtf', 'pages', 'numbers', 'key',
  // archives & packages
  'zip', 'tar', 'gz', 'tgz', 'bz2', 'xz', '7z', 'rar', 'apk', 'jar', 'deb', 'rpm', 'exe', 'msi', 'dmg', 'iso',
  // binaries/fonts/models
  'bin', 'dat', 'o', 'so', 'dylib', 'class', 'pyc', 'woff', 'woff2', 'ttf', 'otf', 'onnx',
]);

/** Media files the viewers render from bytes (the mime decides the viewer; the extension only says "not text"). */
const MEDIA_EXTS = new Set([
  'png', 'jpg', 'jpeg', 'gif', 'webp', 'avif', 'bmp', 'ico', 'icns', 'tif', 'tiff', 'heic', 'heif', 'svg',
  'mp3', 'wav', 'ogg', 'oga', 'opus', 'flac', 'm4a', 'aac', 'weba',
  'mp4', 'm4v', 'mov', 'webm', 'mkv', 'avi', 'ogv',
]);

/** Extensions the code highlighter (`src/lib/markdown/highlight.ts`) knows
 * a language for — kept in sync with its `ALIASES`/`LOADERS` tables by
 * hand (small, stable list); anything else with a text mime falls back to
 * plain monospace text. */
const CODE_EXTENSIONS = new Set([
  'js', 'jsx', 'mjs', 'cjs', 'ts', 'tsx', 'py', 'py3', 'rs', 'sh', 'bash', 'zsh', 'console',
  'yml', 'yaml', 'htm', 'html', 'c++', 'cc', 'cxx', 'c', 'cpp', 'h', 'hpp', 'json', 'css', 'sql',
  'go', 'java', 'cs', 'rb', 'toml',
]);

function extOf(path: string): string {
  const name = path.split(/[/\\]/).pop() ?? path;
  const dot = name.lastIndexOf('.');
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : '';
}

/** Which full-screen viewer to use for `mime` (and/or a file `path`/name).
 * Falls back to extension when the mime is missing or generic
 * (`application/octet-stream`). */
export function classifyMedia(mime: string | undefined, path?: string): MediaKind {
  const m = (mime ?? '').toLowerCase();
  if (m.startsWith('image/')) return 'image';
  if (m === 'application/pdf') return 'pdf';
  if (m.startsWith('audio/')) return 'audio';
  if (m.startsWith('video/')) return 'video';

  const ext = path ? extOf(path) : '';
  if (ext && EXT_KIND[ext]) return EXT_KIND[ext];
  if (ext === 'pdf') return 'pdf';
  if (ext && CODE_EXTENSIONS.has(ext)) return 'code';
  if (ext && OPEN_WITH_EXTS.has(ext)) return 'other';

  if (m === 'text/markdown') return 'markdown';
  if (m === 'application/json' || m.startsWith('text/x-') || m === 'application/javascript') return 'code';
  if (m.startsWith('text/')) return 'text';
  // An opaque binary with no known extension: no garbage text — the
  // no-preview card with "Open with… / Download" (UNI-169).
  if (m === 'application/octet-stream' || m === '') return 'other';
  // Unknown binary with no useful extension: show as text/log tail rather
  // than silently failing; the viewer still offers "Open with…" as escape.
  return ext ? 'text' : 'log';
}

/** The CodeMirror/Lezer language id (see `highlight.ts` `ALIASES`) for a
 * file extension, or `undefined` when there is none (plain text). */
export function languageOf(path: string): string | undefined {
  const ext = extOf(path);
  return ext || undefined;
}

/** Whether a file opens straight in the previewer (bypassing the editor),
 * based on its extension alone (UNI-168: folder paging). */
export function isDirectPreview(path: string): boolean {
  const ext = extOf(path);
  if (!ext) return false;
  const kind = classifyMedia(undefined, path);
  if (kind === 'other') return OPEN_WITH_EXTS.has(ext) || MEDIA_EXTS.has(ext);
  return kind === 'image' || kind === 'pdf' || kind === 'audio' || kind === 'video';
}

/** A binary extension we know has no in-app viewer (everything else is sniffed by content). */
export function isKnownBinaryExt(path: string): boolean {
  const ext = extOf(path);
  return OPEN_WITH_EXTS.has(ext) || MEDIA_EXTS.has(ext);
}

/**
 * Byte sniff (owner 2026-11-05: `.env`, `Makefile`, `.gitignore`… read "No preview"):
 * text = no NUL byte in the first 8 KB and the head decodes as UTF-8 (a multi-byte
 * character cut at the 8 KB edge still counts). Mirrors the host's `looks_like_text`.
 */
export function looksLikeText(bytes: Uint8Array): boolean {
  const head = bytes.subarray(0, 8192);
  for (let i = 0; i < head.length; i++) if (head[i] === 0) return false;
  try {
    new TextDecoder('utf-8', { fatal: true }).decode(head);
    return true;
  } catch {
    if (bytes.length <= 8192) return false;
    // Cut mid-character at the edge: retry without the last 3 bytes.
    try {
      new TextDecoder('utf-8', { fatal: true }).decode(head.subarray(0, Math.max(0, head.length - 3)));
      return true;
    } catch {
      return false;
    }
  }
}

/** `classifyMedia` refined by content: an unknown / generic `other` that sniffs as text renders as text. */
export function classifyBytes(mime: string | undefined, path: string, bytes: Uint8Array | ArrayLike<number>): MediaKind {
  const kind = classifyMedia(mime, path);
  if (kind !== 'other' && kind !== 'log') return kind;
  if (isKnownBinaryExt(path)) return kind;
  const u8 = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  return looksLikeText(u8) ? (kind === 'log' ? 'log' : 'text') : 'other';
}
