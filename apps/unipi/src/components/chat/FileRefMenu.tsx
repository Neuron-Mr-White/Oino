/**
 * Long-press / right-click menu for a file ref in chat (UNI-204): Open,
 * Copy path, Open in Files, Mention. Signal-driven (lib/chat/fileRefOpen)
 * so MdBlock, bubbles and tool output share one instance, mounted once in
 * ChatView.
 */
import { createSignal, Show } from 'solid-js';
import { AtSign, Copy, ExternalLink, FolderOpen } from 'lucide-solid';
import { AnchoredMenu, MenuRow } from '../board/AnchoredMenu';
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
        <MenuRow
          icon={<AtSign size={18} />}
          label="Mention"
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
