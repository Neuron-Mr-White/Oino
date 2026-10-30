/**
 * Query navigator rail (app spec WP-A7, Z-25; ZCode
 * `conversation-turn-navigator`): one compact bar per user query on the left
 * of the timeline, shown only when the conversation column is ≥ 1100 px
 * (§3.9) and there are at least two queries. Hovering a bar shows the query
 * plus its turn's answer excerpt; clicking jumps that turn to the top and
 * leaves the timeline detached (the Timeline owns the virtualizer and the
 * bottom lock).
 *
 * It is an index, not a scroll minimap: the bar position is the query order,
 * evenly stacked. The rail is its own small scroller with a fixed row height
 * and a windowed render, so a 2 000-turn session keeps ~60 buttons mounted
 * (§3.6 DOM budget).
 */
import { createEffect, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { textOf, type Part } from '../../lib/chat/model';
import type { Turn } from '../../lib/session/turns';
import { excerpt } from '../../lib/timeline/actions';

/** The conversation must be this wide before the rail is worth its margin. */
export const NAV_MIN_WIDTH = 1100;
/** Bar row pitch (6 px bar + 3 px gap). */
const ROW = 9;
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
  const items = () => navigatorItems(props.turns());
  const shown = () => props.width() >= NAV_MIN_WIDTH && items().length >= 2;
  const [el, setEl] = createSignal<HTMLDivElement>();
  const [top, setTop] = createSignal(0);
  const [height, setHeight] = createSignal(420);

  // Keep the rail at the tail while the reader is at the tail; a reader who
  // scrolled the rail keeps their place.
  let initialized = false;
  createEffect(() => {
    const rail = el();
    const count = items().length;
    if (!rail || !count) return;
    const atBottom = rail.scrollTop + rail.clientHeight >= rail.scrollHeight - ROW * 2;
    if (!initialized || atBottom) rail.scrollTop = rail.scrollHeight;
    initialized = true;
  });

  onMount(() => {
    const rail = el();
    if (!rail) return;
    setHeight(rail.clientHeight || 420);
    setTop(rail.scrollTop);
    const onScroll = () => setTop(rail.scrollTop);
    rail.addEventListener('scroll', onScroll, { passive: true });
    const observer =
      typeof ResizeObserver === 'undefined'
        ? undefined
        : new ResizeObserver(() => {
            setHeight(rail.clientHeight || 420);
            setTop(rail.scrollTop);
          });
    observer?.observe(rail);
    onCleanup(() => {
      rail.removeEventListener('scroll', onScroll);
      observer?.disconnect();
    });
  });

  const first = () => Math.max(0, Math.floor(top() / ROW) - OVERSCAN);
  const last = () => Math.min(items().length, Math.ceil((top() + height()) / ROW) + OVERSCAN);

  return (
    <Show when={shown()}>
      <nav class="tl-nav" data-testid="tl-nav" aria-label="Jump to a message" ref={setEl}>
        <div class="tl-nav-track" style={{ height: `${items().length * ROW}px` }}>
          <For each={items().slice(first(), last())}>
            {(item, i) => (
              <button
                type="button"
                class="tl-nav-bar"
                data-testid="tl-nav-bar"
                data-turn={item.turnId}
                data-active={props.activeId?.() === item.turnId ? '' : undefined}
                data-live={item.live ? '' : undefined}
                style={{ top: `${(first() + i()) * ROW}px` }}
                title={`${item.query}\n— ${item.excerpt}`}
                aria-label={`Jump to: ${item.query}`}
                onClick={() => props.onJump(item.turnId)}
              />
            )}
          </For>
        </div>
      </nav>
    </Show>
  );
}
