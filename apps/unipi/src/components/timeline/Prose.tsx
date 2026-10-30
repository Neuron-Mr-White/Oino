/**
 * Assistant prose (WP-A6, moved+adapted from Transcript.tsx `Prose`):
 * a settled part renders **once** through the markdown cache (Z-34 static
 * rule — virtual-scroll remounts and fold toggles reuse the HTML, no
 * fade-in); only the truly live part takes `renderBlocks(text, live)`.
 */
import { createEffect, createMemo, createSignal, For, onCleanup, Show } from 'solid-js';
import type { RefChat } from '../../lib/chat/pathStat';
import { renderBlocks } from '../../lib/chat/markdown';
import { mdBlocks } from '../../lib/timeline/mdCache';
import { attachMessageMenu, attachQuoteSwipe, type MessageTarget } from '../../lib/timeline/actions';
import { MdBlock } from '../chat/MdBlock';

export function Prose(props: {
  text: string;
  live?: boolean;
  stopped?: boolean;
  /** `entryId-partIndex` for the static cache (required for settled text to be cached). */
  entryKey?: string;
  chat?: RefChat;
  /** Column width for the cache's width class. */
  width?: () => number;
  /** The row menu's payload (assistant message: Copy/Quote/Fork). */
  menu?: () => MessageTarget | undefined;
  /** Where Quote puts the chip (the session's composer key). */
  composerKey?: string;
}) {
  const blocks = createMemo(() => {
    if (props.live) return renderBlocks(props.text, true);
    if (props.entryKey) return mdBlocks(props.entryKey, props.width?.() ?? 720, props.text);
    return renderBlocks(props.text, false);
  });
  const [rootEl, setRootEl] = createSignal<HTMLDivElement>();
  createEffect(() => {
    const el = rootEl();
    if (!el) return;
    const menu = attachMessageMenu(el, () => props.menu?.());
    const swipe = props.composerKey ? attachQuoteSwipe(el, () => props.text, () => props.composerKey!) : undefined;
    onCleanup(() => {
      menu();
      swipe?.();
    });
  });
  return (
    <div class="chat-prose md tl-prose" ref={setRootEl} data-live={props.live ? '' : undefined}>
      <For each={blocks()}>{(html) => <MdBlock html={html} class="chat-block" chat={props.chat} />}</For>
      <Show when={props.stopped}>
        <span class="chat-stopped">Stopped</span>
      </Show>
    </div>
  );
}
