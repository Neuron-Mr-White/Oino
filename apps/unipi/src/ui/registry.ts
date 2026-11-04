/**
 * UI entry registry (ui-protocol §6, §9.2, §13.1–13.2): one Solid store per
 * scope (`session` per open session, `host` per paired host) holding
 * `{rev, entries, dialogs, meta}`.
 *
 * - `applySnapshot` reconciles entries by id (unchanged subtrees keep their
 *   identity, so mounted nodes don't re-render).
 * - `applyPatch` checks `rev = last + 1`, applies every op inside one
 *   `produce` with an undo log — a patch that doesn't resolve is rolled back
 *   whole — and requests `_oino/ui_resync{scope, rev}` on a gap or failure.
 * - Slot selectors sort `order` desc, then `seq` asc (§6.4); stack slots put
 *   the most recently opened (seq) or focused entry on top.
 * - One-shot `effect`s and `invalidate`s are handed to listeners, never kept.
 *
 * Mirrors `oino-view`'s `RegistryMirror` (WP-U5) on the shared
 * `crates/oino-ui/fixtures/registry/*.json` corpus.
 */
import { batch, createMemo, createRoot, getOwner, runWithOwner, type Accessor } from 'solid-js';
import { createStore, produce, reconcile, unwrap } from 'solid-js/store';
import { isEnvelopeError, parsePatch, parseSnapshot } from './envelope';
import { dropLocal } from './localState';
import { entryShownInApp } from '../lib/surfaces';
import { Tx, applyNodeOps } from './tree';
import type { RegistryOp, Scope, Slot, UiEntry, UiMeta, UiPatch, UiRequest, UiSnapshot } from './types';

export type RegistryState = {
  rev: number;
  sessionId?: string | null;
  entries: Record<string, UiEntry>;
  dialogs: UiRequest[];
  meta: UiMeta;
  /** A resync is in flight: patches are dropped until the next snapshot. */
  resyncing: boolean;
};

export type Invalidation = { id: string; turns?: string[] };

export type PatchOutcome = { ok: true; effects: UiRequest[]; invalidations: Invalidation[] } | { ok: false; error: string; resync: boolean };

export type RegistryHooks = {
  /** Asks core for a fresh snapshot (`_oino/ui_resync`); the reply goes to `applySnapshot`. */
  resync?: (rev: number) => void;
  onEffect?: (request: UiRequest) => void;
  onInvalidate?: (inv: Invalidation) => void;
};

/** pi + Oino dialog methods (v1 §4.3); other `extension_ui_request`s are retained producers core already mirrored. */
export const DIALOG_METHODS = new Set([
  'select', 'confirm', 'input', 'editor', 'multiSelect', 'form', 'approval', 'custom', 'clipboardReadImage', 'editExternally',
]);

/** Slots that show one entry at a time (§6.4). */
export const STACK_SLOTS: ReadonlySet<Slot> = new Set(['overlay', 'editorReplace']);

export function compareEntries(a: UiEntry, b: UiEntry): number {
  return (b.order ?? 0) - (a.order ?? 0) || (a.seq ?? 0) - (b.seq ?? 0);
}

export type UiRegistry = ReturnType<typeof createUiRegistry>;

