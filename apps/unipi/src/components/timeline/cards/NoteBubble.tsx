/**
 * The owner's note ("Send as note", `oino.note` custom entry): a right-aligned bubble like the
 * owner's message, but visibly different — a dashed gold edge, the note icon and the caption
 * "Note · only you". It is saved in the session and shown here, and the agent never reads it.
 * Markdown is rendered like in any message; attached images sit under the bubble.
 */
import { For, Show } from 'solid-js';
import { StickyNote } from 'lucide-solid';
import type { SessionEntry } from '../../../lib/session/types';
import type { Part } from '../../../lib/chat/model';
import { imagesOf } from '../../../lib/chat/model';
import { renderBlocks } from '../../../lib/chat/markdown';
import { MdBlock } from '../../chat/MdBlock';
import { ImageChipRow } from '../../chat/ImageChip';
import type { TimelineCtx } from '../context';

export function NoteBubble(props: { entry: SessionEntry; ctx: TimelineCtx }) {
  const data = () => (props.entry.data ?? {}) as { text?: unknown; images?: unknown };
  const text = () => (typeof data().text === 'string' ? (data().text as string) : '');
  const images = () => imagesOf(Array.isArray(data().images) ? (data().images as Part[]) : undefined);
  return (
    <div class="chat-user tl-user tl-note" data-testid="tl-note" data-event="note">
      <span class="tl-note-cap" data-testid="tl-note-caption">
        <StickyNote size={12} />
        Note · only you
      </span>
      <Show when={text().trim()}>
        <div class="chat-bubble tl-bubble tl-note-bubble">
          <div class="md tl-note-body" data-testid="tl-note-body">
            <For each={renderBlocks(text(), false)}>{(html) => <MdBlock html={html} chat={props.ctx.refs} />}</For>
          </div>
        </div>
      </Show>
      <ImageChipRow chat={props.ctx.refs} images={images()} />
    </div>
  );
}
