/**
 * Delegated tap / long-press / right-click handling for `[data-file-ref]`
 * elements inside a container (UNI-204): MdBlock's rendered markdown, user
 * bubbles and tool output all attach this once to their root.
 */
import type { RefChat } from './pathStat';
import { refFromElement } from './fileRefs';
import { openFileRef, openFileRefMenu } from './fileRefOpen';

const LONG_PRESS_MS = 500;
const MOVE_SLOP = 10;

export function refElementFrom(target: EventTarget | null, root: HTMLElement): HTMLElement | null {
  const el = (target as HTMLElement | null)?.closest?.<HTMLElement>('[data-file-ref]') ?? null;
  return el && root.contains(el) ? el : null;
}

/** Marks a failed open on the element for a moment (CSS shows it red). */
function flashError(el: HTMLElement): void {
  el.dataset.status = 'error';
  setTimeout(() => delete el.dataset.status, 2000);
}

/** Opens the ref under `event` if any; true when it handled the click. */
export function handleFileRefClick(event: MouseEvent, root: HTMLElement, chat: RefChat | undefined): boolean {
  const el = refElementFrom(event.target, root);
  if (!el || !chat) return false;
  event.preventDefault();
  event.stopPropagation();
  const ref = refFromElement(el);
  if (ref) void openFileRef(chat, ref).catch(() => flashError(el));
  return true;
}

/** Android WebView starts its own word selection (highlight + handles + action bar) on the same
 * long-press that opens our menu; drop it now and once more when the WebView's own timer fires. */
function dropNativeSelection(): void {
  const drop = () => {
    const sel = document.getSelection();
    if (sel && !sel.isCollapsed) sel.removeAllRanges();
  };
  drop();
  setTimeout(drop, 120);
  setTimeout(drop, 400);
}

/** Wires long-press (touch) and right-click to the file-ref menu. Returns a cleanup. Clicks that end a
 * long-press are swallowed so the menu's ref doesn't also open. */
export function attachFileRefGestures(root: HTMLElement, chat: () => RefChat | undefined): () => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let startX = 0;
  let startY = 0;
  let pressedAt = 0;
  const clear = () => {
    clearTimeout(timer);
    timer = undefined;
  };
  const menuFor = (el: HTMLElement) => {
    const c = chat();
    const ref = refFromElement(el);
    if (!c || !ref) return false;
    openFileRefMenu(c, ref, el);
    pressedAt = Date.now();
    dropNativeSelection();
    return true;
  };
  const onTouchStart = (e: TouchEvent) => {
    const el = refElementFrom(e.target, root);
    if (!el || e.touches.length !== 1) return;
    startX = e.touches[0]!.clientX;
    startY = e.touches[0]!.clientY;
    clear();
    timer = setTimeout(() => {
      timer = undefined;
      menuFor(el);
    }, LONG_PRESS_MS);
  };
  const onTouchMove = (e: TouchEvent) => {
    if (!timer) return;
    const t = e.touches[0];
    if (!t || Math.abs(t.clientX - startX) > MOVE_SLOP || Math.abs(t.clientY - startY) > MOVE_SLOP) clear();
  };
  const onContextMenu = (e: MouseEvent) => {
    const el = refElementFrom(e.target, root);
    if (!el) return;
    e.preventDefault();
    e.stopPropagation();
    clear();
    if (Date.now() - pressedAt < 800) {
      dropNativeSelection(); // the long-press timer already opened it
      return;
    }
    menuFor(el);
  };
  const onClickCapture = (e: MouseEvent) => {
    if (Date.now() - pressedAt < 800 && refElementFrom(e.target, root)) {
      e.preventDefault();
      e.stopPropagation();
    }
  };
  root.addEventListener('touchstart', onTouchStart, { passive: true });
  root.addEventListener('touchmove', onTouchMove, { passive: true });
  root.addEventListener('touchend', clear);
  root.addEventListener('touchcancel', clear);
  root.addEventListener('contextmenu', onContextMenu);
  root.addEventListener('click', onClickCapture, true);
  return () => {
    clear();
    root.removeEventListener('touchstart', onTouchStart);
    root.removeEventListener('touchmove', onTouchMove);
    root.removeEventListener('touchend', clear);
    root.removeEventListener('touchcancel', clear);
    root.removeEventListener('contextmenu', onContextMenu);
    root.removeEventListener('click', onClickCapture, true);
  };
}
