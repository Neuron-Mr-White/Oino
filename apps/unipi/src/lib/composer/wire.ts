/**
 * What a composer send puts on the wire (owner spec 2026-11-03, composer chips):
 * the draft string with its inline chips resolved, plus the images in the
 * order the text names them. Pure, so the composer and the tests share it.
 *
 *   `[image]@img:<id>`        → `[image N]`, N = that image's place in `images`
 *                                (the content blocks after the text), so
 *                                "compare [image 1] with [image 2]" names the
 *                                exact pictures, where the user put them;
 *   `[file: name]@att:<id>`   → `[file: name]@<host path>` in place (UNI-203);
 *   `$skill`                  → pi's `/skill:<name>` invocation (`serializeSkills`);
 *   `/command`, `@path`       → sent as typed (the engine routes / reads them).
 *
 * An attachment no chip names any more (its chip was deleted — kept only so
 * Undo can bring it back) is not sent. One whose chip never existed (added by
 * an older path) is sent after the text, as before.
 */
import type { OutboxImage } from '../session/types';
import { fileChipIds, imageChipIds, resolveAttachmentTokens, resolveImageTokens, serializeSkills } from './chips';
import { appendFileMentions, type PendingAttachment } from './attachments';

export type Outgoing = { text: string; images: OutboxImage[] };

type Att = Pick<PendingAttachment, 'id' | 'kind' | 'name' | 'status' | 'mime' | 'data' | 'path'> & { placed?: boolean };

export function outgoingMessage(text: string, attachments: readonly Att[], commands?: ReadonlySet<string>): Outgoing {
  const readyImages = attachments.filter((a) => a.kind === 'image' && a.status === 'ready' && !!a.data);
  const imgs = resolveImageTokens(text, readyImages);
  let out = imgs.text;
  const ids = [...imgs.ids];
  // Never-placed images (no chip ever named them) still go, numbered after the named ones.
  for (const a of readyImages) {
    if (ids.includes(a.id) || a.placed) continue;
    ids.push(a.id);
    const body = out.replace(/\s+$/, '');
    out = body ? `${body} [image ${ids.length}]` : `[image ${ids.length}]`;
  }
  const files = resolveAttachmentTokens(out, (id) => {
    const a = attachments.find((x) => x.id === id);
    if (!a) return undefined;
    return { name: a.name, status: a.status === 'ready' ? 'done' : a.status === 'failed' ? 'error' : 'uploading', ...(a.path ? { path: a.path } : {}) };
  });
  out = appendFileMentions(
    files.text,
    attachments.filter((a) => a.kind === 'file' && !a.placed && !files.sentIds.includes(a.id)),
  );
  // The space a trailing chip keeps after itself is not part of the message.
  out = serializeSkills(out.replace(/[ \t]+$/, ''), commands);
  const byId = new Map(readyImages.map((a) => [a.id, a]));
  return { text: out, images: ids.map((id) => byId.get(id)!).map((a) => ({ mimeType: a.mime, data: a.data! })) };
}

/** The ids of the attachments the draft's chips name (images and files). */
export function referencedIds(text: string): Set<string> {
  return new Set([...imageChipIds(text), ...fileChipIds(text)]);
}
