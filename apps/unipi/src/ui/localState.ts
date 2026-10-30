/**
 * Frontend-local UI state (ui-protocol D-U13, §4.6): selection, offsets,
 * scroll, tab choice, tree expansion, collapsed blocks, typed text — like
 * ratatui `StatefulWidget`s. Keyed by `(entryId, path, name)`, kept across
 * re-mounts (virtualization, tab switches) and dropped with the entry.
 *
 * "Prop changed ⇒ adopt": a `state`/`value`/`selected`/`expanded`/`collapsed`
 * prop wins when its value changes between tree updates; otherwise the local
 * value persists.
 */
import { createComputed, createSignal, on, untrack, type Accessor, type Setter } from 'solid-js';

type Cell<T> = { get: Accessor<T>; set: Setter<T>; seen: string | undefined };

const cells = new Map<string, Map<string, Cell<unknown>>>();

function stamp(v: unknown): string | undefined {
  return v === undefined ? undefined : JSON.stringify(v);
}

function cellFor<T>(entryId: string, key: string, init: () => T, propNow: unknown): Cell<T> {
  let byEntry = cells.get(entryId);
  if (!byEntry) cells.set(entryId, (byEntry = new Map()));
  let cell = byEntry.get(key) as Cell<T> | undefined;
  if (!cell) {
    const [get, set] = createSignal<T>(init(), { equals: false });
    cell = { get, set, seen: stamp(propNow) };
    byEntry.set(key, cell as Cell<unknown>);
  }
  return cell;
}

/**
 * Local state bound to an optional driving prop. `prop()` is read reactively;
 * when its serialized value differs from the last one seen, the local value
 * adopts it (`fromProp` maps prop → state). Returns `[value, setValue]`.
 */
export function useLocal<T, P = T>(
  entryId: string,
  path: readonly string[],
  name: string,
  opts: { prop?: Accessor<P | undefined>; fromProp?: (p: P) => T; init: () => T },
): [Accessor<T>, (v: T) => void] {
  const key = `${path.join('\u0000')}\u0001${name}`;
  const propNow = opts.prop ? untrack(opts.prop) : undefined;
  const map = opts.fromProp ?? ((p: P) => p as unknown as T);
  const cell = cellFor<T>(entryId, key, () => (propNow !== undefined ? map(propNow) : opts.init()), propNow);
  if (opts.prop) {
    const prop = opts.prop;
    createComputed(
      on(prop, (p) => {
        const s = stamp(p);
        if (s === cell.seen) return;
        cell.seen = s;
        if (p !== undefined) cell.set(() => map(p));
      }),
    );
  }
  return [cell.get, (v: T) => cell.set(() => v)];
}

/** Drops an entry's local state (entry deleted, §6.6). */
export function dropLocal(entryId: string): void {
  cells.delete(entryId);
}

/** Clears everything (tests). */
export function resetLocal(): void {
  cells.clear();
}

/** How many entries hold local state (tests). */
export function localEntries(): number {
  return cells.size;
}
