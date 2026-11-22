/**
 * Thinking row (WP-A8, Z-41 — hybrid): ZCode's labels — "Thinking…" with
 * the shimmer while live, "Thought for 12s" when settled — with unipi's
 * one-line preview of the latest finished paragraph. Hidden entirely when
 * the part is empty. The full text mounts only while open (expansion).
 *
 * The duration is the model's time on that message: from the thinking's
 * entry timestamp to the next entry's (the tool result / next message).
 * Streaming parts (`stream-*` keys) and missing timestamps fall back to a
 * plain "Thought".
 */
import { Show } from 'solid-js';
import { Brain, ChevronRight } from 'lucide-solid';
import { formatDuration, thinkingPreview } from '../../../lib/chat/model';
import { expansionOpen, toggleExpansion } from '../../../lib/timeline/expansion';
import type { SessionEntry } from '../../../lib/session/types';
import type { TimelineCtx } from '../context';
import { Prose } from '../Prose';

const tsOf = (e: SessionEntry | undefined): number | undefined => {
  const m = e?.message?.timestamp;
  if (typeof m === 'number' && Number.isFinite(m)) return m;
  const t = e?.timestamp ? Date.parse(e.timestamp) : NaN;
  return Number.isFinite(t) ? t : undefined;
};

/** How long the model thought before the entry after `itemKey`'s landed. */
export function thoughtForMs(ctx: TimelineCtx, itemKey: string): number | undefined {
  if (itemKey.startsWith('stream-')) return undefined;
  const entryId = itemKey.slice(0, itemKey.lastIndexOf('-'));
  const at = ctx.session.state.byId[entryId];
  if (at === undefined) return undefined;
  const from = tsOf(ctx.session.state.entries[at]);
  const to = tsOf(ctx.session.state.entries[at + 1]);
  return from !== undefined && to !== undefined && to > from ? to - from : undefined;
}

export function ThinkingRow(props: { item: { key: string; text: string; live: boolean }; ctx: TimelineCtx }) {
  const i = () => props.item;
  const empty = () => !i().text.trim() && !i().live;
  const open = () => expansionOpen(props.ctx.sessionId(), `think:${i().key}`, false);
  const toggle = () => toggleExpansion(props.ctx.sessionId(), `think:${i().key}`, false);
  const label = () => {
    if (i().live) return 'Thinking…';
    const ms = thoughtForMs(props.ctx, i().key);
    return ms !== undefined ? `Thought for ${formatDuration(ms)}` : 'Thought';
  };
  const preview = () => thinkingPreview(i().text, i().live);
  const fullTitle = () => (preview() ? `${label()} · ${preview()}` : label());
  return (
    <Show when={!empty()}>
      <div class="think tl-think" data-live={i().live ? '' : undefined} data-testid="tl-think">
        <button type="button" class="tool-row" aria-expanded={open()} onClick={toggle} title={fullTitle()}>
          <span class="tool-icon">
            <Brain size={15} />
          </span>
          <span class="tool-name" classList={{ shimmer: i().live }}>
            {label()}
          </span>
          <Show when={preview()}>
            <>
              <span class="tool-sep" />
              <span class="tool-subject tl-think-preview">{preview()}</span>
            </>
          </Show>
          <ChevronRight size={15} class="proc-chev" data-open={open() ? '' : undefined} />
        </button>
        <Show when={open() && i().text.trim()}>
          <div class="think-body md">
            <Prose text={i().text} live={i().live} entryKey={i().key} chat={props.ctx.refs} width={props.ctx.columnWidth} />
          </div>
        </Show>
      </div>
    </Show>
  );
}
