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
import type { RefChat } from './pathStat';
import { openFilePath } from '../media/pathTokens';
import { setEditorTarget, switchEditorTarget } from '../editorTarget';
import { setFilesTarget } from '../filesTarget';
import { desktopShell, layout } from '../layout';
import { appNavigate } from '../nav';
import { panes } from '../panes';
import { markReturnToChat, openChatHref } from '../session/open';
import { queueMention } from './mention';

export type FileRefTarget = { resolved: string; kind: 'file' | 'dir'; line?: number };

export function openFileRef(chat: RefChat, ref: FileRefTarget): Promise<void> {
  if (ref.kind === 'dir') {
    openInFiles(chat, ref);
    return Promise.resolve();
  }
  if (ref.line) {
    const next = { hostId: chat.hostId, path: ref.resolved, line: ref.line };
    if (desktopShell()) {
      const root = chat.cwd?.() || '~';
      switchEditorTarget(next);
      panes().open({ hostId: chat.hostId, projectKey: root }, 'side', { sessionId: chat.session }, { kind: 'files', params: { hostId: chat.hostId, root, path: ref.resolved } });
      layout().toggleSide(true);
      return Promise.resolve();
    }
    markReturnToChat(chat.session ? openChatHref(chat.hostId, chat.session) : undefined);
    setEditorTarget(next);
    appNavigate('/files/edit');
    return Promise.resolve();
  }
  return openFilePath(chat, ref.resolved);
}

/** The Files surface at `ref` (a dir) or the folder holding it (a file). */
export function openInFiles(chat: RefChat, ref: FileRefTarget): void {
  const dir = ref.kind === 'dir' ? ref.resolved.replace(/(.)\/+$/, '$1') : ref.resolved.replace(/\/[^/]*$/, '') || '/';
  setFilesTarget({ hostId: chat.hostId, path: dir });
  if (desktopShell()) {
    const root = chat.cwd?.() || dir;
    panes().open({ hostId: chat.hostId, projectKey: root }, 'side', { sessionId: chat.session }, { kind: 'files', params: { hostId: chat.hostId, root: dir } });
    layout().toggleSide(true);
    return;
  }
  markReturnToChat(chat.session ? openChatHref(chat.hostId, chat.session) : undefined);
  appNavigate('/files');
}

/** `@path` as pi's own `@` finder writes it: relative to the cwd when inside it, quoted with spaces. */
export function mentionText(resolved: string, kind: 'file' | 'dir', cwd?: string): string {
  let p = resolved;
  const root = cwd?.replace(/\/+$/, '');
  if (root && (p === root || p.startsWith(`${root}/`))) p = p.slice(root.length + 1) || '.';
  if (kind === 'dir' && !p.endsWith('/')) p += '/';
  return /[\s"']/.test(p) ? `@"${p}"` : `@${p}`;
}

export function mentionFileRef(chat: RefChat, ref: FileRefTarget): void {
  queueMention(`${chat.hostId}:${chat.session}`, `${mentionText(ref.resolved, ref.kind, chat.cwd?.())} `);
}

// ---- long-press menu state (rendered by components/chat/FileRefMenu.tsx) ----

export type FileRefMenuState = { chat: RefChat; ref: FileRefTarget; rect: DOMRect };
const [menu, setMenu] = createSignal<FileRefMenuState | undefined>(undefined);
export const fileRefMenu = menu;
export function openFileRefMenu(chat: RefChat, ref: FileRefTarget, anchor: Element): void {
  setMenu({ chat, ref, rect: anchor.getBoundingClientRect() });
}
export function closeFileRefMenu(): void {
  setMenu(undefined);
}
