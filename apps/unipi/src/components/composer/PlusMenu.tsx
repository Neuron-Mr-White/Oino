/**
 * The composer's `+` menu (WP-A12, Z-63): Photo, Camera, File, Side question,
 * Board task… — the unipi entries minus New session / Resume (Z-63 ruling).
 * Photo/Camera/File open pickers (images attach inline; generic files upload
 * with WP-A13's attachments); Side question opens the `btw` side-chat tab
 * (WP-A10) carrying the draft's text; Board task… opens the board's New task
 * sheet for this project (UNI-181 portal).
 */
import { Camera, FilePlus, Image as ImageIcon, ListPlus, MessageSquareText } from 'lucide-solid';
import { Show } from 'solid-js';
import { AnchoredMenu, MenuRow } from '../board/AnchoredMenu';

export function PlusMenu(props: {
  anchor: DOMRect;
  onDismiss: () => void;
  onPhoto: () => void;
  /** Phone / tablet only: a desktop has no camera to hand over. */
  onCamera?: () => void;
  onFile: () => void;
  /** Absent in the new-chat draft (a side question needs a chat). */
  onSideQuestion?: () => void;
  onAddTask: () => void;
}) {
  return (
    <AnchoredMenu anchor={props.anchor} alignEnd width={248} label="Add to the message" onDismiss={props.onDismiss}>
      <MenuRow icon={<ImageIcon size={18} />} label="Photo" onSelect={() => (props.onDismiss(), props.onPhoto())} />
      <Show when={props.onCamera}>
        <MenuRow icon={<Camera size={18} />} label="Camera" onSelect={() => (props.onDismiss(), props.onCamera!())} />
      </Show>
      <MenuRow icon={<FilePlus size={18} />} label="File" onSelect={() => (props.onDismiss(), props.onFile())} />
      <Show when={props.onSideQuestion}>
        <MenuRow icon={<MessageSquareText size={18} />} label="Side question" onSelect={() => (props.onDismiss(), props.onSideQuestion!())} />
      </Show>
      <MenuRow icon={<ListPlus size={18} />} label="Board task…" onSelect={() => (props.onDismiss(), props.onAddTask())} />
    </AnchoredMenu>
  );
}
