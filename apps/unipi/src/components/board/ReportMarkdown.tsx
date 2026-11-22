/**
 * A ticket report rendered like a chat reply (the Workbench side pane): the same block renderer
 * and `MdBlock` the timeline uses for assistant text (`.chat-prose`: code fences with Copy/Wrap,
 * tap-to-copy inline code, link menu, mermaid), with the board's `att:` refs resolved against the
 * open project. Inline attachments open like they do in the ticket (viewer / reader / save).
 */
import { createMemo, For } from 'solid-js';
import { renderBlocks } from '../../lib/chat/markdown';
import { attDisplayName, attPath } from '../../lib/board/markdown';
import { registerMarkdownSource } from '../../lib/markdown/copyAsMarkdown';
import { boardStore } from '../../lib/board/store';
import { MdBlock } from '../chat/MdBlock';
import { openFile } from './Markdown';

export function ReportMarkdown(props: { text: string; class?: string }) {
  const hostId = () => boardStore().target()?.hostId;
  // `att:` resolution depends on the open project, so it is read when the text or the project changes.
  const blocks = createMemo(() => renderBlocks(props.text, false, { resolveAttachment: attPath, allowImages: true }));
  let el: HTMLDivElement | undefined;
  const onClick = (event: MouseEvent) => {
    const anchor = (event.target as HTMLElement).closest<HTMLAnchorElement>('a[data-att-path]');
    if (!anchor || !el?.contains(anchor)) return;
    event.preventDefault();
    event.stopPropagation();
    const path = anchor.dataset.attPath!;
    if (anchor.classList.contains('att-image')) {
      const images = [...el.querySelectorAll<HTMLAnchorElement>('a.att-image')].map((node) => ({
        path: node.dataset.attPath ?? '',
        name: node.querySelector('img')?.alt || attDisplayName(node.dataset.attPath?.split('/').pop() ?? ''),
      }));
      void openFile(path, path.split('/').pop() ?? '', images);
    } else void openFile(path, anchor.dataset.attName ?? path.split('/').pop() ?? '');
  };
  return (
    <div class={`chat-prose md tl-prose ${props.class ?? ''}`} ref={(node) => { el = node; registerMarkdownSource(node, () => props.text); }} onClick={onClick}>
      <For each={blocks()}>{(html) => <MdBlock html={html} class="chat-block" attHost={hostId()} />}</For>
    </div>
  );
}
