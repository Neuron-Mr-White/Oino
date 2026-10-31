/**
 * Segment fold (WP-A6, Z-21/Z-54/Z-56): a work segment's header —
 * "Working for 12s" live, "Worked for 1m 3s · read 4 files, edited 2"
 * settled (unipi wording from `foldDetail`), "Stopped" without a duration
 * when interrupted. Forced open while running / stopped / error; otherwise
 * the user's toggle is remembered in the expansion store across
 * virtual-scroll remounts.
 *
 * The fold body is lazy: work DOM only exists while open. WP-A8 moved the
 * work-item renderers to `tools/` (rows, groups, thinking, bodies) and
 * `cards/` (native + extension-rendered cards).
 */
import { createSignal, onCleanup, Show } from 'solid-js';
import { Dynamic } from 'solid-js/web';
import { Brain, ChevronRight } from 'lucide-solid';
import { formatDuration, toolCategory, type ToolCategory } from '../../lib/chat/model';
import { createPresence } from '../../lib/motion';
import { expansionOpen, toggleExpansion } from '../../lib/timeline/expansion';
import { workingMessageOf } from '../../ui/slots/effects';
import type { Segment } from '../../lib/session/turns';
import { TOOL_ICON } from './tools/toolIcons';
import { WorkNodes } from './tools/WorkItem';
import type { TimelineCtx } from './context';

/** 1 s clock for the running fold's "Working for". */
function useNow(live: () => boolean): () => number {
  const [now, setNow] = createSignal(Date.now());
  const t = setInterval(() => live() && setNow(Date.now()), 1000);
  onCleanup(() => clearInterval(t));
  return now;
}

export function SegmentFold(props: { segment: Segment; ctx: TimelineCtx }) {
  const fold = () => props.segment.fold!;
  const open = () => fold().forcedOpen || expansionOpen(props.ctx.sessionId(), `fold:${props.segment.id}`, false);
  const toggle = () => !fold().forcedOpen && toggleExpansion(props.ctx.sessionId(), `fold:${props.segment.id}`, false);
  const running = () => fold().state === 'running';
  const now = useNow(running);
  const label = () => {
    if (!running()) return fold().label;
    // §3.2 `setWorkingMessage`: the working line's text (default "Working for").
    const msg = workingMessageOf(props.ctx.session.key);
    const dur = formatDuration(Math.max(0, now() - (fold().startedAt ?? now())));
    return msg ? `${msg} · ${dur}` : `Working for ${dur}`;
  };
  const lead = () => {
    const last = [...props.segment.work].reverse().find((w) => w.kind === 'tool');
    return last && last.kind === 'tool' ? toolCategory(last.tool.name) : 'think';
  };
  const anyError = () => props.segment.work.some((w) => (w.kind === 'tool' && w.tool.state === 'error') || w.kind === 'error');
  const fullTitle = () => (fold().detail ? `${label()} · ${fold().detail}` : label());
  // Z-91: closing the fold fades the body out (160 ms) instead of dropping it.
  const body = createPresence(open);
  return (
    <div class="proc tl-fold" data-live={running() ? '' : undefined} data-open={open() ? '' : undefined} data-state={fold().state} data-testid="tl-fold">
      <button type="button" class="proc-head" aria-expanded={open()} onClick={toggle} title={fullTitle()}>
        <span class="proc-icon">
          <Show when={lead() !== 'think'} fallback={<Brain size={15} />}>
            <Dynamic component={TOOL_ICON[lead() as ToolCategory]} size={15} />
          </Show>
        </span>
        <span class="proc-title" classList={{ shimmer: running() }}>
          {label()}
        </span>
        <Show when={fold().detail}>
          <span class="proc-detail" data-error={anyError() && !running() ? '' : undefined}>
            {fold().detail}
          </span>
        </Show>
        <Show when={!fold().forcedOpen}>
          <ChevronRight size={15} class="proc-chev" />
        </Show>
      </button>
      <Show when={body.mounted()}>
        <div class="proc-body tl-fold-body" data-leaving={body.leaving() ? '' : undefined}>
          <WorkNodes nodes={props.segment.nodes} ctx={props.ctx} live={running()} />
        </div>
      </Show>
    </div>
  );
}

/** Re-exported for A6's callers (preamble turns render loose work). */
export { WorkNodes };
