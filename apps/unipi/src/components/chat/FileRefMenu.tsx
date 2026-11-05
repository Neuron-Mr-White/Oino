/**
 * Long-press / right-click menu for a file ref in chat (UNI-204): Open,
 * Copy path, Open in Files, Mention. Signal-driven (lib/chat/fileRefOpen)
 * so MdBlock, bubbles and tool output share one instance, mounted once in
 * ChatView.
 */
import { createResource, createSignal, For, Show } from 'solid-js';
import { AppWindow, AtSign, Code2, Copy, ExternalLink, FolderOpen, FolderSearch } from 'lucide-solid';
import { AnchoredMenu, MenuRow, MenuSeparator } from '../board/AnchoredMenu';
import { isOwnPcFile, localEditors, localOpen, type OpenTarget } from '../../lib/openLocal';
import { copyText } from '../../lib/board/share';
import { closeFileRefMenu, fileRefMenu, mentionFileRef, openFileRef, openInFiles } from '../../lib/chat/fileRefOpen';

export function FileRefMenu() {
  const [copied, setCopied] = createSignal(false);
  const close = () => {
    setCopied(false);
    closeFileRefMenu();
  };
  const m = () => fileRefMenu()!;
  const where = () => (m().ref.line ? `${m().ref.resolved}:${m().ref.line}` : m().ref.resolved);
  const ownPc = () => !!fileRefMenu() && isOwnPcFile(m().chat.hostId);
  const [editors] = createResource(() => (ownPc() ? true : undefined), () => localEditors().catch((): OpenTarget[] => []));
  const openLocal = (how: string) => {
    const path = m().ref.resolved;
    close();
    void localOpen(path, how);
  };
  return (
    <Show when={fileRefMenu()}>
      <AnchoredMenu anchor={m().rect} label="File actions" onDismiss={close} width={260}>
        <div class="file-ref-menu-head" title={where()}>
          {where()}
        </div>
        <MenuRow
          icon={<ExternalLink size={18} />}
          label={m().ref.kind === 'dir' ? 'Open folder' : m().ref.line ? `Open at line ${m().ref.line}` : 'Open'}
          onSelect={() => {
            const { chat, ref } = m();
            close();
            void openFileRef(chat, ref).catch(() => {});
          }}
        />
        <MenuRow
          icon={<Copy size={18} />}
          label={copied() ? 'Copied' : 'Copy path'}
          onSelect={() => {
            if (copied()) return close();
            void copyText(m().ref.resolved).then((ok) => {
              if (!ok) return;
              setCopied(true);
              setTimeout(close, 600);
            });
          }}
        />
        <MenuRow
          icon={<FolderOpen size={18} />}
          label="Open in Files"
          onSelect={() => {
            const { chat, ref } = m();
            close();
            openInFiles(chat, ref);
          }}
        />
        <Show when={ownPc()}>
          <MenuSeparator />
          <MenuRow icon={<FolderSearch size={18} />} label="Open in file explorer" testId="fileref-reveal" onSelect={() => openLocal('reveal')} />
          <Show when={m().ref.kind !== 'dir'}>
            <MenuRow icon={<AppWindow size={18} />} label="Open with default app" onSelect={() => openLocal('default')} />
          </Show>
          <For each={editors() ?? []}>{(ed) => <MenuRow icon={<Code2 size={18} />} label={`Open in ${ed.name}`} onSelect={() => openLocal(ed.id)} />}</For>
          <MenuSeparator />
        </Show>
        <MenuRow
          icon={<AtSign size={18} />}
          label="Add to chat"
          onSelect={() => {
            const { chat, ref } = m();
            close();
            mentionFileRef(chat, ref);
          }}
        />
      </AnchoredMenu>
    </Show>
  );
}
