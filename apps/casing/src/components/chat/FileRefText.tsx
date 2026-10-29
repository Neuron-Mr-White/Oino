/**
 * Plain text with verified, tappable file refs (UNI-204): user bubbles and
 * tool output (grep/find/ls/bash results, tool subjects). The text renders
 * immediately; refs light up once `paths_stat` confirms them. Plain runs
 * between refs go through `plain` (task refs in bubbles).
 */
import { createEffect, createResource, For, onCleanup, type JSX } from 'solid-js';
import type { Chat } from '../../lib/chat/store';
import type { ScanMode } from '../../lib/chat/fileRefScan';
import { fileRefHits, splitByHits, type FileRefHit } from '../../lib/chat/fileRefs';
import { attachFileRefGestures, handleFileRefClick } from '../../lib/chat/fileRefGestures';

export function FileRefText(props: { chat?: Chat; text: string; mode: ScanMode; base?: string; plain?: (text: string) => JSX.Element; class?: string }) {
  let root: HTMLSpanElement | undefined;
  const [hits] = createResource(
    () => (props.chat && props.text ? { chat: props.chat, text: props.text, mode: props.mode, base: props.base } : undefined),
    (k) => fileRefHits(k.chat, k.text, k.mode, k.base).catch(() => [] as FileRefHit[]),
  );
  const parts = () => {
    const h = hits.latest;
    return h?.length ? splitByHits(props.text, h) : [props.text];
  };
  createEffect(() => {
    if (!root || !props.chat) return;
    onCleanup(attachFileRefGestures(root, () => props.chat));
  });
  return (
    <span ref={root} class={props.class} onClick={(e) => root && handleFileRefClick(e, root, props.chat)}>
      <For each={parts()}>
        {(part) =>
          typeof part === 'string' ? (
            (props.plain?.(part) ?? part)
          ) : (
            <span
              class="file-ref"
              role="link"
              data-file-ref={part.hit.resolved}
              data-file-kind={part.hit.kind}
              data-file-line={part.hit.line}
              title={part.hit.line ? `${part.hit.resolved}:${part.hit.line}` : part.hit.resolved}
            >
              {part.text}
            </span>
          )
        }
      </For>
    </span>
  );
}
