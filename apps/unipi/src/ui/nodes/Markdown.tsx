/**
 * `Markdown` (ui-protocol §4.5.2) through the chat's `renderChatMarkdown`
 * (marked + DOMPurify, `.md` tokens): code blocks get the sticky language +
 * Copy header and lazy highlighting; mermaid fences open `MermaidViewer` on
 * tap; links open externally via `openLink` (not an action). Sources over
 * 2 000 lines render in ~500-line chunks with `content-visibility: auto`.
 *
 * This file is the **only** place in `src/ui/` that writes HTML strings into
 * the DOM (§13.3 rule; `__tests__/noInnerHtml.test.ts` enforces it).
 */
import { For, createEffect, createMemo, on } from 'solid-js';
import { renderChatMarkdown, hydrateCode } from '../../lib/chat/markdown';
import { copyText } from '../../lib/board/share';
import { anchorFromEvent, isExternalHref, openLink } from '../../lib/markdown/links';
import { openMermaidViewer } from '../../components/chat/MermaidViewer';
import type { MarkdownNode } from '../types';
import { baseStyle, cx, type NodeProps } from './props';

/** Lines per chunk once a source is long (§13.4). */
const CHUNK_LINES = 500;
const CHUNK_AFTER = 2000;

/**
 * Writes trusted HTML into `el`. Callers pass only sanitized markdown
 * (DOMPurify) or highlighter output built from escaped text.
 */
export function setTrustedHtml(el: HTMLElement, html: string): void {
  el.innerHTML = html;
}

/** Splits a long source at blank lines near every 500th line, outside fences. */
export function chunkMarkdown(source: string): string[] {
  const lines = source.split('\n');
  if (lines.length <= CHUNK_AFTER) return [source];
  const out: string[] = [];
  let start = 0;
  let fence = false;
  for (let i = 0; i < lines.length; i++) {
    if (/^\s*(```|~~~)/.test(lines[i]!)) fence = !fence;
    if (!fence && i - start >= CHUNK_LINES && lines[i]!.trim() === '') {
      out.push(lines.slice(start, i).join('\n'));
      start = i + 1;
    }
  }
  out.push(lines.slice(start).join('\n'));
  return out.filter((c) => c.trim() !== '');
}

/** Click handling shared by markdown containers: Copy, wrap toggle, mermaid tap, links. */
export function onMarkdownClick(event: MouseEvent, root: HTMLElement): void {
  const target = event.target as HTMLElement;
  const copy = target.closest<HTMLButtonElement>('[data-copy]');
  if (copy && root.contains(copy)) {
    const code = copy.closest('.cb')?.querySelector('code')?.textContent ?? '';
    void copyText(code).then((ok) => {
      copy.textContent = ok ? 'Copied' : 'Copy';
      setTimeout(() => (copy.textContent = 'Copy'), 1200);
    });
    return;
  }
  const wrap = target.closest<HTMLButtonElement>('[data-wrap-toggle]');
  if (wrap && root.contains(wrap)) {
    wrap.closest('.cb')?.classList.toggle('wrapped');
    return;
  }
  const mmd = target.closest<HTMLElement>('[data-mmd-tap]');
  if (mmd && root.contains(mmd)) {
    const source = decodeURIComponent(mmd.closest<HTMLElement>('.mmd')?.dataset.mmdSource ?? '');
    openMermaidViewer({ svg: mmd.innerHTML, source });
    return;
  }
  const anchor = anchorFromEvent(event, root);
  if (!anchor) return;
  event.preventDefault();
  event.stopPropagation();
  const href = anchor.getAttribute('href') ?? '';
  if (isExternalHref(href)) void openLink(href);
}

function Chunk(props: { source: string; chunked: boolean }) {
  let el!: HTMLDivElement;
  createEffect(
    on(
      () => props.source,
      (source) => {
        setTrustedHtml(el, renderChatMarkdown(source));
        void hydrateCode(el);
      },
    ),
  );
  return <div ref={el} class={props.chunked ? 'ui-md-chunk' : undefined} />;
}

export function Markdown(props: NodeProps<MarkdownNode>) {
  let root!: HTMLDivElement;
  const chunks = createMemo(() => chunkMarkdown(typeof props.node.source === 'string' ? props.node.source : ''));
  return (
    <div
      ref={root}
      class={cx('md ui-md', baseStyle(props.node).class)}
      data-mode={props.mode}
      style={baseStyle(props.node).style}
      onClick={(e) => onMarkdownClick(e, root)}
    >
      <For each={chunks()}>{(c) => <Chunk source={c} chunked={chunks().length > 1} />}</For>
    </div>
  );
}
