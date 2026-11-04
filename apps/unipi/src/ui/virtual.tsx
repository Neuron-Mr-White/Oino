/**
 * Virtual rows (ui-protocol §13.4) on `@tanstack/solid-virtual`, shared by
 * List, Table, Log and Tree. Callers keep the scroll element; rows are
 * absolutely positioned inside a spacer of the total height and measured
 * (`measureElement`) when their height varies.
 */
import { For, createEffect, on, type Accessor, type JSX } from 'solid-js';
import { createVirtualizer, observeElementRect, type Virtualizer } from '@tanstack/solid-virtual';

/** Rows above which List / Table / Tree virtualize (§13.4). */
export const VIRTUAL_THRESHOLD = 100;

export type VirtualOptions = {
  count: Accessor<number>;
  scroller: Accessor<HTMLElement | undefined>;
  estimate: (index: number) => number;
  key: (index: number) => string | number;
  overscan?: number;
  /** Rows have a fixed height: skip measuring. */
  fixed?: boolean;
};

export function useVirtual(opts: VirtualOptions): Virtualizer<HTMLElement, Element> {
  const virt = createVirtualizer<HTMLElement, Element>({
    get count() {
      return opts.count();
    },
    get getScrollElement() {
      const el = opts.scroller();
      return () => el ?? null;
    },
    // An unlaid-out scroller (0 height: jsdom, display:none) keeps the initial window.
    observeElementRect: (inst, cb) => observeElementRect(inst, (rect) => (rect.height > 0 ? cb(rect) : undefined)),
    estimateSize: opts.estimate,
    getItemKey: opts.key,
    overscan: opts.overscan ?? 8,
    // Render a first window before the scroller is measured (first paint, jsdom).
    initialRect: { width: 0, height: 480 },
  });
  // A late-mounted scroller (ref set on a node not yet inserted, e.g. inside a
  // conditional) is detached when the options computed first sees it, so
  // virtual-core records it without observers (no defaultView). Re-attach once
  // effects run, when the element is connected.
  createEffect(
    on(opts.scroller, (el) => {
      if (!el) return;
      const attach = () => {
        // virtual-core already recorded the detached element (no observers); drop it so
        // _willUpdate sees a new scroll element and observes rect + offset for real.
        (virt as unknown as { cleanup: () => void }).cleanup();
        virt._willUpdate();
        virt.measure();
      };
      if (el.isConnected) attach();
      else queueMicrotask(attach);
    }),
  );
  return virt;
}

/** Renders the virtual window: a full-height spacer with the visible rows translated into place. */
export function VirtualWindow(props: {
  virt: Virtualizer<HTMLElement, Element>;
  fixed?: boolean;
  children: (index: number) => JSX.Element;
  class?: string;
}) {
  return (
    <div class={props.class ?? 'ui-virtual'} style={{ height: `${props.virt.getTotalSize()}px`, position: 'relative' }}>
      <For each={props.virt.getVirtualItems()}>
        {(item) => (
          <div
            class="ui-virtual-row"
            data-index={item.index}
            ref={(el) => (props.fixed ? undefined : queueMicrotask(() => props.virt.measureElement(el)))}
            style={{ position: 'absolute', top: 0, left: 0, right: 0, transform: `translateY(${item.start}px)` }}
          >
            {props.children(item.index)}
          </div>
        )}
      </For>
    </div>
  );
}
