/**
 * "Send as note" mode: a note is a message the owner saves into a chat that
 * is shown in the timeline and NEVER sent to the agent (`oino.note` custom
 * entry, engine `append_note`). The mode is per chat, lives in memory only
 * (a reload or a chat switch turns it off) and stays on until the owner turns
 * it off: the send menu's item again, or the ✕ on the toolbar's Note chip.
 */
import { createSignal } from 'solid-js';
import type { OutboxImage } from '../session/types';
import type { RpcCommand } from '../host/client';

const [chats, setChats] = createSignal<ReadonlySet<string>>(new Set());

/** Whether the chat with this session key is in note mode (reactive). */
export const noteMode = (key: string): boolean => chats().has(key);

export function setNoteMode(key: string, on: boolean): void {
  if (chats().has(key) === on) return;
  const next = new Set(chats());
  if (on) next.add(key);
  else next.delete(key);
  setChats(next);
}

/** Over this many base64 chars the images go through the blob channel (a frame is 1 MiB). */
const INLINE_LIMIT = 700 * 1024;

/** The `append_note` command: the text verbatim, images inline or as blob refs. */
export async function noteCommand(hostId: string, text: string, images: readonly OutboxImage[] = []): Promise<RpcCommand> {
  if (!images.length) return { type: 'append_note', text };
  if (images.reduce((n, image) => n + image.data.length, 0) <= INLINE_LIMIT) {
    return { type: 'append_note', text, images: images.map((i) => ({ type: 'image', data: i.data, mimeType: i.mimeType })) };
  }
  const { blobTransport } = await import('../blob');
  const refs: Array<{ type: 'image'; mediaRef: string; mimeType: string }> = [];
  for (const image of images) {
    const bytes = Uint8Array.from(atob(image.data), (c) => c.charCodeAt(0));
    const blob = await blobTransport().putBytes(hostId, bytes, image.mimeType);
    refs.push({ type: 'image', mediaRef: blob.sha256, mimeType: image.mimeType });
  }
  return { type: 'append_note', text, images: refs };
}
