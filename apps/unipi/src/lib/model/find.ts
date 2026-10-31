/**
 * `/model <name>` (P0-02): the catalogue row a typed name means — an exact
 * `provider/id` key or id first, then the display name, then a unique
 * substring of either. Ambiguous or unknown → undefined (the caller says so).
 */
import { modelKey, modelName, type ModelInfo } from './picker';

const norm = (s: string) => s.toLowerCase().replace(/[\s_]+/g, '-');

export function findModel(models: readonly ModelInfo[], query: string): ModelInfo | undefined {
  const q = query.trim();
  if (!q) return undefined;
  const exact = models.find((m) => modelKey(m) === q || m.id === q);
  if (exact) return exact;
  const nq = norm(q);
  const byName = models.filter((m) => norm(modelName(m)) === nq || norm(m.id.split('/').pop() ?? m.id) === nq);
  if (byName.length === 1) return byName[0];
  const partial = models.filter((m) => norm(modelKey(m)).includes(nq) || norm(modelName(m)).includes(nq));
  return partial.length === 1 ? partial[0] : undefined;
}
