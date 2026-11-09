/**
 * Review-queue UI state that must outlive the pane (OIN-40). On the phone the
 * pane lives in a bottom sheet that closes while a ticket is open and mounts
 * afresh on return, so the "Review" accordion, its "Show more" and the scroll
 * offset are kept here instead of in the component.
 */
import { createSignal } from 'solid-js';

export const [reviewOpen, setReviewOpen] = createSignal(false);
export const [reviewAll, setReviewAll] = createSignal(false);

let scroll = 0;
const SCROLLER = '.sa-side, .ph-sheet-body';
const scroller = (): HTMLElement | null => document.querySelector('.sa-pane')?.closest<HTMLElement>(SCROLLER) ?? null;

/** Remembers the pane's scroll offset (call just before a ticket opens over it). */
export function savePaneScroll(): void {
  scroll = scroller()?.scrollTop ?? 0;
}

/** Puts it back once the pane is mounted (the phone sheet animates in: retry a few frames). */
export function restorePaneScroll(): void {
  const target = scroll;
  if (!target) return;
  let tries = 0;
  const tick = () => {
    const el = scroller();
    if (el && el.scrollHeight - el.clientHeight >= target - 1) { el.scrollTop = target; return; }
    if (++tries < 30) requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}
