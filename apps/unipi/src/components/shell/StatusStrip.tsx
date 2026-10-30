/**
 * Status strip (app spec §3.10, WP-A3 builds the row; U10 feeds it): one
 * 24 px row at the bottom of the conversation frame (under the composer) with
 * the extensions' `setStatus` / `setFooter` segments. Segments beyond
 * `INLINE_MAX` go to an overflow popover. Not in the capsule. Renders
 * nothing while there are no segments (no empty chrome).
 *
 *   setStatusSegment(`${hostId}/${sessionId}`, 'ext:git', { text: 'main ↑1' });
 */
import { createSignal, For, Show } from 'solid-js';
import { AnchoredMenu } from '../board/AnchoredMenu';
import { layout } from '../../lib/layout';

export type StatusSegment = { text: string; title?: string; tone?: 'info' | 'warn' | 'error' | 'ok' };

/** Desktop shows up to 5 inline; on the phone (under the header, §3.10) up to 4. */
export const INLINE_MAX = 5;
export const INLINE_MAX_PHONE = 4;

const [segments, setSegments] = createSignal<Record<string, Record<string, StatusSegment>>>({});

/** Sets (or with `undefined` clears) one segment of a scope (`hostId/sessionId`). */
export function setStatusSegment(scope: string, key: string, segment: StatusSegment | undefined): void {
  setSegments((cur) => {
    const mine = { ...(cur[scope] ?? {}) };
    if (segment) mine[key] = segment;
    else delete mine[key];
    return { ...cur, [scope]: mine };
  });
}

/** Clears every segment of a scope (session closed / extension reloaded). */
export function clearStatusScope(scope: string): void {
  setSegments((cur) => {
    if (!cur[scope]) return cur;
    const next = { ...cur };
    delete next[scope];
    return next;
  });
}

export function statusSegments(scope: string): Array<StatusSegment & { key: string }> {
  return Object.entries(segments()[scope] ?? {}).map(([key, s]) => ({ key, ...s }));
}

export function StatusStrip(props: { scope: string }) {
  const max = () => (layout().desktop() ? INLINE_MAX : INLINE_MAX_PHONE);
  const all = () => statusSegments(props.scope);
  const inline = () => all().slice(0, max());
  const rest = () => all().slice(max());
  const [anchor, setAnchor] = createSignal<DOMRect | undefined>();
  return (
    <Show when={all().length > 0}>
      <div class="status-strip" role="status" aria-label="Status" data-testid="status-strip">
        <For each={inline()}>
          {(s) => (
            <span class="status-seg" data-tone={s.tone} title={s.title ?? s.text}>
              {s.text}
            </span>
          )}
        </For>
        <Show when={rest().length > 0}>
          <button type="button" class="status-more" aria-label={`${rest().length} more`} onClick={(e) => setAnchor(e.currentTarget.getBoundingClientRect())}>
            +{rest().length}
          </button>
        </Show>
        <Show when={anchor()}>
          {(a) => (
            <AnchoredMenu anchor={a()} alignEnd width={280} label="More status" onDismiss={() => setAnchor(undefined)}>
              <For each={rest()}>{(s) => <div class="status-pop-row" data-tone={s.tone}>{s.text}</div>}</For>
            </AnchoredMenu>
          )}
        </Show>
      </div>
    </Show>
  );
}
