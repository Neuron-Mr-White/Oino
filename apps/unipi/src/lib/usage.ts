/**
 * Settings › Usage data (owner fix 2026-11-04): the host's `usage_stats` op over
 * the session files, plus the pure formatting the page shares (tested).
 */
import { UsageDataSchema, type UsageData } from '@unipi/protocol';
import { hostCall } from './host/factory';
import type { HostResult } from './host/client';

export type { UsageData };

/** The ranges the page offers (days). */
export const USAGE_RANGES = [7, 30, 90] as const;
export type UsageRange = (typeof USAGE_RANGES)[number];

/** `usage_stats` for the last `days` local days on `hostId`. */
export function fetchUsage(hostId: string, days: number): Promise<HostResult<UsageData>> {
  return hostCall(hostId, 'usage_stats', { days, tz_offset_min: -new Date().getTimezoneOffset() }, UsageDataSchema);
}

/** `1.2M` / `34.5k` / `845`. */
export function compactTokens(n: number): string {
  if (!Number.isFinite(n)) return '—';
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(1)}B`;
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 10_000) return `${Math.round(n / 1000)}k`;
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
  return String(Math.round(n));
}

/** `$12.40` / `$0.0042`; an unpriced total is `—`, never `$0.00` (cost 0 while tokens were spent). */
export function costText(cost: number, unpricedTokens: number, tokens: number): string {
  if (cost > 0) return cost < 0.01 ? `$${cost.toFixed(4)}` : cost >= 100 ? `$${Math.round(cost)}` : `$${cost.toFixed(2)}`;
  return unpricedTokens > 0 || tokens > 0 ? '—' : '$0.00';
}

/** The note under a cost: what no price covers (`+ 1.2M tokens unpriced`, `no price`). */
export function costNote(cost: number, unpricedTokens: number): string | undefined {
  if (unpricedTokens <= 0) return undefined;
  return cost > 0 ? `+ ${compactTokens(unpricedTokens)} tokens unpriced` : `${compactTokens(unpricedTokens)} tokens, no price`;
}

/** `Sep 12` for a `YYYY-MM-DD` day. */
export function dayLabel(day: string, long = false): string {
  const d = new Date(`${day}T00:00:00.000Z`);
  if (Number.isNaN(d.getTime())) return day;
  return new Intl.DateTimeFormat('en', { timeZone: 'UTC', month: 'short', day: 'numeric', ...(long ? { weekday: 'short' } : {}) }).format(d);
}

/** The last path segment of a project folder, and the rest (`/home/u/app` → `app`, `/home/u`). */
export function projectLabel(path: string): { name: string; parent: string } {
  const trimmed = path.replace(/[\\/]+$/, '');
  const at = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'));
  if (at < 0) return { name: trimmed || 'unknown', parent: '' };
  return { name: trimmed.slice(at + 1) || trimmed, parent: trimmed.slice(0, at) };
}

/** Bar height share (0..1) per value against the peak; zero stays zero. */
export function barShares(values: number[]): number[] {
  const peak = Math.max(0, ...values);
  return values.map((v) => (peak > 0 ? v / peak : 0));
}

/** The index of the largest value (first on ties); -1 when everything is 0. */
export function peakIndex(values: number[]): number {
  let best = -1;
  let max = 0;
  values.forEach((v, i) => {
    if (v > max) {
      max = v;
      best = i;
    }
  });
  return best;
}

/** Cache read share of every input-side token (`read / (input + cache read + cache write)`), or undefined. */
export function cacheHitRate(t: { input: number; cache_read: number; cache_write: number }): number | undefined {
  const side = t.input + t.cache_read + t.cache_write;
  return side > 0 ? t.cache_read / side : undefined;
}
