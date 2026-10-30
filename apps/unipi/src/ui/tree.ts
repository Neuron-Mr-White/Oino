/**
 * `NodeOp` apply (ui-protocol §4.7) on a Solid store draft — or any plain
 * object — mirroring `oino_ui::patch::apply` (`crates/oino-ui/src/patch.rs`).
 *
 * Ops mutate in place so Solid's fine-grained reactivity re-renders only the
 * components reading a changed prop (§13.2 step 3). Every mutation goes
 * through a `Tx` undo log: a batch whose op doesn't resolve is rolled back
 * whole and the caller requests a resync (§9.2), so a refused patch changes
 * nothing — the same contract as the TUI's mirror (`oino-view` `RegistryMirror`).
 */
import type { NodeOp, PatchField } from './types';

type Obj = Record<string, unknown>;

export class TreeOpError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'TreeOpError';
  }
}

/** Undo log over in-place mutations. */
export class Tx {
  private log: Array<() => void> = [];

  set(obj: Obj, key: string, value: unknown): void {
    const had = Object.prototype.hasOwnProperty.call(obj, key);
    const old = obj[key];
    this.log.push(() => {
      if (had) obj[key] = old;
      else delete obj[key];
    });
    if (value === undefined) delete obj[key];
    else obj[key] = value;
  }

  splice(arr: unknown[], index: number, remove: number, insert: unknown[]): void {
    const removed = arr.splice(index, remove, ...insert);
    this.log.push(() => {
      arr.splice(index, insert.length, ...removed);
    });
  }

  /** Records a custom undo step. */
  undo(step: () => void): void {
    this.log.push(step);
  }

  rollback(): void {
    for (let i = this.log.length - 1; i >= 0; i--) this.log[i]!();
    this.log = [];
  }
}

const isObj = (v: unknown): v is Obj => typeof v === 'object' && v !== null && !Array.isArray(v);

/** Collections each node kind owns (patch.rs `collection_fields`). */
export const COLLECTIONS: Record<string, readonly PatchField[]> = {
  Layout: ['children'],
  Tabs: ['titles', 'panes'],
  List: ['items'],
  Table: ['rows'],
  Tree: ['nodes'],
  Log: ['lines'],
  Chart: ['datasets'],
  Sparkline: ['data'],
  BarChart: ['data'],
};

/** Ring limits for `append` (§4.8): Log trims to `maxLines`, lists/tables to 5 000. */
export const MAX_ITEMS = 5000;
export const DEFAULT_LOG_LINES = 2000;

export function logLimit(node: Obj): number {
  const max = typeof node.maxLines === 'number' && node.maxLines > 0 ? node.maxLines : DEFAULT_LOG_LINES;
  return Math.min(max, MAX_ITEMS);
}

/** Derived member key (§4.1): explicit `key`, else positional `#<index>`. */
export function derivedKey(item: unknown, index: number): string {
  const k = isObj(item) ? item.key : undefined;
  return typeof k === 'string' && k.length > 0 ? k : `#${index}`;
}

/** The explicit key of a collection item, if it has one. */
function explicitKey(field: PatchField, item: unknown): string | undefined {
  if (!isObj(item)) return undefined;
  if (field === 'lines' || field === 'datasets' || field === 'data') return undefined;
  if (field === 'items' && !('content' in item)) return undefined; // a bare Text item
  return typeof item.key === 'string' ? item.key : undefined;
}

function fmt(path: readonly string[]): string {
  return `[${path.map((s) => JSON.stringify(s)).join(', ')}]`;
}

function kindOf(node: Obj): string {
  return typeof node.t === 'string' ? node.t : '?';
}

/** The node at `path` under `holder[slot]` (§4.6 paths). */
function nodeAt(holder: Obj, slot: string, path: readonly string[]): { parent: Obj; key: string; node: Obj } {
  let parent = holder;
  let key = slot;
  let node = holder[slot];
  for (let depth = 0; depth < path.length; depth++) {
    const seg = path[depth]!;
    if (!isObj(node)) throw new TreeOpError(`missing path ${fmt(path.slice(0, depth + 1))}`);
    let nextParent: Obj | undefined;
    let nextKey: string | undefined;
    if (seg === 'child' && node.t === 'Block') {
      nextParent = node;
      nextKey = 'child';
    } else if (seg === 'fallback') {
      nextParent = node;
      nextKey = 'fallback';
    } else if (seg === 'action' && node.t === 'Empty') {
      nextParent = node;
      nextKey = 'action';
    } else if (node.t === 'Layout' && Array.isArray(node.children)) {
      const children = node.children as unknown[];
      const i = children.findIndex((c, index) => derivedKey(c, index) === seg);
      if (i >= 0) {
        nextParent = children as unknown as Obj;
        nextKey = String(i);
      }
    } else if (node.t === 'Tabs' && isObj(node.panes) && seg in node.panes) {
      nextParent = node.panes;
      nextKey = seg;
    }
    if (!nextParent || nextKey === undefined || !isObj(nextParent[nextKey]))
      throw new TreeOpError(`missing path ${fmt(path.slice(0, depth + 1))}`);
    parent = nextParent;
    key = nextKey;
    node = nextParent[nextKey];
  }
  if (!isObj(node)) throw new TreeOpError(`missing path ${fmt(path)}`);
  return { parent, key, node };
}

