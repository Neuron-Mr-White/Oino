/**
 * Pointer-event card drag for the desktop board (UNI-218 F24–F29).
 *
 * Replaces HTML5 drag & drop: WebKitGTK (the Linux desktop app) draws its
 * own translucent snapshot as the drag image and stops firing `dragover`
 * while the pointer rests, so the hovered lane lost its ring while the
 * strip autoscrolled (F27). Here the app owns everything:
 *
 *  - press + move ≥ DRAG_THRESHOLD px picks the card up (a plain click
 *    still opens it); a running card refuses with a toast (dragBlocked);
 *  - a ghost (a clone of the card) follows the pointer, lifted with a
 *    small scale/tilt + shadow; the source card stays in place at .35;
 *  - every animation frame: autoscroll the lane strip near its left/right
 *    edge and the hovered column near its top/bottom (time-based, so the
 *    speed is the same at any frame rate), then re-resolve the lane and
 *    insert position under the last pointer position;
 *  - release over a lane → `onDrop(task, lane, before)`; Esc cancels; the
 *    click that follows a drag is swallowed.
 */
import type { Task } from '../../lib/board/api';
import { boardDrag, dragBlocked, snapBefore } from '../../lib/board/dnd';

export const DRAG_THRESHOLD = 5;
/** Autoscroll zone at a scroller's edge (px) and the speed at the very edge (px/s): 22 px/frame at 60 Hz, as the web UI. */
export const SCROLL_EDGE = 80;
export const SCROLL_MAX_SPEED = 1320;

/** Signed autoscroll speed (px/s) for a pointer at `at` in a scroller spanning [start, end]. */
export function edgeSpeed(at: number, start: number, end: number, edge = SCROLL_EDGE, max = SCROLL_MAX_SPEED): number {
  const into = (distance: number) => Math.max(0, Math.min(1, (edge - distance) / edge));
  return (into(end - at) - into(at - start)) * max;
}

export type DropTarget = { lane: string | null; before: string | null };

/**
 * The lane under `node` and the card id a drop at `y` lands before (null =
 * end): the web's midpoint test, snapped to the start of a chain block.
 * A refused lane gives no insert position.
 */
export function dropTargetFrom(node: Element | null, y: number, draggedId: string, x?: number): DropTarget {
  let lane = node?.closest<HTMLElement>('[data-lane-page]');
  // In the strip but between / beside columns (gap, edge padding while it
  // autoscrolls): the nearest column, so its ring never flickers off (F27).
  const strip = !lane && x !== undefined ? node?.closest<HTMLElement>('.lane-pager') : null;
  if (strip) {
    let best = Infinity;
    for (const column of strip.querySelectorAll<HTMLElement>('[data-lane-page]')) {
      const box = column.getBoundingClientRect();
      const distance = x! < box.left ? box.left - x! : x! > box.right ? x! - box.right : 0;
      if (distance < best && distance <= 40) {
        best = distance;
        lane = column;
      }
    }
  }
  if (!lane) return { lane: null, before: null };
  const id = lane.dataset.lanePage ?? null;
  if (lane.dataset.drop === 'refused') return { lane: id, before: null };
  const cards = [...lane.querySelectorAll<HTMLElement>('.lane-cards > .bcard')].filter((card) => card.dataset.id !== draggedId);
  const index = cards.findIndex((card) => {
    const box = card.getBoundingClientRect();
    return y < box.top + box.height / 2;
  });
  const items = cards.map((card) => ({ chain: (card.dataset.chain ?? 'single') as 'single', task: { id: card.dataset.id ?? '' } }));
  return { lane: id, before: snapBefore(items, index) };
}

type Options = {
  task: () => Task;
  /** The horizontal lane scroller (autoscroll). */
  pager: () => HTMLElement | undefined;
  onDrop: (task: Task, lane: string, before: string | null) => void;
  onBlocked: (message: string) => void;
};

