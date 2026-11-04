/**
 * Turn rail (Higgsfield-style): a thin vertical rail of short horizontal tick
 * bars at the chat's left edge, one per user message. The pointer fisheyes the
 * ticks (smooth falloff over ~5 ticks, transform only); the turn in view is
 * accent-coloured and a little longer. Hovering or focusing the rail opens a
 * floating list of every message as one truncated line, in sync with the tick
 * under the pointer. A click on a tick or row calls `onJump`, which the
 * Timeline routes through its single scroll writer (this file never writes
 * the timeline's scrollTop).
 *
 * Many turns compress: ticks are bucketed so the rail never needs more than
 * one tick per ~3 px, and the list is windowed (fixed row height).
 */
import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { textOf, type Part } from '../../lib/chat/model';
import type { Turn } from '../../lib/session/turns';
import { excerpt } from '../../lib/timeline/actions';

/** Hidden below this conversation width (phone); a side pane open next to the chat still leaves the rail. */
export const NAV_MIN_WIDTH = 560;
/** Preferred tick pitch and the densest pitch before ticks are bucketed. */
const PITCH = 10;
const MIN_PITCH = 3;
/** Fisheye reach in ticks and peak extra scale. */
const REACH = 5;
const GAIN = 2.4;
const ACTIVE_SCALE = 1.5;
const ROW = 28;
const OVERSCAN = 4;

export type NavItem = { turnId: string; query: string; excerpt: string; live: boolean };

const queryOf = (turn: Turn): string => textOf(turn.user?.message?.content as string | Part[] | undefined).trim();

/** One item per user query of a turn (hidden command echoes excluded). */
export function navigatorItems(turns: readonly Turn[]): NavItem[] {
  const items: NavItem[] = [];
  for (const turn of turns) {
    if (turn.kind !== 'turn' || turn.command?.hidden) continue;
    const query = queryOf(turn);
    if (!query) continue;
    let answer = '';
    for (let i = turn.segments.length - 1; i >= 0; i--) {
      const text = turn.segments[i]!.answer?.text.trim();
      if (text) {
        answer = text;
        break;
      }
    }
    items.push({
      turnId: turn.turnId,
      query: excerpt(query, 160),
      excerpt: answer ? excerpt(answer) : turn.live ? 'Working…' : 'No answer yet',
      live: turn.live,
    });
  }
  return items;
}

