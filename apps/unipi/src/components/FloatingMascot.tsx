/**
 * The floating Unicrab (UNI-200), phone only: an AssistiveTouch-style
 * button that can be dragged anywhere and snaps to the nearer side
 * (position remembered), half-transparent while idle. Tap → the "needs you"
 * drawer; long-press → quick actions. New "needs you" / finished items pop a
 * chat bubble beside it with the summarized reply (the same text ntfy
 * sends), which fades in and out; tapping the bubble opens that chat.
 */
import { createEffect, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { Bell, CheckCheck, EyeOff, MessagesSquare } from 'lucide-solid';
import { Unicrab } from './Unicrab';
import { AnchoredMenu, MenuRow, MenuSeparator } from './board/AnchoredMenu';
import { needsYou, type NeedsYouItem } from '../lib/needsYou';
import { mascotLive } from '../lib/mascotLive';
import { useHosts } from '../hosts/context';
import { openChat } from '../lib/session/open';
import { bareTarget } from '../lib/session/list';
import {
  BUBBLE_MS,
  DRAG_SLOP,
  IDLE_FADE_MS,
  LONG_PRESS_MS,
  bubbleText,
  freshBubbles,
  hostForItem,
  loadPos,
  place,
  savePos,
  snap,
  type Bounds,
  type MascotPos,
} from '../lib/mascot';
import './mascot.css';

const SIZE = 56;

function bounds(): Bounds {
  const css = getComputedStyle(document.documentElement);
  const inset = (v: string) => parseFloat(css.getPropertyValue(v)) || 0;
  const nav = document.querySelector('.bottom-nav')?.getBoundingClientRect().height ?? 0;
  return { width: window.innerWidth, height: window.innerHeight, top: 8 + inset('--inset-top'), bottom: 8 + Math.max(nav, inset('--inset-bottom')), margin: 6 };
}

/** Opens the chat an item points at (or the drawer when it has no reachable pi). */
export function openItem(item: NeedsYouItem, hosts: ReturnType<typeof useHosts>, fallback: () => void): void {
  needsYou().markRead(item.id);
  const host = hostForItem(hosts.hosts(), item);
  if (!item.session || !host) return fallback();
  openChat(
    bareTarget(host.id, item.session, item.title),
    host.name,
  );
}

export function FloatingMascot(props: { onOpenDrawer: () => void; onHide: () => void }) {
  const hosts = useHosts();
  const navigate = useNavigate();
  const live = mascotLive(() => hosts.hosts());
  const store = needsYou();

  const [pos, setPos] = createSignal<MascotPos>(loadPos());
  const [vp, setVp] = createSignal(0);
  const [drag, setDrag] = createSignal<{ left: number; top: number } | null>(null);
  const [menu, setMenu] = createSignal<DOMRect | null>(null);
  const [awake, setAwake] = createSignal(true);
  const [bubble, setBubble] = createSignal<{ item: NeedsYouItem; text: string; key: number } | null>(null);
  let button: HTMLButtonElement | undefined;

  onMount(() => {
    const onResize = () => setVp((n) => n + 1);
    window.addEventListener('resize', onResize);
    window.addEventListener('unipi-ime', onResize);
    onCleanup(() => {
      window.removeEventListener('resize', onResize);
      window.removeEventListener('unipi-ime', onResize);
    });
  });
  const at = () => {
    vp();
    return drag() ?? place(pos(), SIZE, bounds());
  };

  // Idle fade: half-transparent after a while untouched, unless something needs you or a bubble shows.
  let fadeTimer: ReturnType<typeof setTimeout> | undefined;
  const wake = () => {
    setAwake(true);
    clearTimeout(fadeTimer);
    fadeTimer = setTimeout(() => setAwake(false), IDLE_FADE_MS);
  };
  onMount(wake);
  onCleanup(() => clearTimeout(fadeTimer));
  const dim = () => !awake() && !bubble() && !menu() && !drag() && live.state() !== 'needs-you';

  // Bubbles: a queue of fresh items, one at a time.
  let seen: Set<string> | undefined;
  const queue: NeedsYouItem[] = [];
  let bubbleTimer: ReturnType<typeof setTimeout> | undefined;
  let key = 0;
  const showNext = () => {
    const next = queue.shift();
    if (!next) return setBubble(null);
    setBubble({ item: next, text: bubbleText(next), key: ++key });
    wake();
    clearTimeout(bubbleTimer);
    bubbleTimer = setTimeout(showNext, BUBBLE_MS);
  };
  createEffect(() => {
    const items = store.list();
    const fresh = freshBubbles(seen, items, store.isRead);
    seen = new Set(items.map((i) => i.id));
    if (!fresh.length) return;
    queue.push(...fresh.filter((f) => !queue.some((q) => q.id === f.id)));
    if (!bubble()) showNext();
  });
  onCleanup(() => clearTimeout(bubbleTimer));

  // Pointer: drag (past slop), tap, or long-press.
  let start: { x: number; y: number; left: number; top: number; id: number } | null = null;
  let pressTimer: ReturnType<typeof setTimeout> | undefined;
  let longPressed = false;
  const onDown = (e: PointerEvent) => {
    if (e.button !== 0) return;
    const p = at();
    start = { x: e.clientX, y: e.clientY, left: p.left, top: p.top, id: e.pointerId };
    longPressed = false;
    wake();
    button?.setPointerCapture?.(e.pointerId);
    clearTimeout(pressTimer);
    pressTimer = setTimeout(() => {
      if (!start || drag()) return;
      longPressed = true;
      start = null;
      setMenu(button!.getBoundingClientRect());
    }, LONG_PRESS_MS);
  };
  const onMove = (e: PointerEvent) => {
    if (!start || e.pointerId !== start.id) return;
    const dx = e.clientX - start.x;
    const dy = e.clientY - start.y;
    if (!drag() && Math.hypot(dx, dy) < DRAG_SLOP) return;
    clearTimeout(pressTimer);
    const b = bounds();
    setDrag({ left: Math.max(0, Math.min(b.width - SIZE, start.left + dx)), top: Math.max(b.top, Math.min(b.height - b.bottom - SIZE, start.top + dy)) });
  };
  const onUp = (e: PointerEvent) => {
    clearTimeout(pressTimer);
    if (!start || e.pointerId !== start.id) return;
    start = null;
    const d = drag();
    if (d) {
      const next = snap(d.left, d.top, SIZE, bounds());
      setPos(next);
      savePos(next);
      setDrag(null);
      wake();
      return;
    }
    if (!longPressed) {
      live.poke();
      props.onOpenDrawer();
    }
  };
  const onCancel = () => {
    clearTimeout(pressTimer);
    start = null;
    setDrag(null);
  };

  const side = () => (drag() ? (drag()!.left + SIZE / 2 < window.innerWidth / 2 ? 'left' : 'right') : pos().side);
  const openBubble = () => {
    const b = bubble();
    if (!b) return;
    clearTimeout(bubbleTimer);
    setBubble(null);
    queue.length = 0;
    openItem(b.item, hosts, props.onOpenDrawer);
  };

  return (
    <>
      <div
        class="mascot-float"
        data-testid="mascot-float"
        data-side={side()}
        data-dragging={drag() ? '' : undefined}
        data-dim={dim() ? '' : undefined}
        style={{ left: `${at().left}px`, top: `${at().top}px` }}
      >
        <button
          ref={button}
          type="button"
          class="mascot-button"
          aria-label={`Needs you${live.count() ? ` (${live.count()})` : ''}`}
          aria-haspopup="dialog"
          onPointerDown={onDown}
          onPointerMove={onMove}
          onPointerUp={onUp}
          onPointerCancel={onCancel}
          onContextMenu={(e) => {
            e.preventDefault();
            setMenu(button!.getBoundingClientRect());
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              props.onOpenDrawer();
            }
          }}
        >
          <Unicrab state={live.state()} size={44} />
          <Show when={live.count() > 0}>
            <span class="ny-badge mascot-badge">{live.count() > 9 ? '9+' : live.count()}</span>
          </Show>
        </button>
        <For each={bubble() ? [bubble()!] : []}>
          {(b) => (
            <button type="button" class="mascot-bubble" data-group={b.item.group} data-testid="mascot-bubble" onClick={openBubble} style={{ 'animation-duration': `${BUBBLE_MS}ms` }}>
              <span class="mascot-bubble-head">{b.item.title}</span>
              <span class="mascot-bubble-text">{b.text}</span>
            </button>
          )}
        </For>
      </div>
      <Show when={menu()}>
        {(rect) => (
          <AnchoredMenu anchor={rect()} label="Unicrab quick actions" onDismiss={() => setMenu(null)} alignEnd={pos().side === 'right'}>
            <MenuRow icon={<Bell size={18} />} label="Needs you" hint={live.count() ? String(live.count()) : undefined} onSelect={() => (setMenu(null), props.onOpenDrawer())} />
            <MenuRow icon={<MessagesSquare size={18} />} label="Chats" onSelect={() => (setMenu(null), navigate('/chat'))} />
            <MenuRow icon={<CheckCheck size={18} />} label="Mark all read" disabled={!live.count()} onSelect={() => (setMenu(null), store.markAllRead())} />
            <MenuSeparator />
            <MenuRow icon={<EyeOff size={18} />} label="Hide the crab" hint="Settings" onSelect={() => (setMenu(null), props.onHide())} />
          </AnchoredMenu>
        )}
      </Show>
    </>
  );
}
