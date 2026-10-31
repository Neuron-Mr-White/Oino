/**
 * Tappable file paths in chat text (UNI-157): an inline-code token (`` `…` ``,
 * rendered as `<code>`) that looks like a file — absolute, `@`-prefixed, or
 * relative with a recognisable extension — opens the previewer. The host
 * only serves paths some pi bridge already registered (`blob_get{path}`),
 * so a path the phone merely saw in text (not in `details.attachments[]`)
 * is registered on demand with `file_share{path}` first (refused outside
 * the session's cwd).
 */
import type { RefChat } from '../chat/pathStat';
import { blobTransport } from '../blob';
import { classifyMedia } from './previewer';
import { openPreviewer, type PreviewItem } from './previewerState';

/** Extensions worth treating as "this inline-code token is a file path".
 * Deliberately generous — a false positive just opens a previewer that
 * 404s; a false negative is a missed tap target, worse for the user. */
const PATH_EXT = /\.[A-Za-z0-9]{1,10}$/;

/** `@path`, an absolute path (`/…`), or a relative path with a `./`/`../`
 * prefix or at least one `/` and a recognisable extension — not a bare
 * word like `foo.bar` (too likely to be prose) and not a URL. */
export function looksLikePath(text: string): string | undefined {
  const trimmed = text.trim();
  if (!trimmed || trimmed.length > 300 || /\s/.test(trimmed)) return undefined;
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(trimmed)) return undefined; // a URL, not a path
  const body = trimmed.startsWith('@') ? trimmed.slice(1) : trimmed;
  if (!body) return undefined;
  const isAbsolute = body.startsWith('/');
  const isExplicitRelative = body.startsWith('./') || body.startsWith('../');
  const hasSlashAndExt = body.includes('/') && PATH_EXT.test(body);
  if (!isAbsolute && !isExplicitRelative && !hasSlashAndExt) return undefined;
  return body;
}

/** Resolves `path` (against `cwd` when relative) to a path the
 * bridge/host understand: absolute or `~`-prefixed passes through (the
 * host expands `~` against its own $HOME, UNI-178), relative is joined
 * with `cwd`. */
export function resolvePathToken(path: string, cwd: string | undefined): string {
  if (path.startsWith('/')) return path;
  if (path.startsWith('~')) return path; // the host expands ~ against its $HOME
  if (!cwd) return path;
  return `${cwd.replace(/\/+$/, '')}/${path.replace(/^\.\//, '')}`;
}

/** Opens `path` in the shared previewer: fetches its bytes via the blob
 * channel, registering it with the bridge first if the host refuses
 * (unauthorized = not yet allow-listed). */
export async function openFilePath(chat: RefChat, path: string): Promise<void> {
  const hostId = chat.hostId;
  const name = path.split('/').pop() ?? path;
  const fetchOnce = () => blobTransport().getBytes(hostId, { path });
  let result;
  try {
    result = await fetchOnce();
  } catch (error) {
    const code = (error as { code?: string })?.code;
    if ((code !== 'E_UNAUTHORIZED' && code !== 'E_NOT_FOUND') || !chat.shareFile) throw error;
    await chat.shareFile(path);
    result = await fetchOnce();
  }
  const kind = classifyMedia(result.mime, path);
  const blob = new Blob([result.bytes as BlobPart], { type: result.mime || 'application/octet-stream' });
  const url = URL.createObjectURL(blob);
  if (kind === 'image') {
    const item: PreviewItem = { url, mime: result.mime, name };
    openPreviewer({ kind: 'image', items: [item], index: 0 });
    return;
  }
  if (kind === 'pdf' || kind === 'audio' || kind === 'video' || kind === 'other') {
    openPreviewer({ kind, name, mime: result.mime, url });
    return;
  }
  const text = await blob.text();
  openPreviewer({ kind, name, mime: result.mime, text });
}
