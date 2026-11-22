/**
 * Files pasted or picked into a reply that has not been sent (the In Review / Blocked dock, P3-6r).
 * Nothing is stored on the task before the reply goes out: the file waits here, its chip carries a
 * `pending:<n>` link in the note's text (`attachDraft`), and `commitHeld` uploads only the files the
 * note still names at the moment of sending. A chip that was removed, a note that was cleared or a
 * dock that was left leave nothing on the task — the same promise as Edit description.
 *
 * Held per ticket key (`noteKey`), like the typed note itself: stepping away from the ticket and back
 * keeps both the words and the pending chips.
 */
import type { Attachment } from './api';
import { namedFile, pendingMarkdown, resolvePending } from './attach';
import { parseRef, pendingId, type DraftRef } from './attachDraft';

type Held = { file: File; url?: string };

const registry = new Map<string, Map<string, Held>>();
let seq = 0;

/** Holds `files` for `key`; returns the chips' links to put in the note. */
export function holdFiles(key: string, files: File[]): DraftRef[] {
  const held = registry.get(key) ?? new Map<string, Held>();
  registry.set(key, held);
  return files.map((raw) => {
    const file = namedFile(raw);
    const id = `held-${++seq}`;
    held.set(id, { file });
    return parseRef(pendingMarkdown(file, id))!;
  });
}

const heldOf = (key: string, ref: DraftRef): Held | undefined => {
  const id = pendingId(ref);
  return id ? registry.get(key)?.get(id) : undefined;
};

/** A preview URL for a held image's chip (undefined for any other link). */
export function heldThumb(key: string, ref: DraftRef): string | undefined {
  const held = heldOf(key, ref);
  if (!held || typeof URL.createObjectURL !== 'function') return undefined;
  return (held.url ??= URL.createObjectURL(held.file));
}

export const heldSize = (key: string, ref: DraftRef): number | undefined => heldOf(key, ref)?.file.size;

/** Forgets everything held for `key` (object URLs revoked). */
export function releaseHeld(key: string): void {
  for (const held of registry.get(key)?.values() ?? []) if (held.url) URL.revokeObjectURL?.(held.url);
  registry.delete(key);
}

/**
 * Stores the held files `body` still names, one at a time, and returns the body with their real links.
 * A file that fails (the uploader says why) is dropped from the text. Everything held for `key` is
 * released afterwards, referenced or not.
 */
export async function commitHeld(
  key: string,
  body: string,
  upload: (file: File) => Promise<Attachment | undefined>,
): Promise<string> {
  const ids = [...body.matchAll(/\(pending:([^)]*)\)/g)].map((m) => m[1]!);
  if (ids.length === 0) {
    releaseHeld(key);
    return body;
  }
  const resolved: Array<{ id: string; markdown: string | null }> = [];
  for (const id of ids) {
    const held = registry.get(key)?.get(id);
    const item = held ? await upload(held.file) : undefined;
    resolved.push({ id, markdown: item?.markdown ?? null });
  }
  releaseHeld(key);
  return resolvePending(body, resolved);
}
