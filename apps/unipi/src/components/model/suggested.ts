/**
 * Suggested models (P0-10): `unipi.fusion`'s curated preset ranks the picker.
 * `fusion.status.curated.ranked` (recent → curated leads → sidekicks) is the
 * "Suggested" group on top of the Single list — each row badged Lead /
 * Sidekick and with its curated default effort — and the full catalogue
 * stays below it, searchable. The Fusion columns put their curated lists
 * first the same way. Pure rules; the picker is markup.
 */
import type { FusionCurated } from '../../lib/model/fusion';
import { hits as modelHits, modelKey, modelName, shortModelName, type ModelInfo, type Selection } from '../../lib/model/picker';

export type SuggestedRow = { model: ModelInfo; lead: boolean; sidekick: boolean; effort?: string };

const hits = modelHits;

/** Family patterns for building a clean Suggested group when no `/fusion-preset` is saved yet (D-07 / F-04). */
const DEFAULT_FAMILIES: readonly RegExp[] = [
  /\bclaude[\w.-]*sonnet[\w.-]*[45]|\bsonnet[\w.-]*[45]/i,
  /\bclaude[\w.-]*opus[\w.-]*[45]|\bopus[\w.-]*[45]/i,
  /\bgpt-5(?!.*(?:mini|nano))|\bcodex|\bgpt-4\.1(?!.*(?:mini|nano))/i,
  /\bgemini-[23][\w.-]*(?:pro|flash)/i,
  /\bdeepseek[\w.-]*(?:v[34]|r1)/i,
  /\bclaude[\w.-]*haiku|\bgpt-5[\w.-]*mini|\bqwen[\w.-]*coder/i,
];

/**
 * Picks up to 6 distinct, high-signal chat models from `all` when the user has not
 * curated a `/fusion-preset` yet (`all.length > 6`). Deduplicates by display name so
 * two routes of the same model never both appear in Suggested or in the default cycle.
 */
export function defaultSuggestedKeys(
  all: readonly ModelInfo[],
  _currentKey?: string,
  startup?: string,
): string[] {
  if (all.length <= 6) return [];
  const byKey = new Map(all.map((m) => [modelKey(m), m]));
  const picked: string[] = [];
  const seenKeys = new Set<string>();
  const seenNames = new Set<string>();
  const add = (m: ModelInfo | undefined) => {
    if (!m) return false;
    const key = modelKey(m);
    const norm = shortModelName(modelName(m)).trim().toLowerCase();
    if (seenKeys.has(key) || (norm && seenNames.has(norm))) return false;
    seenKeys.add(key);
    if (norm) seenNames.add(norm);
    picked.push(key);
    return true;
  };
  if (startup) add(byKey.get(startup));
  for (const family of DEFAULT_FAMILIES) {
    if (picked.length >= 6) break;
    const match = all.find((m) => !seenKeys.has(modelKey(m)) && family.test(`${m.id} ${m.name ?? ''}`));
    add(match);
  }
  for (const m of all) {
    if (picked.length >= 5) break;
    add(m);
  }
  return picked;
}

/** The curated roles + default effort of one model key. */
export function curatedRole(curated: FusionCurated | undefined, key: string): { lead: boolean; sidekick: boolean; effort?: string } {
  const effort = curated?.effort[key];
  return {
    lead: !!curated?.lead.some((e) => e.key === key),
    sidekick: !!curated?.sidekick.some((e) => e.key === key),
    ...(effort ? { effort } : {}),
  };
}

/**
 * The Single list in two groups: `suggested` (the curated `ranked` order,
 * catalogue rows only) and `rest` (the whole catalogue minus those, the
 * running model first). A search filters both groups.
 */
export function groupedModels(
  all: readonly ModelInfo[],
  curated: FusionCurated | undefined,
  current: Selection | undefined,
  query: string,
  startup?: string,
): { suggested: SuggestedRow[]; rest: ModelInfo[] } {
  const q = query.trim().toLowerCase();
  const byKey = new Map(all.map((m) => [modelKey(m), m]));
  const suggested: SuggestedRow[] = [];
  const taken = new Set<string>();
  for (const entry of curated?.ranked ?? []) {
    const m = byKey.get(entry.key);
    if (!m || taken.has(entry.key)) continue;
    taken.add(entry.key);
    if (q && !hits(m, q)) continue;
    suggested.push({ model: m, ...curatedRole(curated, entry.key) });
  }
  // D-07: when no `/fusion-preset` is saved (or on New chat before a chat ran), surface a clean Suggested group on large catalogues.
  if (taken.size === 0 && all.length > 6) {
    const curKey = current?.kind === 'single' ? current.key : undefined;
    for (const key of defaultSuggestedKeys(all, curKey, startup)) {
      const m = byKey.get(key);
      if (!m || taken.has(key)) continue;
      taken.add(key);
      if (q && !hits(m, q)) continue;
      suggested.push({ model: m, ...curatedRole(curated, key) });
    }
  }
  let rest = all.filter((m) => !taken.has(modelKey(m)) && (!q || hits(m, q)));
  if (!q && current?.kind === 'single') {
    const at = rest.findIndex((m) => modelKey(m) === current.key);
    if (at > 0) rest = [rest[at]!, ...rest.slice(0, at), ...rest.slice(at + 1)];
  }
  return { suggested, rest };
}

/** A Fusion column with its curated list first (in curated order), then the rest. */
export function curatedFirst(rows: readonly ModelInfo[], keys: readonly string[]): { curated: ModelInfo[]; rest: ModelInfo[] } {
  const byKey = new Map(rows.map((m) => [modelKey(m), m]));
  const curated: ModelInfo[] = [];
  for (const key of keys) {
    const m = byKey.get(key);
    if (m && !curated.includes(m)) curated.push(m);
  }
  const set = new Set(curated);
  return { curated, rest: rows.filter((m) => !set.has(m)) };
}

/**
 * Where a row runs, short: the provider plus any route in the id
 * (`omniroute` + `cc/claude-opus-5-5` → "omniroute · cc") — two routes to
 * the same model never read alike, and no raw id is shown on the row.
 */
export function routeLabel(m: Pick<ModelInfo, 'provider' | 'id'>): string {
  const parts = m.id.split('/').filter(Boolean);
  return [m.provider, ...parts.slice(0, -1)].join(' · ');
}

/** The row tooltip: name, then provider · id in full. */
export function rowTitle(name: string, m: Pick<ModelInfo, 'provider' | 'id'>): string {
  return `${name}\n${m.provider} · ${m.id}`;
}

/** The `defaultProvider` / `defaultModel` pair as one key (`undefined` when unset). */
export function startupKey(provider: unknown, model: unknown): string | undefined {
  if (typeof model !== 'string' || !model) return undefined;
  if (typeof provider === 'string' && provider) return `${provider}/${model}`;
  return model.includes('/') ? model : undefined;
}
