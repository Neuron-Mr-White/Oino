/**
 * Context ring (WP-A14, Z-73): how full the window is, in the composer's
 * toolbar row. One surface, no second breakdown: click — or hover on the
 * desktop for 500 ms — opens the Session modal on its Context tab (D-7),
 * which owns the token breakdown, the cache-hit rate and compaction.
 *
 * QA#8: the ring must be visible at 0 % — the track is always drawn and the
 * arc keeps a 2 % floor, so an empty context is a ring, not a hole.
 */
import { createEffect, createResource, onCleanup, Show } from 'solid-js';
import type { SessionHandle } from '../../lib/session/store';
import { liveSource, parseContextUsage, type ContextUsage } from '../session/modalData';
import { openSessionModal } from '../session/modalTarget';

/** The ring's diameter (the toolbar row is 36 px). */
const SIZE = 22;
/** The stream fires many frames: numbers refresh at most this often. */
const REFRESH_MS = 1500;
/** Desktop hover before opening the Context tab (Z-73 "hover/click"). */
export const HOVER_OPEN_MS = 500;

/** The clamped percentage the ring draws. */
export function ringPercent(usage: ContextUsage | undefined): number {
  const pct = usage?.percent;
  if (pct == null || !Number.isFinite(pct)) return 0;
  return Math.min(100, Math.max(0, pct));
}

/** The arc's visible share: a 2 % floor keeps 0 % — and 1 % — visible (QA#8). */
export const arcShare = (pct: number): number => Math.max(2, Math.min(100, pct));

export function ContextRing(props: { session: SessionHandle }) {
  const s = () => props.session;
  let last = 0;
  const due = () => {
    const seq = s().state.seq;
    if (seq === 0) return true;
    const now = Date.now();
    return now - last >= REFRESH_MS ? ((last = now), true) : false;
  };
  const [data, { refetch }] = createResource(liveSource(s), async (state) => {
    if (state !== 'live') return undefined;
    const reply = await s().cmd<unknown>({ type: '_oino/get_context_usage' }).catch(() => undefined);
    return parseContextUsage(reply);
  });
  createEffect(() => {
    void s().state.seq; // track
    if (due()) refetch();
  });

  const usage = () => (data.loading && !data.latest ? undefined : data.latest);
  const pct = () => ringPercent(usage());
  const known = () => usage()?.percent != null;
  const label = () => (known() ? `Context: ${Math.round(pct())}% used` : 'Context usage unknown');

  const open = () => openSessionModal({ hostId: s().hostId, sessionId: s().sessionId(), tab: 'context' });
  let hoverTimer: ReturnType<typeof setTimeout> | undefined;
  const cancelHover = () => clearTimeout(hoverTimer);
  onCleanup(cancelHover);
  const hover = () => {
    if (typeof matchMedia !== 'function' || !matchMedia('(hover: hover)').matches) return;
    cancelHover();
    hoverTimer = setTimeout(open, HOVER_OPEN_MS);
  };

  // Geometry: a 2 px stroke inside the 22 px box.
  const r = (SIZE - 4) / 2;
  const c = 2 * Math.PI * r;
  const dash = () => (arcShare(pct()) / 100) * c;

  return (
    <button
      type="button"
      class="ctx-ring"
      aria-label={`${label()}. Open the context breakdown`}
      title={`${label()} — click for the breakdown`}
      onClick={open}
      onMouseEnter={hover}
      onMouseLeave={cancelHover}
      data-percent={known() ? Math.round(pct()) : undefined}
      data-empty={pct() <= 0 ? '' : undefined}
      data-high={pct() >= 80 ? '' : undefined}
      data-testid="context-ring"
    >
      <svg width={SIZE} height={SIZE} viewBox={`0 0 ${SIZE} ${SIZE}`} aria-hidden="true">
        <circle class="ctx-ring-track" cx={SIZE / 2} cy={SIZE / 2} r={r} fill="none" stroke-width="2" />
        <Show when={known()}>
          <circle
            class="ctx-ring-arc"
            cx={SIZE / 2}
            cy={SIZE / 2}
            r={r}
            fill="none"
            stroke-width="2"
            stroke-linecap="round"
            stroke-dasharray={`${dash()} ${c}`}
            transform={`rotate(-90 ${SIZE / 2} ${SIZE / 2})`}
          />
        </Show>
      </svg>
    </button>
  );
}
