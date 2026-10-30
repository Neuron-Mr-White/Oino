/**
 * Compaction divider (WP-A6, Z-35 — kept unipi, moved+adapted from
 * Transcript.tsx `Compaction`): "Context compacted · Nk tokens before",
 * expandable summary. The open state lives in the expansion store so a
 * virtual-scroll remount keeps it.
 */
import { createSignal, For, Show } from 'solid-js';
import { renderBlocks } from '../../lib/chat/markdown';
import { expansionOpen, setExpansion } from '../../lib/timeline/expansion';
import type { SessionEntry } from '../../lib/session/types';
import { MdBlock } from '../chat/MdBlock';

export function Compaction(props: { entry: SessionEntry; sessionId: () => string }) {
  const key = () => `cmp:${props.entry.id}`;
  const open = () => expansionOpen(props.sessionId(), key(), false);
  const toggle = () => setExpansion(props.sessionId(), key(), !open());
  const e = () => props.entry as { summary?: string; tokensBefore?: number };
  return (
    <div class="compaction tl-compaction" data-testid="tl-compaction">
      <button type="button" class="compaction-head" aria-expanded={open()} onClick={toggle}>
        <span class="compaction-line" />
        <span>Context compacted{e().tokensBefore ? ` · ${Math.round(e().tokensBefore! / 1000)}k tokens before` : ''}</span>
        <span class="compaction-line" />
      </button>
      <Show when={open() && e().summary}>
        <div class="compaction-body md">
          <For each={renderBlocks(e().summary!, false)}>{(html) => <MdBlock html={html} />}</For>
        </div>
      </Show>
    </div>
  );
}