export function createUiRegistry(scope: Scope, hooks: RegistryHooks = {}) {
  return createRoot((dispose) => {
    const [state, setState] = createStore<RegistryState>({ rev: 0, sessionId: null, entries: {}, dialogs: [], meta: {}, resyncing: false });
    const slotMemos = new Map<Slot, Accessor<UiEntry[]>>();
    const stackMemos = new Map<Slot, Accessor<UiEntry[]>>();
    // Slot memos must belong to this root, never to the first caller's
    // computation (a memo parented by a caller is disposed when that caller
    // re-runs — the Map would keep a frozen memo forever). U10 hit this with
    // hosts rendering `entriesFor(...)` inside components.
    const self = getOwner()!;

    const requestResync = (rev: number) => {
      if (state.resyncing) return;
      setState('resyncing', true);
      hooks.resync?.(rev);
    };

    const applySnapshot = (raw: UiSnapshot | unknown): { ok: boolean; error?: string } => {
      const snap = parseSnapshot(raw);
      if (isEnvelopeError(snap)) return { ok: false, error: snap.error };
      if (snap.scope !== scope) return { ok: false, error: `scope ${snap.scope} ≠ ${scope}` };
      const byId: Record<string, UiEntry> = {};
      for (const e of snap.entries) byId[e.id] = e;
      const removed = Object.keys(unwrap(state.entries)).filter((id) => !(id in byId));
      batch(() => {
        setState('entries', reconcile(byId, { key: 'key', merge: true }));
        setState('dialogs', reconcile(snap.dialogs, { key: 'id' }));
        setState('meta', reconcile(snap.meta ?? {}));
        setState({ rev: snap.rev, sessionId: snap.sessionId ?? null, resyncing: false });
      });
      for (const id of removed) dropLocal(id);
      return { ok: true };
    };

    const applyPatch = (raw: UiPatch | unknown): PatchOutcome => {
      const patch = parsePatch(raw);
      if (isEnvelopeError(patch)) {
        requestResync(state.rev);
        return { ok: false, error: patch.error, resync: true };
      }
      if (patch.scope !== scope) return { ok: false, error: `scope ${patch.scope} ≠ ${scope}`, resync: false };
      if (state.resyncing) return { ok: false, error: 'resync in flight', resync: false };
      if (patch.rev !== state.rev + 1) {
        requestResync(state.rev);
        return { ok: false, error: `rev gap: expected ${state.rev + 1}, got ${patch.rev}`, resync: true };
      }
      const effects: UiRequest[] = [];
      const invalidations: Invalidation[] = [];
      const deleted: string[] = [];
      let error: string | undefined;
      batch(() => {
        setState(
          produce((s) => {
            const tx = new Tx();
            try {
              for (const op of patch.ops) applyOp(s, op, tx, effects, invalidations, deleted);
              tx.set(s as unknown as Record<string, unknown>, 'rev', patch.rev);
            } catch (e) {
              tx.rollback();
              error = e instanceof Error ? e.message : String(e);
            }
          }),
        );
      });
      if (error !== undefined) {
        requestResync(state.rev);
        return { ok: false, error, resync: true };
      }
      for (const id of deleted) if (!(id in state.entries)) dropLocal(id);
      for (const e of effects) hooks.onEffect?.(e);
      for (const inv of invalidations) hooks.onInvalidate?.(inv);
      return { ok: true, effects, invalidations };
    };

    /** pi `extension_ui_request` / `extension_ui_dismiss` (dialogs only, §9.4). */
    const applyRequest = (msg: Record<string, unknown>) => {
      if (msg.type === 'extension_ui_dismiss') {
        const id = String(msg.id ?? '');
        setState('dialogs', (d) => d.filter((x) => x.id !== id));
        return;
      }
      const method = typeof msg.method === 'string' ? msg.method : '';
      if (!DIALOG_METHODS.has(method) || msg.id === undefined) return;
      const { type: _type, ...rest } = msg;
      const request = { ...rest, id: String(msg.id), method } as UiRequest;
      setState(
        'dialogs',
        produce((d) => {
          const i = d.findIndex((x) => x.id === request.id);
          if (i >= 0) d[i] = request;
          else d.push(request);
        }),
      );
    };

    const entriesFor = (slot: Slot): Accessor<UiEntry[]> => {
      let memo = slotMemos.get(slot);
      if (!memo) {
        // A `meta` op mutates the entry proxy in place and leaves the key set
        // alone, so the selector must track the per-entry fields it sorts and
        // its hosts read (`meta` bumps `rev`); an identity comparator or a
        // keys-only read would swallow title/badge/order/focus updates (U10:
        // pane tabs never retitled, the strip never retoned). `For` keyed by
        // item identity still keeps the DOM as is; only the memo re-runs.
        memo = runWithOwner(self, () =>
          createMemo(() => {
            const all = Object.values(state.entries);
            for (const e of all) {
              void e.rev;
              void e.order;
              void e.seq;
              void e.focus;
            }
            // [ask-user] entries meant for the terminal only never reach a slot host here.
            return all.filter((e) => e.slot === slot && entryShownInApp(e)).sort(compareEntries);
          }),
        )!;
        slotMemos.set(slot, memo);
      }
      return memo;
    };

    /** Bottom → top: seq asc, focused entries last (§6.4 stack slots). */
    const stack = (slot: Slot): Accessor<UiEntry[]> => {
      let memo = stackMemos.get(slot);
      if (!memo) {
        memo = runWithOwner(self, () =>
          createMemo(() => {
            const all = Object.values(state.entries);
            for (const e of all) {
              void e.rev;
              void e.seq;
              void e.focus;
            }
            return all
              .filter((e) => e.slot === slot && entryShownInApp(e))
              .sort((a, b) => Number(!!a.focus) - Number(!!b.focus) || (a.seq ?? 0) - (b.seq ?? 0));
          }),
        )!;
        stackMemos.set(slot, memo);
      }
      return memo;
    };

    return {
      scope,
      state,
      applySnapshot,
      applyPatch,
      applyRequest,
      entriesFor,
      stack,
      stackTop: (slot: Slot) => {
        const s = stack(slot)();
        return s[s.length - 1];
      },
      entry: (id: string): UiEntry | undefined => state.entries[id],
      dialogs: (): UiRequest[] => state.dialogs,
      /** A resync request failed: the next patch may ask again. */
      endResync: () => setState('resyncing', false),
      /** Removes a dialog locally once answered (first answer wins; core confirms with `dismiss`). */
      dismissDialog: (id: string) => setState('dialogs', (d) => d.filter((x) => x.id !== id)),
      dispose: () => {
        for (const id of Object.keys(unwrap(state.entries))) dropLocal(id);
        dispose();
      },
    };
  });
}

