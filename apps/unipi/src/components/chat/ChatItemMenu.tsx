/**
 * Right-click / long-press menu for an image or file shown in the chat (owner
 * spec 2026-11-03): "Add to chat" puts it back into this chat's composer as an
 * inline chip at the caret (`lib/chat/addToChat`). One instance per
 * conversation, next to the file-ref menu.
 */
import { Show } from 'solid-js';
import { MessageSquarePlus } from 'lucide-solid';
import { AnchoredMenu, MenuRow } from '../board/AnchoredMenu';
import { addToChat, chatItemMenu, closeChatItemMenu } from '../../lib/chat/addToChat';
import { showActionNotice } from '../../lib/timeline/actions';

export function ChatItemMenu() {
  const m = () => chatItemMenu()!;
  return (
    <Show when={chatItemMenu()}>
      <AnchoredMenu anchor={m().rect} label="Image and file actions" title={m().item.name} onDismiss={closeChatItemMenu} width={220}>
        <MenuRow
          icon={<MessageSquarePlus size={18} />}
          label="Add to chat"
          testId="chat-item-add"
          onSelect={() => {
            const { chat, item } = m();
            closeChatItemMenu();
            void addToChat(chat, item).catch((error: unknown) => showActionNotice(`Couldn't add ${item.name}: ${(error as { message?: string })?.message ?? String(error)}`));
          }}
        />
      </AnchoredMenu>
    </Show>
  );
}