function needField(node: Obj, field: PatchField, path: readonly string[]): void {
  if (!(COLLECTIONS[kindOf(node)] ?? []).includes(field))
    throw new TreeOpError(`field "${field}" does not exist on ${kindOf(node)} at ${fmt(path)}`);
}

function needNode(value: unknown, field: PatchField, path: readonly string[]): Obj {
  if (!isObj(value) || typeof value.t !== 'string') throw new TreeOpError(`bad ${field} item at ${fmt(path)}: not a node`);
  return value;
}

function sortedPanes(panes: unknown): Array<[string, unknown]> {
  return isObj(panes) ? Object.keys(panes).sort().map((k) => [k, panes[k]] as [string, unknown]) : [];
}

function seqOf(node: Obj, field: PatchField, tx: Tx): unknown[] {
  const cur = node[field];
  if (Array.isArray(cur)) return cur;
  tx.set(node, field, []);
  return node[field] as unknown[];
}

/** Applies one op. Throws `TreeOpError` when it doesn't resolve (nothing is rolled back here). */
export function applyNodeOp(holder: Obj, slot: string, op: NodeOp, tx: Tx): void {
  const path = Array.isArray(op.path) ? op.path : [];
  switch (op.op) {
    case 'replace': {
      const target = path.length === 0 ? { parent: holder, key: slot } : nodeAt(holder, slot, path);
      tx.set(target.parent, target.key, needNode(op.node, 'children', path));
      return;
    }
    case 'set': {
      const { node } = nodeAt(holder, slot, path);
      if (!isObj(op.props)) throw new TreeOpError(`bad patch at ${fmt(path)}: props is not an object`);
      for (const [k, v] of Object.entries(op.props)) {
        if (k === 't') throw new TreeOpError(`bad patch at ${fmt(path)}: "t" cannot be set`);
        tx.set(node, k, v === null ? undefined : v);
      }
      return;
    }
    case 'splice': {
      const { node } = nodeAt(holder, slot, path);
      needField(node, op.field, path);
      const index = op.index >>> 0;
      const remove = op.remove >>> 0;
      const insert = Array.isArray(op.insert) ? op.insert : [];
      if (op.field === 'panes') {
        const entries = sortedPanes(node.panes);
        if (index + remove > entries.length) throw new TreeOpError(`index ${index} out of bounds (len ${entries.length}) at ${fmt(path)}`);
        const added = insert.map((v) => {
          const n = needNode(v, 'panes', path);
          if (typeof n.key !== 'string') throw new TreeOpError(`pane insert without a key at ${fmt(path)}`);
          return [n.key, n] as [string, unknown];
        });
        entries.splice(index, remove, ...added);
        tx.set(node, 'panes', entries.length ? Object.fromEntries(entries) : undefined);
        return;
      }
      const arr = seqOf(node, op.field, tx);
      if (index + remove > arr.length) throw new TreeOpError(`index ${index} out of bounds (len ${arr.length}) at ${fmt(path)}`);
      const items = op.field === 'children' ? insert.map((v) => needNode(v, 'children', path)) : insert;
      tx.splice(arr, index, remove, items);
      return;
    }
    case 'append': {
      const { node } = nodeAt(holder, slot, path);
      const kind = kindOf(node);
      const ok = (kind === 'List' && op.field === 'items') || (kind === 'Table' && op.field === 'rows') || (kind === 'Log' && op.field === 'lines');
      if (!ok) throw new TreeOpError(`field "${op.field}" does not exist on ${kind} at ${fmt(path)}`);
      const arr = seqOf(node, op.field, tx);
      const items = Array.isArray(op.items) ? op.items : [];
      tx.splice(arr, arr.length, 0, items);
      // Ring trim (§4.8): older items drop silently past the limit.
      const limit = kind === 'Log' ? logLimit(node) : MAX_ITEMS;
      if (arr.length > limit) tx.splice(arr, 0, arr.length - limit, []);
      return;
    }
    case 'item': {
      const { node } = nodeAt(holder, slot, path);
      needField(node, op.field, path);
      if (op.field === 'panes') {
        if (!isObj(node.panes) || !(op.key in node.panes)) throw new TreeOpError(`missing key "${op.key}" in panes at ${fmt(path)}`);
        tx.set(node.panes, op.key, needNode(op.value, 'panes', path));
        return;
      }
      const arr = Array.isArray(node[op.field]) ? (node[op.field] as unknown[]) : [];
      let i =
        op.field === 'children'
          ? arr.findIndex((c, index) => derivedKey(c, index) === op.key)
          : arr.findIndex((c) => explicitKey(op.field, c) === op.key);
      if (i < 0 && op.key.startsWith('#')) {
        const n = Number(op.key.slice(1));
        if (Number.isInteger(n) && n >= 0 && n < arr.length) i = n;
      }
      if (i < 0) throw new TreeOpError(`missing key "${op.key}" in ${op.field} at ${fmt(path)}`);
      tx.splice(arr, i, 1, [op.field === 'children' ? needNode(op.value, 'children', path) : op.value]);
      return;
    }
    default:
      throw new TreeOpError(`unknown op "${(op as { op?: unknown }).op}"`);
  }
}

/**
 * Applies `ops` to `holder[slot]` atomically: on the first op that doesn't
 * resolve everything is rolled back and the error rethrown.
 */
export function applyNodeOps(holder: Obj, slot: string, ops: readonly NodeOp[], tx = new Tx()): void {
  try {
    for (const op of ops) applyNodeOp(holder, slot, op, tx);
  } catch (error) {
    tx.rollback();
    throw error;
  }
}