let active: { cancel: () => void } | undefined;

/** Lane collapse/expand width transition (board-desktop.css) + slack. */
const PIN_MS = 400;

/**
 * Runs `change` (which adds/removes Backlog and collapses/expands lanes)
 * while keeping `anchor`'s left edge where it was: the web UI pins the
 * source lane the same way, so the board never jumps under the pointer.
 */
export function pinWhile(anchor: HTMLElement | null | undefined, pager: HTMLElement | undefined, change: () => void, dragging = true): void {
  if (!anchor || !pager) return change();
  const left0 = anchor.getBoundingClientRect().left;
  // While dragging, the strip may get shorter (lanes collapse) or longer
  // (Backlog appears in front): give it room at the end so the scroll can
  // always keep the source lane in place. Dropped when the drag ends.
  if (!dragging) pager.style.removeProperty('--drag-room');
  change();
  const fix = () => {
    const delta = anchor.getBoundingClientRect().left - left0;
    if (Math.abs(delta) <= 0.5) return;
    if (dragging) {
      const short = pager.scrollLeft + delta - (pager.scrollWidth - pager.clientWidth);
      if (short > 0) pager.style.setProperty('--drag-room', `${Math.ceil(short + (parseFloat(pager.style.getPropertyValue('--drag-room')) || 0))}px`);
    }
    pager.scrollLeft += delta;
  };
  fix();
  const stopAt = performance.now() + PIN_MS;
  const step = () => {
    if (!anchor.isConnected) return;
    fix();
    if (performance.now() < stopAt) requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
}

/** Call from the card's pointerdown. Does nothing until the pointer moves DRAG_THRESHOLD px. */
export function pressCard(event: PointerEvent, card: HTMLElement, options: Options): void {
  if ((event.button ?? 0) !== 0 || event.ctrlKey || event.metaKey || event.shiftKey) return;
  if ((event.target as HTMLElement | null)?.closest?.('button, a, input, textarea, select')) return;
  active?.cancel();
  const dnd = boardDrag();
  const start = { x: event.clientX, y: event.clientY };
  let at = { ...start };
  let lastTarget: Element | null = null;
  let started = false;
  let snapshot: Task | undefined;
  let ghost: HTMLElement | undefined;
  let offset = { x: 0, y: 0 };
  let frame = 0;
  let lastTime = 0;
  let sourceLane: HTMLElement | null = null;

  const placeGhost = () => {
    if (ghost) ghost.style.transform = `translate3d(${at.x - offset.x}px, ${at.y - offset.y}px, 0)`;
  };
  const resolve = (fromPoint: boolean) => {
    if (!snapshot) return;
    const node = fromPoint && typeof document.elementFromPoint === 'function' ? document.elementFromPoint(at.x, at.y) : lastTarget;
    const target = dropTargetFrom(node, at.y, snapshot.id, at.x);
    dnd.over(target.lane, target.before);
  };
  const begin = () => {
    const blocked = dragBlocked(options.task());
    if (blocked) {
      options.onBlocked(blocked);
      stop();
      return;
    }
    started = true;
    // Snapshot: store proxies reflect later writes.
    snapshot = JSON.parse(JSON.stringify(options.task())) as Task;
    const box = card.getBoundingClientRect();
    offset = { x: start.x - box.left, y: start.y - box.top };
    const wrap = document.createElement('div');
    wrap.className = 'bcard-ghost';
    wrap.setAttribute('aria-hidden', 'true');
    wrap.style.width = `${box.width}px`;
    const clone = card.cloneNode(true) as HTMLElement;
    clone.removeAttribute('data-id');
    clone.removeAttribute('tabindex');
    clone.removeAttribute('role');
    clone.classList.remove('dragging', 'kb-selected', 'flash');
    wrap.append(clone);
    document.body.append(wrap);
    ghost = wrap;
    placeGhost();
    requestAnimationFrame(() => wrap.setAttribute('data-lifted', ''));
    document.documentElement.setAttribute('data-card-drag', '');
    sourceLane = card.closest<HTMLElement>('.lane-page');
    pinWhile(sourceLane, options.pager(), () => dnd.start(snapshot!));
    resolve(true);
    lastTime = performance.now();
    frame = requestAnimationFrame(tick);
  };
  const tick = (now: number) => {
    const dt = Math.min(64, now - lastTime) / 1000;
    lastTime = now;
    const pager = options.pager();
    if (pager) {
      const box = pager.getBoundingClientRect();
      const vx = edgeSpeed(at.x, box.left, box.right);
      if (vx) pager.scrollLeft += vx * dt;
    }
    const node = typeof document.elementFromPoint === 'function' ? document.elementFromPoint(at.x, at.y) : null;
    const column = node?.closest<HTMLElement>('.lane-page');
    if (column && column.scrollHeight > column.clientHeight) {
      const box = column.getBoundingClientRect();
      const vy = edgeSpeed(at.y, box.top, box.bottom);
      if (vy) column.scrollTop += vy * dt;
    }
    // The content may have scrolled under a resting pointer: re-resolve (F27).
    resolve(true);
    frame = requestAnimationFrame(tick);
  };
  const move = (moveEvent: PointerEvent) => {
    at = { x: moveEvent.clientX, y: moveEvent.clientY };
    lastTarget = moveEvent.target as Element | null;
    if (!started) {
      if (Math.hypot(at.x - start.x, at.y - start.y) < DRAG_THRESHOLD) return;
      begin();
      if (!started) return;
    }
    moveEvent.preventDefault();
    placeGhost();
    resolve(false);
  };
  const up = (upEvent: PointerEvent) => {
    if (!started) return stop();
    at = { x: upEvent.clientX, y: upEvent.clientY };
    lastTarget = upEvent.target as Element | null;
    resolve(typeof document.elementFromPoint === 'function');
    const current = dnd.drag();
    const task = snapshot!;
    swallowClick();
    stop();
    if (current?.lane) options.onDrop(task, current.lane, current.before);
  };
  const key = (keyEvent: KeyboardEvent) => {
    if (keyEvent.key !== 'Escape') return;
    keyEvent.preventDefault();
    keyEvent.stopPropagation();
    if (started) swallowClick();
    stop();
  };
  function stop() {
    window.removeEventListener('pointermove', move);
    window.removeEventListener('pointerup', up);
    window.removeEventListener('pointercancel', cancel);
    window.removeEventListener('keydown', key, true);
    window.removeEventListener('blur', cancel);
    cancelAnimationFrame(frame);
    ghost?.remove();
    ghost = undefined;
    document.documentElement.removeAttribute('data-card-drag');
    if (started) pinWhile(sourceLane, options.pager(), () => dnd.end(), false);
    started = false;
    if (active === handle) active = undefined;
  }
  function cancel() {
    stop();
  }
  const handle = { cancel };
  active = handle;
  window.addEventListener('pointermove', move);
  window.addEventListener('pointerup', up);
  window.addEventListener('pointercancel', cancel);
  window.addEventListener('keydown', key, true);
  window.addEventListener('blur', cancel);
}

/** The click that ends a drag must not open the card (it fires right after pointerup, or not at all). */
function swallowClick() {
  const until = performance.now() + 250;
  const eat = (event: MouseEvent) => {
    window.removeEventListener('click', eat, true);
    // Only the board columns: a toast's Undo right after a drop still works.
    if (performance.now() > until || !(event.target as Element | null)?.closest?.('.lane-pager')) return;
    event.stopPropagation();
    event.preventDefault();
  };
  window.addEventListener('click', eat, true);
  setTimeout(() => window.removeEventListener('click', eat, true), 250);
}

/** Test hook: drop any drag in flight. */
export function cancelCardDrag(): void {
  active?.cancel();
}
