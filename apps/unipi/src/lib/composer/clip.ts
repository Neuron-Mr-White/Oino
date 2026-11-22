/**
 * Rich copy / paste of the composer (owner 2026-11-06): a copy from the message box puts
 *
 * - `text/plain` — readable text for any app: an image chip reads `[image]`, a file chip its
 *   host path (`@/path`) or `[file: name]`;
 * - `application/x-unipi-composer` and a `data-unipi-composer` span in `text/html` — the draft
 *   text with its chip tokens plus the bytes / paths of the attachments they name, so a paste
 *   back into any Unipi message box restores the chips, images and edits included.
 *
 * Pure helpers; the editor (`ChipEditor`) and the composer (`Composer`) wire them up.
 */
import { attachmentToken, imageToken } from './chips';

export const CLIP_MIME = 'application/x-unipi-composer';

/** An attachment a copied draft names. `data` is a `data:` URL (images); `path` a ready file's host path. */
export type ClipAttachment = { id: string; kind: 'image' | 'file'; name: string; mime: string; data?: string; path?: string };
export type ComposerClip = { v: 1; text: string; attachments: ClipAttachment[] };

const IMG_TOKEN = /\[image]@img:(\S+)/g;
const FILE_TOKEN = /\[file: ([^\]\n]*)\]@att:(\S+)/g;

/** The ids of the attachment chips in `text`, in order (each once). */
export function clipIds(text: string): string[] {
  const ids = [...text.matchAll(IMG_TOKEN)].map((m) => m[1]!).concat([...text.matchAll(FILE_TOKEN)].map((m) => m[2]!));
  return [...new Set(ids)];
}

/** What other apps get: chips as readable words. */
export function plainOf(text: string, attachments: readonly ClipAttachment[]): string {
  return text
    .replace(IMG_TOKEN, '[image]')
    .replace(FILE_TOKEN, (_all, name: string, id: string) => {
      const path = attachments.find((a) => a.id === id)?.path;
      return path ? `@${path}` : `[file: ${name}]`;
    });
}

const toBase64 = (s: string) => {
  const bytes = new TextEncoder().encode(s);
  let bin = '';
  // Chunked: a spread of a multi-megabyte image payload overflows the call stack.
  for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(bin);
};
const fromBase64 = (b: string) => new TextDecoder().decode(Uint8Array.from(atob(b), (c) => c.charCodeAt(0)));
const escapeHtml = (s: string) => s.replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]!);

/** The clipboard flavours of a copy: plain text, the rich payload, and HTML carrying it. */
export function encodeClip(clip: ComposerClip): { plain: string; json: string; html: string } {
  const json = JSON.stringify(clip);
  const plain = plainOf(clip.text, clip.attachments);
  // The HTML flavour is the fallback for engines that drop custom types between copy and paste.
  const html = `<span data-unipi-composer="${toBase64(json)}">${escapeHtml(plain).replace(/\n/g, '<br>')}</span>`;
  return { plain, json, html };
}

const parse = (json: string | undefined | null): ComposerClip | undefined => {
  if (!json) return undefined;
  try {
    const v = JSON.parse(json) as Partial<ComposerClip>;
    if (v?.v !== 1 || typeof v.text !== 'string' || !Array.isArray(v.attachments)) return undefined;
    const attachments = v.attachments.filter(
      (a): a is ClipAttachment => !!a && typeof a.id === 'string' && (a.kind === 'image' || a.kind === 'file') && typeof a.name === 'string' && typeof a.mime === 'string',
    );
    return { v: 1, text: v.text, attachments };
  } catch {
    return undefined;
  }
};

/** The rich payload of a paste, when it came from a Unipi message box. */
export function decodeClip(data: Pick<DataTransfer, 'getData'> | null | undefined): ComposerClip | undefined {
  if (!data) return undefined;
  const direct = parse(data.getData(CLIP_MIME));
  if (direct) return direct;
  const html = data.getData('text/html');
  const m = html ? /data-unipi-composer="([A-Za-z0-9+/=]+)"/.exec(html) : null;
  if (!m) return undefined;
  try {
    return parse(fromBase64(m[1]!));
  } catch {
    return undefined;
  }
}

/** A `data:` URL as a File (the composer attaches it like a picked image). */
export function fileOfDataUrl(url: string, name: string, mime: string): File | undefined {
  const m = /^data:([^;,]+)?(;base64)?,(.*)$/s.exec(url);
  if (!m) return undefined;
  const bytes = m[2] ? Uint8Array.from(atob(m[3]!), (c) => c.charCodeAt(0)) : new TextEncoder().encode(decodeURIComponent(m[3]!));
  return new File([bytes], name, { type: m[1] || mime });
}

/**
 * The text to insert for a pasted clip. `known` = the ids this message box already holds (a copy
 * and paste in the same box keeps its chips as they are); `remap` = old id → the id a re-attached
 * copy got here. A chip that can't be restored becomes its readable form.
 */
export function pastedText(clip: ComposerClip, known: (id: string) => boolean, remap: ReadonlyMap<string, string>): string {
  return clip.text
    .replace(IMG_TOKEN, (all, id: string) => (known(id) ? all : remap.has(id) ? imageToken(remap.get(id)!) : '[image]'))
    .replace(FILE_TOKEN, (all, name: string, id: string) => {
      if (known(id)) return all;
      if (remap.has(id)) return attachmentToken(name, remap.get(id)!);
      const path = clip.attachments.find((a) => a.id === id)?.path;
      return path ? `@${path}` : `[file: ${name}]`;
    });
}