export function TurnNavigator(props: {
  turns: () => Turn[];
  width: () => number;
  activeId?: () => string | undefined;
  onJump: (turnId: string) => void;
}) {
  const items = createMemo(() => navigatorItems(props.turns()));
  const shown = () => props.width() >= NAV_MIN_WIDTH && items().length >= 2;
  const [nav, setNav] = createSignal<HTMLElement>();
  const [maxH, setMaxH] = createSignal(420);
  const [open, setOpen] = createSignal(false);
  /** Item index under the pointer / keyboard focus (-1 = none). */
  const [hover, setHover] = createSignal(-1);
  const [listTop, setListTop] = createSignal(0);
  const [listH, setListH] = createSignal(300);
  let list: HTMLDivElement | undefined;
  let closeTimer: ReturnType<typeof setTimeout> | undefined;

  const count = () => items().length;
  /** Ticks shown: one per item until the rail is denser than MIN_PITCH. */
  const ticks = createMemo(() => Math.max(1, Math.min(count(), Math.floor(maxH() / MIN_PITCH))));
  const bucket = () => count() / ticks();
  const pitch = () => Math.min(PITCH, maxH() / ticks());
  const railH = () => Math.round(ticks() * pitch());
  const tickOf = (itemIdx: number) => Math.min(ticks() - 1, Math.floor(itemIdx / bucket()));
  const activeIdx = () => {
    const id = props.activeId?.();
    return id ? items().findIndex((i) => i.turnId === id) : -1;
  };
  const activeTick = () => (activeIdx() < 0 ? -1 : tickOf(activeIdx()));
  const jumpTick = (k: number) => {
    const item = items()[Math.min(count() - 1, Math.floor(k * bucket()))];
    if (item) props.onJump(item.turnId);
  };

  onMount(() => {
    const rail = nav();
    const parent = rail?.offsetParent as HTMLElement | null;
    const measure = () => setMaxH(Math.max(60, Math.min(520, (parent?.clientHeight || 600) * 0.7)));
    measure();
    const observer = typeof ResizeObserver === 'undefined' || !parent ? undefined : new ResizeObserver(measure);
    if (parent) observer?.observe(parent);
    onCleanup(() => {
      observer?.disconnect();
      clearTimeout(closeTimer);
      cancelAnimationFrame(frame);
    });
  });

  // ── fisheye: write --s on the ticks directly, one rAF per pointer burst ──
  let frame = 0;
  let focusTick = -1; // fractional tick position of the pointer, -1 = none
  const paint = () => {
    frame = 0;
    const rail = nav();
    if (!rail) return;
    const act = activeTick();
    rail.querySelectorAll<HTMLElement>('.tl-nav-tick').forEach((tick, k) => {
      let s = k === act ? ACTIVE_SCALE : 1;
      if (focusTick >= 0) {
        const d = Math.abs(k + 0.5 - focusTick) / REACH;
        if (d < 1) s += GAIN * Math.cos((d * Math.PI) / 2) ** 2;
      }
      tick.style.setProperty('--s', s.toFixed(3));
    });
  };
  const schedule = () => {
    if (!frame) frame = requestAnimationFrame(paint);
  };
  createEffect(() => {
    activeTick();
    ticks();
    shown();
    schedule();
  });

  // ── list window + sync ────────────────────────────────────────────
  const first = () => Math.max(0, Math.floor(listTop() / ROW) - OVERSCAN);
  const last = () => Math.min(count(), Math.ceil((listTop() + listH()) / ROW) + OVERSCAN);
  const reveal = (idx: number) => {
    if (!list || idx < 0) return;
    const top = idx * ROW;
    const h = list.clientHeight || listH();
    if (top < list.scrollTop) list.scrollTop = top;
    else if (top + ROW > list.scrollTop + h) list.scrollTop = top + ROW - h;
    setListTop(list.scrollTop);
  };
  createEffect(() => {
    if (!open()) return;
    const idx = hover() >= 0 ? hover() : activeIdx();
    queueMicrotask(() => reveal(idx));
  });

  const openNow = () => {
    clearTimeout(closeTimer);
    setOpen(true);
  };
  const closeSoon = () => {
    clearTimeout(closeTimer);
    closeTimer = setTimeout(() => {
      setOpen(false);
      setHover(-1);
      focusTick = -1;
      schedule();
    }, 160);
  };
  const onRailMove = (e: PointerEvent) => {
    const rail = (e.currentTarget as HTMLElement).querySelector<HTMLElement>('.tl-nav-ticks');
    if (!rail) return;
    const y = e.clientY - rail.getBoundingClientRect().top;
    focusTick = Math.max(0, Math.min(ticks(), y / pitch()));
    setHover(Math.min(count() - 1, Math.floor(Math.min(ticks() - 1, focusTick) * bucket())));
    openNow();
    schedule();
  };
  const onTickKey = (e: KeyboardEvent, k: number) => {
    const step = e.key === 'ArrowDown' ? 1 : e.key === 'ArrowUp' ? -1 : e.key === 'Home' ? -k : e.key === 'End' ? ticks() : 0;
    if (!step) return;
    e.preventDefault();
    const next = Math.max(0, Math.min(ticks() - 1, k + step));
    (nav()?.querySelectorAll<HTMLElement>('.tl-nav-tick')[next])?.focus();
  };
  const onTickFocus = (k: number) => {
    focusTick = k + 0.5;
    setHover(Math.min(count() - 1, Math.floor(k * bucket())));
    openNow();
    schedule();
  };

  return (
    <Show when={shown()}>
      <nav
        class="tl-nav"
        data-testid="tl-nav"
        data-open={open() ? '' : undefined}
        aria-label="Jump to a message"
        ref={setNav}
        onPointerMove={onRailMove}
        onPointerLeave={closeSoon}
        onPointerEnter={openNow}
        onFocusOut={(e) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node | null)) closeSoon();
        }}
      >
        <div class="tl-nav-ticks" style={{ height: `${railH()}px` }}>
          <For each={Array.from({ length: ticks() }, (_, k) => k)}>
            {(k) => {
              const item = () => items()[Math.min(count() - 1, Math.floor(k * bucket()))]!;
              return (
                <button
                  type="button"
                  class="tl-nav-tick"
                  data-testid="tl-nav-bar"
                  data-nav-turn={item().turnId}
                  data-active={activeTick() === k ? '' : undefined}
                  data-hot={hover() >= 0 && tickOf(hover()) === k ? '' : undefined}
                  data-live={item().live ? '' : undefined}
                  style={{ top: `${k * pitch()}px`, height: `${pitch()}px` }}
                  tabIndex={k === Math.max(0, activeTick()) ? 0 : -1}
                  aria-label={`Jump to: ${item().query}`}
                  aria-current={activeTick() === k ? 'true' : undefined}
                  onClick={() => jumpTick(k)}
                  onFocus={() => onTickFocus(k)}
                  onKeyDown={(e) => onTickKey(e, k)}
                />
              );
            }}
          </For>
        </div>
        <Show when={open()}>
          <div
            class="tl-nav-panel"
            data-testid="tl-nav-panel"
            style={{ 'max-height': `${Math.min(count() * ROW, Math.max(maxH(), 160)) + 12}px` }}
            onPointerEnter={openNow}
          >
            <div
              class="tl-nav-list"
              ref={(el) => {
                list = el;
                setListH(el.clientHeight || 300);
              }}
              onScroll={(e) => setListTop(e.currentTarget.scrollTop)}
              onPointerMove={(e) => e.stopPropagation()}
            >
              <div style={{ height: `${count() * ROW}px`, position: 'relative' }}>
                <For each={items().slice(first(), last())}>
                  {(item, i) => (
                    <button
                      type="button"
                      class="tl-nav-row"
                      data-testid="tl-nav-row"
                      data-nav-turn={item.turnId}
                      data-active={props.activeId?.() === item.turnId ? '' : undefined}
                      data-hot={hover() === first() + i() ? '' : undefined}
                      style={{ top: `${(first() + i()) * ROW}px`, height: `${ROW}px` }}
                      tabIndex={-1}
                      title={item.query}
                      onPointerEnter={() => {
                        setHover(first() + i());
                        focusTick = tickOf(first() + i()) + 0.5;
                        schedule();
                      }}
                      onClick={() => props.onJump(item.turnId)}
                    >
                      {item.query.replace(/\s+/g, ' ')}
                    </button>
                  )}
                </For>
              </div>
            </div>
          </div>
        </Show>
      </nav>
    </Show>
  );
}
