/**
 * `att:<task>/<name>` attachment handling (UNI-130, shared in UNI-156):
 * chips for files, inline `<img>/<video>/<audio>` for media. The phone
 * can't load attachment URLs directly, so rendered nodes carry
 * `data-att-path` with no live `src`; `hydrateAttachments` resolves them
 * to object URLs through a caller-supplied fetcher (the board transport).
 * Kept decoupled from `lib/board/api` so the shared renderer has no board
 * dependency — only `lib/board/markdown.ts` wires a real resolver/fetcher.
 */
import { escapeHtml } from './escape';

export const ATT_REF = /^att:([A-Za-z0-9._-]+)\/([A-Za-z0-9._-]+)$/;
const IMAGE = /\.(png|jpe?g|gif|webp|avif|bmp)$/i;
const VIDEO = /\.(mp4|webm|mov|m4v|ogv)$/i;
const AUDIO = /\.(mp3|wav|ogg|m4a|flac|opus)$/i;
const PDF = /\.pdf$/i;
const TEXT = /\.(txt|log|md|csv|json|ya?ml|toml|diff|patch)$/i;

export type AttKind = 'image' | 'video' | 'audio' | 'pdf' | 'text' | 'file';

export function attKind(name: string): AttKind {
  if (IMAGE.test(name)) return 'image';
  if (VIDEO.test(name)) return 'video';
  if (AUDIO.test(name)) return 'audio';
  if (PDF.test(name)) return 'pdf';
  if (TEXT.test(name)) return 'text';
  return 'file';
}

/** Stored as `<sha8>-<name>`. */
export function attDisplayName(name: string): string {
  return name.replace(/^[0-9a-f]{8}-/, '');
}

const FILE_ICON: Record<AttKind, string> = { image: 'IMG', video: 'VID', audio: 'AUD', pdf: 'PDF', text: 'TXT', file: 'FILE' };

function chip(path: string, name: string, kind: AttKind, label?: string): string {
  const shown = escapeHtml(label && label.trim() ? label : attDisplayName(name));
  const ext = (name.split('.').pop() ?? '').slice(0, 5).toUpperCase();
  return `<a class="att-chip att-${kind}" href="#" data-att-path="${escapeHtml(path)}" data-att-name="${escapeHtml(name)}"><span class="att-badge">${escapeHtml(
    kind === 'file' && ext ? ext : FILE_ICON[kind],
  )}</span><span class="att-name">${shown}</span></a>`;
}

/** Resolves an `att:` reference to attachment-handling HTML, or null when not an att: ref / unresolvable. */
export function attachmentHtml(reference: string, alt: string, resolvePath: (ref: string) => string | null): string | null {
  const match = ATT_REF.exec(reference.trim());
  const path = resolvePath(reference);
  if (!match || !path) return null;
  const name = match[2]!;
  const kind = attKind(name);
  const label = escapeHtml(alt || attDisplayName(name));
  const at = escapeHtml(path);
  switch (kind) {
    case 'image':
      return `<a class="att-image" href="#" data-att-path="${at}"><img data-att-path="${at}" alt="${label}" loading="lazy"></a>`;
    case 'video':
      return `<video class="att-video" data-att-path="${at}" controls preload="metadata"></video>`;
    case 'audio':
      return `<audio class="att-audio" data-att-path="${at}" controls preload="metadata"></audio>`;
    default:
      return chip(path, name, kind, alt);
  }
}

/** Chip/media HTML for an `att:` link token (not an image/media embed). */
export function attachmentLinkHtml(href: string, text: string, resolvePath: (ref: string) => string | null): string | null {
  const match = ATT_REF.exec(href.trim());
  const path = resolvePath(href);
  if (!match || !path) return null;
  const kind = attKind(match[2]!);
  if (kind === 'image' || kind === 'video' || kind === 'audio') return attachmentHtml(href, text, resolvePath);
  return chip(path, match[2]!, kind, text);
}

/**
 * Resolves `data-att-path` media inside `root` to object URLs fetched by
 * the caller's `fetchUrl`. Images/video/audio get a `src`; chips and image
 * links get an `href`. Safe to call repeatedly.
 */
export async function hydrateAttachments(root: HTMLElement, fetchUrl: (path: string) => Promise<string>): Promise<void> {
  const nodes = [...root.querySelectorAll<HTMLElement>('[data-att-path]')];
  await Promise.all(
    nodes.map(async (node) => {
      const path = node.dataset.attPath;
      if (!path || node.dataset.attReady) return;
      try {
        const url = await fetchUrl(path);
        node.dataset.attReady = '1';
        if (node instanceof HTMLImageElement || node instanceof HTMLMediaElement) node.src = url;
        else if (node instanceof HTMLAnchorElement) {
          node.href = url;
          node.target = '_blank';
          if (node.dataset.attName) node.download = attDisplayName(node.dataset.attName);
        }
      } catch {
        node.dataset.attFailed = '1';
      }
    }),
  );
}
