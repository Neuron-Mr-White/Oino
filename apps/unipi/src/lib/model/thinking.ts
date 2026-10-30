/**
 * Thinking levels (WP-A14, Z-71): the chip shows the level pi runs and lets
 * the user pick another. The host is the only authority — `get_available_thinking_levels`
 * says what the current model supports, `set_thinking_level` / `cycle_thinking_level`
 * change it, and `thinking_level_changed` events keep the store's
 * `state.thinkingLevel` current. The level list is cached per
 * `(session, model)`: a model switch invalidates it.
 */
import { createSignal } from 'solid-js';
import type { SessionHandle } from '../session/store';

/** pi's levels in order (`ThinkingLevel::ALL`). */
export const THINKING_ORDER = ['off', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max'] as const;

/** The chip/menu label for a level id (`xhigh` reads "X-high", not "Xhigh"). */
export function thinkingLabel(level: string): string {
  if (level === 'xhigh') return 'X-high';
  if (!level) return '';
  return level[0]!.toUpperCase() + level.slice(1);
}

/** pi order, unknown future ids last (they still show). */
export function sortLevels(levels: readonly string[]): string[] {
  const rank = (level: string) => {
    const at = THINKING_ORDER.indexOf(level as (typeof THINKING_ORDER)[number]);
    return at < 0 ? THINKING_ORDER.length : at;
  };
  return [...levels].sort((a, b) => rank(a) - rank(b) || a.localeCompare(b));
}

/** `get_available_thinking_levels` reply (`{levels: string[]}`) → the ids. */
export function parseLevels(reply: unknown): string[] {
  const list =
    reply && typeof reply === 'object' && !Array.isArray(reply) ? (reply as { levels?: unknown }).levels : reply;
  if (!Array.isArray(list)) return [];
  return sortLevels(list.filter((level): level is string => typeof level === 'string' && level.length > 0));
}

/** Whether a level list means the chip is worth showing (a reasoning model has > 1 level). */
export const canThink = (levels: readonly string[] | undefined): boolean => (levels?.length ?? 0) > 1;

const [cache, setCache] = createSignal<Record<string, string[]>>({}, { equals: false });
const inFlight = new Set<string>();

/** The cache key: the model is part of it (its levels change with it). */
export const levelsKey = (s: SessionHandle): string => {
  const model = s.state.model;
  return `${s.key}|${model?.provider ?? ''}/${model?.id ?? ''}`;
};

/** The levels already known for the session's current model. */
export function thinkingLevels(s: SessionHandle): string[] | undefined {
  return cache()[levelsKey(s)];
}

/**
 * Fetches the levels once per (session, model) while the stream is live; a
 * failed fetch retries on the next call. Called from an effect that tracks
 * the model + connection, so a switch or a reconnect asks again.
 */
export function ensureThinkingLevels(s: SessionHandle): void {
  if (s.state.conn !== 'live') return;
  const key = levelsKey(s);
  if (cache()[key] || inFlight.has(key)) return;
  inFlight.add(key);
  void s
    .cmd<unknown>({ type: 'get_available_thinking_levels' })
    .then((reply) => {
      const levels = parseLevels(reply);
      if (levels.length) setCache((cur) => ({ ...cur, [key]: levels }));
    })
    .catch(() => undefined)
    .finally(() => inFlight.delete(key));
}

/** `set_thinking_level` (the store follows through `thinking_level_changed`). */
export async function setThinkingLevel(s: SessionHandle, level: string): Promise<void> {
  await s.cmd({ type: 'set_thinking_level', level });
}

/** `cycle_thinking_level` (Ctrl+T, Z-71): the host picks the next level. */
export async function cycleThinkingLevel(s: SessionHandle): Promise<void> {
  await s.cmd({ type: 'cycle_thinking_level' });
}

/** Tests/support: forget every cached list. */
export function resetThinkingLevels(): void {
  inFlight.clear();
  setCache({});
}
