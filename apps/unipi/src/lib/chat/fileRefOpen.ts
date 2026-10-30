/**
 * What a tapped/long-pressed file ref does (UNI-204):
 * - tap a file → the previewer (images, markdown, code…); with a line
 *   (`a.ts:42`) → the editor scrolled to that line;
 * - tap a dir → the Files screen at that dir (Back returns to the chat);
 * - long-press / right-click → Copy path, Open in Files, Mention (`@path`
 *   into this chat's draft).
 * The path is always the bridge-resolved absolute one (`paths_stat`).
 */
import { createSignal } from 'solid-js';
import type { Chat } from './store';
import { openFilePath } from '../media/pathTokens';
import { setEditorTarget } from '../editorTarget';
import { setFilesTarget } from '../filesTarget';
import { hideChat, markReturnToChat } from '../session/open';
import { queueMention } from './mention';

export type FileRefTarget = { resolved: string; kind: 'file' | 'dir'; line?: number };

const go = (hash: string) => {
  if (typeof location !== 'undefined') location.hash = hash;
};

export function openFileRef(chat: Chat, ref: FileRefTarget): Promise<void> {
  if (ref.kind === 'dir') {
    openInFiles(chat, ref);
    return Promise.resolve();
  }
  if (ref.line) {
    // The editor overlays the chat; its Back returns here (Editor.leave → /chat).
    setEditorTarget({ hostId: chat.hostId, path: ref.resolved, line: ref.line });
    go('#/files/edit');
    return Promise.resolve();
  }
  return openFilePath(chat, ref.resolved);
}

/** The Files screen at `ref` (a dir) or the folder holding it (a file). */
export function openInFiles(chat: Chat, ref: FileRefTarget): void {
  const dir = ref.kind === 'dir' ? ref.resolved.replace(/(.)\/+$/, '$1') : ref.resolved.replace(/\/[^/]*$/, '') || '/';
  setFilesTarget({ hostId: chat.hostId, path: dir });
  hideChat();
  markReturnToChat();
  go('#/files');
}

/** `@path` as pi's own `@` finder writes it: relative to the cwd when inside it, quoted with spaces. */
export function mentionText(resolved: string, kind: 'file' | 'dir', cwd?: string): string {
  let p = resolved;
  const root = cwd?.replace(/\/+$/, '');
  if (root && (p === root || p.startsWith(`${root}/`))) p = p.slice(root.length + 1) || '.';
  if (kind === 'dir' && !p.endsWith('/')) p += '/';
  return /[\s"']/.test(p) ? `@"${p}"` : `@${p}`;
}

export function mentionFileRef(chat: Chat, ref: FileRefTarget): void {
  queueMention(`${chat.hostId}:${chat.session}`, `${mentionText(ref.resolved, ref.kind, chat.state?.session?.cwd)} `);
}

// ---- long-press menu state (rendered by components/chat/FileRefMenu.tsx) ----

export type FileRefMenuState = { chat: Chat; ref: FileRefTarget; rect: DOMRect };
const [menu, setMenu] = createSignal<FileRefMenuState | undefined>(undefined);
export const fileRefMenu = menu;
export function openFileRefMenu(chat: Chat, ref: FileRefTarget, anchor: Element): void {
  setMenu({ chat, ref, rect: anchor.getBoundingClientRect() });
}
export function closeFileRefMenu(): void {
  setMenu(undefined);
}
