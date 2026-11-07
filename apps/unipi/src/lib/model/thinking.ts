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
import { cachedModels, rememberModels } from '../../components/draft/hostCache';
import { parseModels, type ModelInfo } from './picker';

/** pi's levels in order (`ThinkingLevel::ALL`). */
export const THINKING_ORDER = ['off', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max'] as const;

/** The chip/menu label for a level id (`xhigh` reads "X-high", not "Xhigh"). */
export function thinkingLabel(level: string): string {
  if (level === 'xhigh') return 'X-high';
  if (!level) return '';
  return level[0]!.toUpperCase() + level.slice(1);
}

const HINTS: Record<string, string> = {
  off: 'No reasoning, fastest replies',
  minimal: 'A brief think before answering',
  low: 'Light reasoning for simple tasks',
  medium: 'Balanced depth and speed',
  high: 'Deeper reasoning for hard problems',
  xhigh: 'Very deep reasoning, slower',
  max: 'The most the model can reason',
};

/** The thinking menu's one-line description of a level (D-23); empty for an unknown id. */
export function thinkingHint(level: string): string {
  return HINTS[level] ?? '';
}

/**
 * The level a session actually runs when `want` is asked of a model offering
 * `levels` (oino's `ThinkingLevel::clamp`: the nearest offered level at or
 * above, else the highest below) — the draft shows this, not a level the new
 * chat can't keep (D-02).
 */
export function clampLevel(want: string, levels: readonly string[]): string {
  if (!levels.length || levels.includes(want)) return want;
  const rank = (level: string) => THINKING_ORDER.indexOf(level as (typeof THINKING_ORDER)[number]);
  const w = rank(want);
  if (w < 0) return levels[0]!;
  const sorted = sortLevels(levels).filter((level) => rank(level) >= 0);
  return sorted.find((level) => rank(level) >= w) ?? sorted[sorted.length - 1] ?? want;
}

/**
 * The levels a catalogue model offers (oino's `Model::available_thinking_levels`):
 * a model that doesn't reason has only `off`; otherwise every level except the
 * ones its `thinkingLevelMap` nulls — and `xhigh` / `max` only when the map
 * lists them. `undefined` when the catalogue row says nothing about reasoning.
 */
export function levelsOfModel(m: { reasoning?: boolean; thinkingLevelMap?: Record<string, unknown> } | undefined): string[] | undefined {
  if (!m || m.reasoning === undefined) return undefined;
  if (!m.reasoning) return ['off'];
  const map = m.thinkingLevelMap ?? {};
  return THINKING_ORDER.filter((level) => {
    const entry = map[level];
    if (entry === null) return false;
    if (entry === undefined) return level !== 'xhigh' && level !== 'max';
    return true;
  });
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
const [catalogues, setCatalogues] = createSignal<Record<string, ModelInfo[]>>({});
const cataloguesInFlight = new Set<string>();

/** The cache key: the model is part of it (its levels change with it). */
export const levelsKey = (s: SessionHandle): string => {
  const model = s.state.model;
  return `${s.key}|${model?.provider ?? ''}/${model?.id ?? ''}`;
};

/** The levels already known for the session's current model. */
export function thinkingLevels(s: SessionHandle): string[] | undefined {
  const model = s.state.model;
  const rows = catalogues()[s.hostId] ?? cachedModels(s.hostId);
  const row = rows?.find((m) => m.provider === model?.provider && m.id === model?.id);
  return cache()[levelsKey(s)] ?? levelsOfModel(row);
}

/**
 * Fetches the levels once per (session, model) while the stream is live; a
 * failed fetch retries on the next call. Called from an effect that tracks
 * the model + connection, so a switch or a reconnect asks again.
 */
export function ensureThinkingLevels(s: SessionHandle): void {
  if (s.state.conn !== 'live') return;
  const key = levelsKey(s);
  // Fetch independently of the level request: either can hang or fail, and the
  // catalog also supplies the next model immediately after a switch.
  if (!catalogues()[s.hostId] && !cataloguesInFlight.has(s.hostId)) {
    cataloguesInFlight.add(s.hostId);
    void s.cmd<unknown>({ type: 'get_available_models' }).then((reply) => {
      const rows = parseModels(reply);
      if (rows.length) {
        rememberModels(s.hostId, rows);
        setCatalogues((cur) => ({ ...cur, [s.hostId]: rows }));
      }
    }).catch(() => undefined).finally(() => cataloguesInFlight.delete(s.hostId));
  }
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
  cataloguesInFlight.clear();
  setCache({});
  setCatalogues({});
}
