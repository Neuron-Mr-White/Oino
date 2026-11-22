/**
 * Context tab (WP-A17, Z-73): used/total tokens with a meter, the token
 * breakdown (input / output / cache read / cache write, `get_session_stats`),
 * the cache-hit rate (tinted good at ≥ 78 %) and the compact actions — one
 * surface, no header strip (Z-75). Data: `_oino/get_context_usage` +
 * `get_session_stats`, refetched when the session's stream advances
 * (throttled: a streaming reply fires many frames).
 */
import { createEffect, createResource, Show } from 'solid-js';
import { Trash2 } from 'lucide-solid';
import type { SessionHandle } from '../../lib/session/store';
import { formatCount, parseContextUsage, sessionCostView, parseSessionStats, liveSource, transcriptToolCalls, type ContextUsage, type SessionStats } from './modalData';

/** Cache-read share of everything read: ≥ 78 % counts as a healthy hit rate (Z-73). */
export const CACHE_HIT_GOOD = 0.78;
const REFRESH_MS = 1500;

export function cacheHitRate(tokens: SessionStats['tokens']): number | undefined {
  const read = tokens.input + tokens.cacheRead;
  if (read <= 0) return undefined;
  return tokens.cacheRead / read;
}

/** The stacked breakdown bar's segments: share of the token total. */
export function breakdownSegments(tokens: SessionStats['tokens']): Array<{ key: string; label: string; value: number; color: string }> {
  const parts = [
    { key: 'in', label: 'Tokens in', value: tokens.input, color: 'var(--color-wm-cyan)' },
    { key: 'out', label: 'Tokens out', value: tokens.output, color: 'var(--color-claw)' },
    { key: 'read', label: 'Cache hit', value: tokens.cacheRead, color: 'var(--color-wm-green)' },
    { key: 'write', label: 'Cache write', value: tokens.cacheWrite, color: 'var(--color-gold)' },
  ];
  return parts.filter((p) => p.value > 0);
}

export function ContextTab(props: { s: SessionHandle }) {
  const s = () => props.s;
  // Any stream advance (seq) refreshes the numbers, at most every REFRESH_MS.
  let last = 0;
  const due = () => {
    const seq = s().state.seq;
    if (seq === 0) return true;
    const now = Date.now();
    return now - last >= REFRESH_MS ? ((last = now), true) : false;
  };
  const [data, { refetch }] = createResource(liveSource(s), async (state) => {
    if (state !== 'live') return undefined;
    const handle = s();
    const [usage, stats] = await Promise.all([
      handle.cmd<unknown>({ type: '_oino/get_context_usage' }).catch(() => undefined),
      handle.cmd<unknown>({ type: 'get_session_stats' }).catch(() => undefined),
    ]);
    return { usage: parseContextUsage(usage), stats: parseSessionStats(stats) };
  });
  createEffect(() => {
    void s().state.seq; // track
    if (due()) refetch();
  });
  const usage = () => (data.loading && !data.latest ? undefined : data.latest?.usage);
  const stats = () => data.latest?.stats;
  const ctx = (): ContextUsage | undefined => usage() ?? stats()?.contextUsage ?? undefined;
  const pct = () => (ctx()?.percent == null ? undefined : Math.round(ctx()!.percent!));
  const segments = () => (stats() ? breakdownSegments(stats()!.tokens) : []);
  const hit = () => (stats() ? cacheHitRate(stats()!.tokens) : undefined);
  const compacting = () => s().state.run.phase === 'compacting';
  const compact = () => void s().cmd({ type: 'compact' }).catch(() => {});

  return (
    <Show when={!data.loading || data.latest} fallback={<div class="sm-skeleton"><span style={{ width: '62%' }} /><span /><span style={{ width: '40%' }} /></div>}>
      <div class="sm-ctx-meter" data-testid="sm-context">
        <div class="sm-ctx-meter-top">
          <span class="sm-ctx-pct" data-high={pct() != null && pct()! >= 80 ? '' : undefined}>
            {pct() == null ? '—' : `${pct()}%`}
          </span>
          <span class="sm-ctx-tokens">
            {ctx() && ctx()!.tokens != null ? `${formatCount(ctx()!.tokens!)} / ${formatCount(ctx()!.contextWindow)} tokens` : 'context usage unknown'}
          </span>
        </div>
        <div class="sm-bar" aria-hidden="true">
          <div class="sm-bar-fill" data-high={pct() != null && pct()! >= 80 ? '' : undefined} style={{ width: `${Math.min(100, Math.max(0, pct() ?? 0))}%` }} />
        </div>
        <Show when={segments().length > 0}>
          <div class="sm-ctx-stack" aria-hidden="true">
            {segments().map((seg) => (
              <span style={{ width: `${(seg.value / Math.max(1, stats()!.tokens.total)) * 100}%`, background: seg.color }} />
            ))}
          </div>
          <div class="sm-legend">
            {segments().map((seg) => (
              <span>
                <i style={{ background: seg.color }} />
                {seg.label} {formatCount(seg.value)}
              </span>
            ))}
          </div>
        </Show>
      </div>

      <Show when={stats()}>
        <div class="sm-grid">
          <div class="sm-stat">
            <span class="sm-stat-label">Cache hit</span>
            <span class="sm-stat-value" data-good={hit() != null && hit()! >= CACHE_HIT_GOOD ? '' : undefined}>
              {hit() == null ? '—' : `${Math.round(hit()! * 100)}%`}
            </span>
          </div>
          <div class="sm-stat" title={sessionCostView(stats()).title} data-testid="sm-session-cost">
            <span class="sm-stat-label">Session cost</span>
            <span class="sm-stat-value">{sessionCostView(stats()).text}</span>
            <Show when={sessionCostView(stats()).sub}>{(sub) => <span class="sm-stat-sub">{sub()}</span>}</Show>
          </div>
          <div class="sm-stat">
            <span class="sm-stat-label">Replies</span>
            <span class="sm-stat-value">{stats()!.assistantMessages}</span>
          </div>
          <div class="sm-stat">
            <span class="sm-stat-label">Tool calls</span>
            <span class="sm-stat-value">{Math.max(stats()!.toolCalls, transcriptToolCalls(s().state.entries))}</span>
          </div>
        </div>
      </Show>

      <div class="sm-actions">
        <button type="button" class="btn btn-secondary btn-sm" disabled={compacting() || s().state.conn !== 'live'} aria-busy={compacting() ? 'true' : undefined} onClick={compact} data-testid="sm-compact">
          <Trash2 size={14} />
          {compacting() ? 'Compacting…' : 'Compact now'}
        </button>
        <span class="hint">Summarises the conversation; auto-compaction keeps doing it on its own.</span>
      </div>
    </Show>
  );
}
