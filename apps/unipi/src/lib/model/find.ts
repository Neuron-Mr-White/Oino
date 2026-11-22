/**
 * `/model <name>` (P0-02): the catalogue row a typed name means — an exact
 * `provider/id` key or id first, then the id's last segment or the display
 * name, then a substring of either. Several matches: the one on the current
 * provider wins, else the caller lists them (`ambiguous`).
 */
import { modelKey, modelName, type ModelInfo } from './picker';

const norm = (s: string) => s.toLowerCase().replace(/[\s_]+/g, '-');
const leaf = (m: ModelInfo) => norm(m.id.split('/').pop() ?? m.id);

export type ModelMatch = { hit: ModelInfo } | { ambiguous: ModelInfo[] } | { none: true };

export function matchModel(models: readonly ModelInfo[], query: string, current?: { provider?: string }): ModelMatch {
  const q = query.trim();
  if (!q) return { none: true };
  const pick = (rows: ModelInfo[]): ModelMatch | undefined => {
    if (rows.length === 1) return { hit: rows[0]! };
    if (rows.length > 1) {
      const same = rows.filter((m) => current?.provider && m.provider === current.provider);
      return same.length === 1 ? { hit: same[0]! } : { ambiguous: rows };
    }
    return undefined;
  };
  const nq = norm(q);
  return (
    pick(models.filter((m) => modelKey(m) === q || m.id === q)) ??
    pick(models.filter((m) => leaf(m) === nq || norm(modelName(m)) === nq)) ??
    pick(models.filter((m) => norm(modelKey(m)).includes(nq) || norm(modelName(m)).includes(nq))) ?? { none: true }
  );
}

/** The single match, or undefined (ambiguous / unknown). */
export function findModel(models: readonly ModelInfo[], query: string, current?: { provider?: string }): ModelInfo | undefined {
  const m = matchModel(models, query, current);
  return 'hit' in m ? m.hit : undefined;
}
