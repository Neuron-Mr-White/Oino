/**
 * Compaction divider (WP-A6, Z-35 — kept unipi, moved+adapted from
 * Transcript.tsx `Compaction`): "Context compacted · Nk tokens before",
 * expandable summary. The open state lives in the expansion store so a
 * virtual-scroll remount keeps it.
 */
import { For, Show } from 'solid-js';
import { ChevronDown } from 'lucide-solid';
import { renderBlocks } from '../../lib/chat/markdown';
import { expansionOpen, setExpansion } from '../../lib/timeline/expansion';
import { AUDIENCE_LABEL, AUDIENCE_TITLE } from '../../lib/timeline/eventKinds';
import type { SessionEntry } from '../../lib/session/types';
import { MdBlock } from '../chat/MdBlock';

export function Compaction(props: { entry: SessionEntry; sessionId: () => string }) {
  const key = () => `cmp:${props.entry.id}`;
  // Shown open by default so the compaction summary shared with both user and agent is immediately readable.
  const open = () => expansionOpen(props.sessionId(), key(), true);
  const toggle = () => setExpansion(props.sessionId(), key(), !open());
  const e = () => props.entry as { summary?: string; tokensBefore?: number };
  return (
    <div class="compaction tl-compaction" data-audience="both" data-testid="tl-compaction">
      <button type="button" class="compaction-head" aria-expanded={open()} onClick={toggle}>
        <span class="compaction-line" />
        <span>Context compacted{e().tokensBefore ? ` · ${Math.round(e().tokensBefore! / 1000)}k tokens before` : ''}</span>
        <span class="tl-audience" data-audience="both" title={AUDIENCE_TITLE.both} data-testid="tl-compaction-audience">
          {AUDIENCE_LABEL.both}
        </span>
        <Show when={e().summary}>
          <ChevronDown size={13} class="compaction-chev" data-open={open() ? '' : undefined} />
        </Show>
        <span class="compaction-line" />
      </button>
      <Show when={open() && e().summary}>
        <div class="compaction-body md" data-testid="tl-compaction-body">
          <For each={renderBlocks(e().summary!, false)}>{(html) => <MdBlock html={html} />}</For>
        </div>
      </Show>
    </div>
  );
}
