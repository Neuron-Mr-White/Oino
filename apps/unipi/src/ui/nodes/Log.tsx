/**
 * `Log` (ui-protocol §4.5.2, §13.4): a monospace virtual list, fixed 20 px
 * rows (`pre`; measured when `wrap`), a ring of `maxLines` (default 2 000,
 * ≤ 5 000). `follow` (default on) is a single bottom-lock: scrolling up
 * pauses it and shows a "↓ live" pill that resumes it.
 */
import { Show, createEffect, createMemo, createSignal, on } from 'solid-js';
import { ArrowDown } from 'lucide-solid';
import { useEntry } from '../actions';
import { useLocal } from '../localState';
import { logLimit } from '../tree';
import type { LogNode } from '../types';
import { VirtualWindow, useVirtual } from '../virtual';
import { LineView } from './Text';
import { baseStyle, cx, type NodeProps } from './props';

const ROW = 20;
/** Distance from the bottom that still counts as "at the bottom". */
const STICK_PX = 24;

export function Log(props: NodeProps<LogNode>) {
  const entry = useEntry();
  const lines = createMemo(() => {
    const all = Array.isArray(props.node.lines) ? props.node.lines : [];
    const max = logLimit(props.node as unknown as Record<string, unknown>);
    return all.length > max ? all.slice(all.length - max) : all;
  });
  /** Offset of the first shown line in the full sequence (line numbers survive ring trims). */
  const first = () => Math.max(0, (props.node.lines?.length ?? 0) - lines().length);
  const [follow, setFollow] = useLocal<boolean>(entry.entryId, props.path, 'follow', {
    prop: () => props.node.follow,
    init: () => true,
  });
  const [scrollEl, setScrollEl] = createSignal<HTMLElement>();
  const wrap = () => !!props.node.wrap;
  const virt = useVirtual({
    count: () => lines().length,
    scroller: scrollEl,
    estimate: () => ROW,
    key: (i) => first() + i,
    overscan: 12,
  });

  let programmatic = false;
  const toBottom = () => {
    const el = scrollEl();
    if (!el) return;
    programmatic = true;
    requestAnimationFrame(() => {
      el.scrollTop = el.scrollHeight;
      requestAnimationFrame(() => (programmatic = false));
    });
  };
  createEffect(
    on(
      () => [lines().length, follow(), lines()[lines().length - 1]] as const,
      ([, f]) => {
        if (f !== false) toBottom();
      },
    ),
  );
  const onScroll = () => {
    const el = scrollEl();
    if (!el || programmatic) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight <= STICK_PX;
    if (!atBottom && follow()) setFollow(false);
    else if (atBottom && !follow()) setFollow(true);
  };
  const resume = () => {
    setFollow(true);
    toBottom();
  };

  return (
    <div class={cx('ui-log', baseStyle(props.node).class)} data-mode={props.mode} data-wrap={wrap() ? '' : undefined} style={baseStyle(props.node).style}>
      <div ref={setScrollEl} class="ui-log-scroll" role="log" aria-live="off" tabindex="0" onScroll={onScroll}>
        <VirtualWindow virt={virt} fixed={!wrap()}>
          {(i) => (
            <div class="ui-log-row">
              <Show when={props.node.lineNumbers}>
                <span class="ui-log-no" aria-hidden="true">
                  {first() + i + 1}
                </span>
              </Show>
              <LineView line={lines()[i]} class="ui-log-line" />
            </div>
          )}
        </VirtualWindow>
      </div>
      <Show when={!follow() && lines().length > 0}>
        <button type="button" class="ui-log-live" onClick={resume}>
          <ArrowDown size={14} aria-hidden="true" /> live
        </button>
      </Show>
    </div>
  );
}
