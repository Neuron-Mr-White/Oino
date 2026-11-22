/**
 * Inner bottom-follow (chat-scroll, owner 2026-11-03): an expanded accordion
 * in the live tail (tool output, "Worked for…" fold, subagent card body,
 * card body) is capped by `max-height` and scrolls on its own, so new
 * output growing inside it no longer grows the timeline — the outer bottom
 * lock cannot see it. Each such box follows its own bottom like a terminal
 * while it was at its bottom before the growth; a box the reader scrolled
 * up inside keeps its position, and a box first seen (just expanded,
 * mounted) only records its height (a finished `read` opens at its top).
 *
 * Bounded: one `WeakMap` of heights keyed by the boxes themselves (gone
 * with their nodes), one sweep per animation frame at most.
 */

/** The capped, self-scrolling bodies a timeline row can hold. */
export const INNER_SCROLLERS = [
  '.proc-body',
  '.tool-term-out',
  '.tool-read',
  '.tool-io pre',
  '.tool-diff',
  '.tool-write-md',
  '.tl-agent-body',
  '.ucard-body',
  '.tl-report-body',
].join(', ');

/** Slack (px) under which a box still counts as "at its bottom". */
const AT_BOTTOM_PX = 8;

export type InnerBox = {
  scrollTop: number;
  readonly scrollHeight: number;
  readonly clientHeight: number;
};

export type InnerFollow = {
  /** One pass over `boxes`: follows the ones that were at their bottom; true when any grew. */
  sweep(boxes: Iterable<InnerBox>): boolean;
};

export function createInnerFollow(): InnerFollow {
  const heights = new WeakMap<InnerBox, number>();
  return {
    sweep(boxes) {
      let grew = false;
      for (const box of boxes) {
        const h = box.scrollHeight;
        const prev = heights.get(box);
        heights.set(box, h);
        if (prev === undefined || h <= prev + 1) continue;
        grew = true;
        // At its bottom relative to the content it had before this growth.
        if (box.scrollTop + box.clientHeight >= prev - AT_BOTTOM_PX) box.scrollTop = h - box.clientHeight;
      }
      return grew;
    },
  };
}
