/**
 * Status strip (app spec §3.10, WP-A3 builds the row; U10 feeds it): one
 * 24 px row at the bottom of the conversation frame (under the composer) with
 * the extensions' `setStatus` / `setFooter` segments. Segments beyond
 * `INLINE_MAX` go to an overflow popover. Not in the capsule. Renders
 * nothing while there are no segments (no empty chrome).
 *
 *   setStatusSegment(`${hostId}/${sessionId}`, 'ext:git', { text: 'main ↑1' });
 *   setStatusSegments(`${hostId}/${sessionId}`, feed.list());   // U10, ordered
 *
 * U10's feed replaces the scope wholesale in registry order (`order` desc,
 * `seq` asc, ui-protocol §6.4) every registry change; a `footer` entry
 * contributes a segment that renders its node tree in auto mode, one row
 * (rule 4.2.2 — no second footer surface).
 */
import { createSignal, For, Show } from 'solid-js';
import { AnchoredMenu } from '../board/AnchoredMenu';
import { layout } from '../../lib/layout';
import { NodeView } from '../../ui/NodeView';
import type { UiNode } from '../../ui/types';

export type StatusSegment = { text: string; title?: string; tone?: 'info' | 'warn' | 'error' | 'ok'; node?: UiNode };

/** Desktop shows up to 5 inline; on the phone (under the header, §3.10) up to 4. */
export const INLINE_MAX = 5;
export const INLINE_MAX_PHONE = 4;

type ScopeSegments = { order: string[]; by: Record<string, StatusSegment> };

const [segments, setSegments] = createSignal<Record<string, ScopeSegments>>({});

function upsert(scope: ScopeSegments | undefined, key: string, segment: StatusSegment | undefined): ScopeSegments {
  const by = { ...(scope?.by ?? {}) };
  const order = [...(scope?.order ?? [])];
  if (segment) {
    if (!by[key]) order.push(key);
    by[key] = segment;
  } else if (by[key]) {
    delete by[key];
    order.splice(order.indexOf(key), 1);
  }
  return { order, by };
}

/** Sets (or with `undefined` clears) one segment of a scope (`hostId/sessionId`). */
export function setStatusSegment(scope: string, key: string, segment: StatusSegment | undefined): void {
  setSegments((cur) => {
    const next = upsert(cur[scope], key, segment);
    return next.order.length || cur[scope] ? { ...cur, [scope]: next } : cur;
  });
}

/** Replaces a scope's segments wholesale, in the given (registry) order (U10's feed). */
export function setStatusSegments(scope: string, list: Array<StatusSegment & { key: string }>): void {
  setSegments((cur) => {
    if (!list.length) {
      if (!cur[scope]) return cur;
      const next = { ...cur };
      delete next[scope];
      return next;
    }
    return { ...cur, [scope]: { order: list.map((s) => s.key), by: Object.fromEntries(list.map((s) => [s.key, { text: s.text, title: s.title, tone: s.tone, node: s.node }])) } };
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

/** The scope's segments in feed order (insertion order for one-by-one sets). */
export function statusSegments(scope: string): Array<StatusSegment & { key: string }> {
  const s = segments()[scope];
  if (!s) return [];
  return s.order.filter((k) => s.by[k]).map((key) => ({ key, ...s.by[key]! }));
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
              <Show when={s.node} fallback={s.text}>
                <NodeView node={s.node!} path={['footer']} mode="auto" />
              </Show>
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
