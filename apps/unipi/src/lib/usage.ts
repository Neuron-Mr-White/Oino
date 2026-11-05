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

// ── ZCode-parity extras (owner 2026-11-05): lifetime summary, activity heatmap, model trend ──

export type UsageDayLike = { day: string; tokens: number; turns?: number; models?: Record<string, number> };

/** Lifetime figures over a long window (the 366-day fetch): total, peak day, streaks. Pure. */
export function lifetimeSummary(daily: readonly UsageDayLike[]): { total: number; peakTokens: number; peakDay?: string; currentStreak: number; longestStreak: number } {
  let total = 0;
  let peakTokens = 0;
  let peakDay: string | undefined;
  let run = 0;
  let longest = 0;
  for (const d of daily) {
    total += d.tokens;
    if (d.tokens > peakTokens) {
      peakTokens = d.tokens;
      peakDay = d.day;
    }
    run = d.tokens > 0 ? run + 1 : 0;
    longest = Math.max(longest, run);
  }
  // Current streak counts back from today; an empty today keeps yesterday's streak alive.
  let current = 0;
  let i = daily.length - 1;
  if (i >= 0 && daily[i]!.tokens <= 0) i--;
  for (; i >= 0 && daily[i]!.tokens > 0; i--) current++;
  return { total, peakTokens, ...(peakDay ? { peakDay } : {}), currentStreak: current, longestStreak: longest };
}

/** `3d 4h` / `2h 5m` / `12m` for a session length. Pure. */
export function durationText(ms: number): string {
  const m = Math.max(0, Math.floor(ms / 60_000));
  const d = Math.floor(m / 1440);
  const h = Math.floor((m % 1440) / 60);
  const mm = m % 60;
  const out: string[] = [];
  if (d) out.push(`${d}d`);
  if (h) out.push(`${h}h`);
  if (mm || out.length === 0) out.push(`${mm}m`);
  return out.slice(0, 2).join(' ');
}

export type HeatMode = 'daily' | 'weekly' | 'cumulative';
export type HeatCell = { day: string; tokens: number; turns: number; level: 0 | 1 | 2 | 3 | 4; future?: boolean };
export type HeatColumn = { key: string; month: string; cells: HeatCell[]; total: number; turns: number };

const dayIndexOf = (day: string): number => Math.floor(Date.parse(`${day}T00:00:00Z`) / 86_400_000);
const dayOfIndex = (i: number): string => new Date(i * 86_400_000).toISOString().slice(0, 10);
const levelOf = (v: number, max: number): 0 | 1 | 2 | 3 | 4 => (v <= 0 || max <= 0 ? 0 : (Math.min(4, Math.max(1, Math.ceil((v / max) * 4))) as 1 | 2 | 3 | 4));

/**
 * The 52-week activity grid (ZCode's heatmap): Sunday-first columns ending on the
 * newest day. `daily`: each column's 7 cells shaded by that day's share of the busiest
 * day; `weekly`: a whole column shaded by its week total; `cumulative`: by the running
 * total. Pure.
 */
export function heatmapColumns(daily: readonly UsageDayLike[], mode: HeatMode = 'daily', weeks = 52): HeatColumn[] {
  if (!daily.length) return [];
  const by = new Map(daily.map((d) => [d.day, d]));
  const last = dayIndexOf(daily[daily.length - 1]!.day);
  const lastDow = new Date(last * 86_400_000).getUTCDay();
  const start = last - lastDow - (weeks - 1) * 7;
  const cols: HeatColumn[] = [];
  for (let w = 0; w < weeks; w++) {
    const cells: HeatCell[] = [];
    let total = 0;
    let turns = 0;
    for (let k = 0; k < 7; k++) {
      const idx = start + w * 7 + k;
      const day = dayOfIndex(idx);
      const d = by.get(day);
      const t = d?.tokens ?? 0;
      total += t;
      turns += d?.turns ?? 0;
      cells.push({ day, tokens: t, turns: d?.turns ?? 0, level: 0, ...(idx > last ? { future: true } : {}) });
    }
    const first = cells.find((c) => c.day.endsWith('-01'))?.day ?? cells[0]!.day;
    cols.push({ key: `w${w}`, month: first.slice(0, 7), cells, total, turns });
  }
  if (mode === 'daily') {
    const max = Math.max(0, ...cols.flatMap((c) => c.cells.map((x) => x.tokens)));
    for (const c of cols) for (const x of c.cells) x.level = levelOf(x.tokens, max);
  } else {
    let run = 0;
    const values = cols.map((c) => (mode === 'weekly' ? c.total : (run += c.total)));
    const max = Math.max(0, ...values);
    cols.forEach((c, i) => {
      const lv = levelOf(values[i]!, max);
      // A column fills from the bottom: taller for a bigger value (7 cells = the peak).
      const filled = values[i]! > 0 && max > 0 ? Math.max(1, Math.ceil((values[i]! / max) * 7)) : 0;
      c.cells.forEach((x, k) => (x.level = k >= 7 - filled ? lv : 0));
      if (mode === 'cumulative') c.total = values[i]!;
    });
  }
  return cols;
}

/** Month labels for the grid: one per month start, spanning its columns. Pure. */
export function heatmapMonths(cols: readonly HeatColumn[]): Array<{ key: string; label: string; span: number }> {
  const out: Array<{ key: string; label: string; span: number }> = [];
  for (const c of cols) {
    const last = out[out.length - 1];
    if (last?.key === c.month) last.span++;
    else out.push({ key: c.month, label: new Intl.DateTimeFormat('en', { month: 'short', timeZone: 'UTC' }).format(new Date(`${c.month}-01T00:00:00Z`)), span: 1 });
  }
  return out;
}

/** The trend chart's series: the model names present in `daily[].models`, biggest first (`other` last). Pure. */
export function trendSeries(daily: readonly UsageDayLike[]): string[] {
  const sum = new Map<string, number>();
  for (const d of daily) for (const [k, v] of Object.entries(d.models ?? {})) sum.set(k, (sum.get(k) ?? 0) + v);
  return [...sum.entries()].sort((a, b) => (a[0] === 'other' ? 1 : b[0] === 'other' ? -1 : b[1] - a[1])).map(([k]) => k);
}

/** The model share donut: top `n - 1` models, the rest as "Other models". Pure. */
export function modelShares(models: ReadonlyArray<{ name: string; tokens: number }>, n = 6): Array<{ name: string; tokens: number; share: number }> {
  const live = models.filter((m) => m.tokens > 0);
  const total = live.reduce((s, m) => s + m.tokens, 0);
  if (total <= 0) return [];
  const head = live.length > n ? live.slice(0, n - 1) : live;
  const rest = live.length > n ? live.slice(n - 1).reduce((s, m) => s + m.tokens, 0) : 0;
  const rows = head.map((m) => ({ name: m.name, tokens: m.tokens, share: m.tokens / total }));
  if (rest > 0) rows.push({ name: 'Other models', tokens: rest, share: rest / total });
  return rows;
}

/** The chart palette (Unipi tokens first). */
export const USAGE_PALETTE = ['#f07818', '#38bdf8', '#a78bfa', '#4ade80', '#f472b6', '#facc15', '#94a3b8'];
