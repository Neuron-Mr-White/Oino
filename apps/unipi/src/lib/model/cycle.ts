/**
 * The model **cycle set** (WP-A25, G-15 `/scoped-models`): pi's
 * `enabledModels` setting — the models Ctrl+P cycles in the TUI, and the set
 * the app's picker marks as "in the cycle". The picker edits it as checkboxes
 * (`_oino/get_settings` / `_oino/set_setting`, key `enabledModels`, global).
 *
 * The stored value is a list of pi **patterns** (`provider/model`, globs like
 * `anthropic/*`, `model:high`). Checkboxes own the exact `provider/model`
 * entries; anything else (a glob, a thinking suffix) is a *custom pattern* the
 * app leaves untouched and shows as a chip — editing it belongs to the
 * extension/TUI that wrote it.
 */

const GLOB = /[*?[\]{}]/;

/** A stored pattern that names exactly one model (`provider/model`, no glob, no `:level`). */
export function isExactPattern(pattern: string): boolean {
  if (GLOB.test(pattern)) return false;
  const slash = pattern.indexOf('/');
  if (slash <= 0 || slash === pattern.length - 1) return false;
  return !pattern.slice(slash + 1).includes(':');
}

/** `enabledModels` → the exact keys the checkboxes own + the custom patterns kept as-is. */
export function cycleKeysOf(value: unknown): { exact: string[]; custom: string[] } {
  const list = Array.isArray(value) ? value.filter((v): v is string => typeof v === 'string' && v.trim() !== '') : [];
  return {
    exact: [...new Set(list.filter(isExactPattern))],
    custom: list.filter((pattern) => !isExactPattern(pattern)),
  };
}

/** The value to store after one checkbox flips (empty set ⇒ `undefined`: every model cycles). */
export function toggleCycleKey(exact: readonly string[], custom: readonly string[], key: string, on: boolean): string[] | undefined {
  const next = new Set(exact);
  if (on) next.add(key);
  else next.delete(key);
  const list = [...custom, ...next];
  return list.length ? list : undefined;
}

/** "Cycle set · 4 of 12 models" (or "every model" when nothing is stored). */
export function cycleLabel(exact: readonly string[], custom: readonly string[], total: number): string {
  const n = exact.length;
  if (n === 0 && custom.length === 0) return `Cycle set · every model (${total})`;
  const extra = custom.length ? ` + ${custom.length} pattern${custom.length === 1 ? '' : 's'}` : '';
  return `Cycle set · ${n} of ${total} models${extra}`;
}

/** The key a catalogue row contributes (`provider/model`). */
export function cycleKeyOf(provider: string, id: string): string {
  return `${provider}/${id}`;
}

const globRe = (pattern: string): RegExp =>
  new RegExp(`^${pattern.replace(/[.+^${}()|\\]/g, '\\$&').replace(/\*/g, '.*').replace(/\?/g, '.')}$`, 'i');

/**
 * F-04: the keys Ctrl+Alt+P steps through, in order — never the whole catalogue.
 * The stored cycle set (exact keys and globs, `:level` suffixes dropped) resolved against
 * the models that can chat and run; else Fusion's curated `ranked` list; else — only when the
 * catalogue is small — every usable chat model. `undefined` = nothing sensible to cycle.
 */
export function cycleList(opts: {
  patterns: readonly string[];
  curated: readonly string[];
  usable: readonly string[];
  smallMax?: number;
}): string[] | undefined {
  const usable = new Set(opts.usable);
  const keep = (keys: Iterable<string>) => [...new Set(keys)].filter((k) => usable.has(k));
  if (opts.patterns.length) {
    const out: string[] = [];
    for (const raw of opts.patterns) {
      const pattern = raw.replace(/:(off|minimal|low|medium|high|xhigh)$/i, '');
      if (!/[*?]/.test(pattern)) out.push(pattern);
      else {
        const re = globRe(pattern);
        for (const k of opts.usable) if (re.test(k)) out.push(k);
      }
    }
    const list = keep(out);
    if (list.length) return list;
  }
  const curated = keep(opts.curated);
  if (curated.length) return curated;
  return opts.usable.length > 0 && opts.usable.length <= (opts.smallMax ?? 12) ? [...opts.usable] : undefined;
}
