import { lazy, type Component } from 'solid-js';

/** Drop-in feature modules: downstream owners add only their exclusively owned file.
 * Each file exports a default component. Globbing keeps absent modules compilable.
 */
const modules = import.meta.glob<{ default: Component }>('./{Compaction,Search}Settings.tsx');
const components = new Map<string, Component>();
export function featureSettings(id: 'compaction' | 'search'): Component | undefined {
  const path = `./${id === 'compaction' ? 'Compaction' : 'Search'}Settings.tsx`;
  const load = modules[path];
  if (!load) return undefined;
  if (!components.has(path)) components.set(path, lazy(load));
  return components.get(path);
}
export function hasFeatureSettings(id: 'compaction' | 'search'): boolean { return !!featureSettings(id); }
