/**
 * "Add to chat" (owner spec 2026-11-03): an image or file shown in the chat —
 * a sent image, a tool result's image or attachment, a sent file chip, a
 * screenshot a tool saved — goes back into this chat's composer as an inline
 * chip at the caret, by right-click (desktop) or long-press (phone).
 *
 * - images: their bytes (already in the chat, or one `media`/blob read on the
 *   PC) are attached like a pasted image (shrink + encode, no upload — prompt
 *   images travel inline), so the model sees the picture itself;
 * - files: a re-reference, never a re-upload — `[file: name]@<host path>`
 *   (the blob the first send uploaded) or `@path` for a file on the PC.
 *
 * Signal-driven like the file-ref menu: one `ChatItemMenu` per conversation.
 */
import { createSignal } from 'solid-js';
import type { ImageRef } from './model';
import type { RefChat } from './pathStat';
import { queueMention, toComposerSessionKey } from './mention';
import { attach } from '../composer/attachments';
import { focusComposer } from '../composer/drafts';
import { blobTransport } from '../blob';

export type ChatItem =
  /** An image the chat already has (inline bytes or a `mediaRef`). */
  | { kind: 'image'; image: ImageRef; name: string }
  /** An image file on the PC (a tool's saved screenshot). */
  | { kind: 'image-path'; path: string; name: string }
  /** A tool result attachment fetched by `mediaRef` (an image) or referenced by path. */
  | { kind: 'attachment'; path: string; mime: string; mediaRef?: string; name: string }
  /** A file already on the PC (a sent attachment's blob, a tool's file): referenced, not re-sent. */
  | { kind: 'file'; path: string; name: string };

/** The composer draft a chat's items go to. */
export const chatComposerKey = (chat: RefChat): string | undefined => (chat.session ? toComposerSessionKey(`${chat.hostId}:${chat.session}`) : undefined);

const fileFromDataUrl = (url: string, name: string, fallbackMime: string): File => {
  const m = /^data:([^;,]*)(;base64)?,([\s\S]*)$/.exec(url);
  if (!m) throw new Error('Not a data URL');
  const mime = m[1] || fallbackMime;
  let bytes: Uint8Array<ArrayBuffer>;
  if (m[2]) {
    const bin = atob(m[3]!);
    bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  } else bytes = new TextEncoder().encode(decodeURIComponent(m[3]!));
  return new File([bytes], name, { type: mime });
};

const isImageMime = (mime: string, path: string) => mime.startsWith('image/') || /\.(png|jpe?g|gif|webp|bmp|avif)$/i.test(path);

/** Reads a PC file's bytes (allow-listing it first when the host hasn't seen the path). */
async function pcFile(chat: RefChat, path: string, name: string): Promise<File> {
  let result;
  try {
    result = await blobTransport().getBytes(chat.hostId, { path });
  } catch (error) {
    const code = (error as { code?: string })?.code;
    if ((code !== 'E_UNAUTHORIZED' && code !== 'E_NOT_FOUND') || !chat.shareFile) throw error;
    await chat.shareFile(path);
    result = await blobTransport().getBytes(chat.hostId, { path });
  }
  return new File([result.bytes as BlobPart], name, { type: result.mime || 'image/png' });
}

/** The token a file item becomes in the draft. */
export function fileItemToken(item: { path: string; name: string }): string {
  return `[file: ${item.name.replace(/[\]\n]/g, '_')}]@${item.path}`;
}

/** Puts `item` into `chat`'s composer as an inline chip. Resolves once it is in the draft
 *  (an image may still be encoding — its chip shows that). */
export async function addToChat(chat: RefChat, item: ChatItem): Promise<void> {
  const key = chatComposerKey(chat);
  if (!key) throw new Error('No chat to add to');
  if (item.kind === 'file' || (item.kind === 'attachment' && !isImageMime(item.mime, item.path))) {
    queueMention(key, `${fileItemToken(item)} `);
    return;
  }
  let file: File;
  if (item.kind === 'image') {
    const { loadImage } = await import('../../components/chat/ImageChip');
    file = fileFromDataUrl(await loadImage(chat, item.image), item.name, item.image.mime);
  } else if (item.kind === 'attachment' && item.mediaRef && chat.media) {
    file = fileFromDataUrl((await chat.media(item.mediaRef, item.mime)).dataUrl, item.name, item.mime);
  } else file = await pcFile(chat, item.path, item.name);
  const { added } = attach(key, chat.hostId, [file]);
  if (!added.length) throw new Error('This message already has 20 attachments');
  focusComposer(key);
}

/* ── the menu's state (rendered by components/chat/ChatItemMenu.tsx) ── */

export type ChatItemMenuState = { chat: RefChat; item: ChatItem; rect: DOMRect };
const [menu, setMenu] = createSignal<ChatItemMenuState | undefined>(undefined);
export const chatItemMenu = menu;
export function openChatItemMenu(chat: RefChat, item: ChatItem, anchor: Element): void {
  if (!chat.session) return;
  setMenu({ chat, item, rect: anchor.getBoundingClientRect() });
}
export function closeChatItemMenu(): void {
  setMenu(undefined);
}

const LONG_PRESS_MS = 500;
const MOVE_SLOP = 10;

/** Right-click and long-press on one element open its "Add to chat" menu; the click that ends a
 *  long-press is swallowed (so the item doesn't also open). Returns a cleanup. */
export function chatItemGestures(el: HTMLElement, open: () => void): () => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let x = 0;
  let y = 0;
  let pressedAt = 0;
  const clear = () => {
    clearTimeout(timer);
    timer = undefined;
  };
  const fire = () => {
    pressedAt = Date.now();
    open();
  };
  const onTouchStart = (e: TouchEvent) => {
    if (e.touches.length !== 1) return;
    x = e.touches[0]!.clientX;
    y = e.touches[0]!.clientY;
    clear();
    timer = setTimeout(() => {
      timer = undefined;
      fire();
    }, LONG_PRESS_MS);
  };
  const onTouchMove = (e: TouchEvent) => {
    const t = e.touches[0];
    if (timer && (!t || Math.abs(t.clientX - x) > MOVE_SLOP || Math.abs(t.clientY - y) > MOVE_SLOP)) clear();
  };
  const onContextMenu = (e: MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    clear();
    if (Date.now() - pressedAt < 800) return; // the long-press already opened it
    fire();
  };
  const onClick = (e: MouseEvent) => {
    if (Date.now() - pressedAt < 800) {
      e.preventDefault();
      e.stopPropagation();
    }
  };
  el.addEventListener('touchstart', onTouchStart, { passive: true });
  el.addEventListener('touchmove', onTouchMove, { passive: true });
  el.addEventListener('touchend', clear);
  el.addEventListener('touchcancel', clear);
  el.addEventListener('contextmenu', onContextMenu);
  el.addEventListener('click', onClick, true);
  return () => {
    clear();
    el.removeEventListener('touchstart', onTouchStart);
    el.removeEventListener('touchmove', onTouchMove);
    el.removeEventListener('touchend', clear);
    el.removeEventListener('touchcancel', clear);
    el.removeEventListener('contextmenu', onContextMenu);
    el.removeEventListener('click', onClick, true);
  };
}