type Draft = RegistryState;

function applyOp(s: Draft, op: RegistryOp, tx: Tx, effects: UiRequest[], invalidations: Invalidation[], deleted: string[]): void {
  const entries = s.entries as unknown as Record<string, unknown>;
  const need = (id: string): UiEntry => {
    const e = s.entries[id];
    if (!e) throw new Error(`missing entry ${id}`);
    return e;
  };
  switch (op.op) {
    case 'put':
      tx.set(entries, op.entry.id, op.entry);
      return;
    case 'del':
      need(op.id);
      tx.set(entries, op.id, undefined);
      deleted.push(op.id);
      return;
    case 'meta': {
      const e = need(op.id) as unknown as Record<string, unknown>;
      for (const [k, v] of Object.entries(op.set ?? {})) if (v !== undefined) tx.set(e, k, v === null ? undefined : v);
      // `meta` carries no rev on the wire; bump it so caches keyed by rev refresh.
      tx.set(e, 'rev', ((e.rev as number) ?? 0) + 1);
      return;
    }
    case 'tree': {
      const e = need(op.id);
      if (e.view?.kind !== 'tree') throw new Error(`entry ${op.id} is not a tree`);
      try {
        applyNodeOps(e.view as unknown as Record<string, unknown>, 'root', op.ops ?? [], tx);
      } catch (error) {
        throw new Error(`entry ${op.id}: ${error instanceof Error ? error.message : String(error)}`);
      }
      tx.set(e as unknown as Record<string, unknown>, 'rev', op.rev);
      return;
    }
    case 'invalidate':
      invalidations.push(op.turns ? { id: op.id, turns: [...op.turns] } : { id: op.id });
      return;
    case 'dialog': {
      const i = s.dialogs.findIndex((d) => d.id === op.request.id);
      if (i >= 0) tx.set(s.dialogs as unknown as Record<string, unknown>, String(i), op.request);
      else tx.splice(s.dialogs, s.dialogs.length, 0, [op.request]);
      return;
    }
    case 'dismiss': {
      const i = s.dialogs.findIndex((d) => d.id === op.id);
      if (i >= 0) tx.splice(s.dialogs, i, 1, []);
      return;
    }
    case 'sessionMeta': {
      const meta = s.meta as unknown as Record<string, unknown>;
      for (const [k, v] of Object.entries(op.set ?? {})) if (v !== undefined) tx.set(meta, k, v === null ? undefined : v);
      return;
    }
    case 'effect':
      effects.push(op.request);
      return;
  }
}
