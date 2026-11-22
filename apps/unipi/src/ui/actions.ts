/**
 * Actions (ui-protocol §4.6, §9.3): interactive nodes call `send` →
 * `_oino/ui_action{entryId, actionId, value?, values?, path?}` over the
 * session stream (`SessionCmd`); visibility goes out as `_oino/ui_visible`.
 * `values` = the current value of every keyed `Input`/`Toggle`/`Select` in
 * the entry, collected for `onSubmit`, `Button` and `custom` submissions.
 */
import { createContext, onCleanup, useContext } from 'solid-js';
import { sessions } from '../lib/session/store';
import type { NodePath, Scope } from './types';

/**
 * Where a registry entry renders. `turnEntryId` is set only by a row that lives
 * inside one turn (`turnFooter`, §6.7): the native component reads it as the
 * turn's id, exactly the `turnEntryId` the render request carries (A27).
 */
export type UiTarget = { scope: Scope; hostId: string; sessionId?: string; turnEntryId?: string };

export type UiActionPayload = { entryId: string; actionId: string; value?: unknown; values?: Record<string, unknown>; path?: NodePath };

/** Where frontend → core UI commands go. Tests and the gallery install their own. */
export type UiTransport = {
  action(target: UiTarget, payload: UiActionPayload): Promise<unknown>;
  visible(target: UiTarget, entryId: string, visible: boolean): Promise<unknown>;
  resync(target: UiTarget, rev: number): Promise<unknown>;
};

const sessionCmd = (target: UiTarget, cmd: { type: string; [k: string]: unknown }) => {
  if (target.scope !== 'session' || !target.sessionId) return Promise.resolve(undefined);
  const handle = sessions().peek(target.hostId, target.sessionId);
  return handle ? handle.cmd(cmd) : Promise.resolve(undefined);
};

/**
 * Default transport: session scope over the open session stream. Host-scope
 * entries (reserved screens, settings) carry no actions today (§9.3); their
 * resync is the hub stream's first snapshot after reopen, so these resolve
 * to nothing.
 */
export const defaultTransport: UiTransport = {
  action: (target, p) => sessionCmd(target, { type: '_oino/ui_action', ...p }),
  visible: (target, entryId, visible) => sessionCmd(target, { type: '_oino/ui_visible', entryId, visible }),
  resync: (target, rev) => sessionCmd(target, { type: '_oino/ui_resync', scope: target.scope, rev }),
};

let transport: UiTransport = defaultTransport;

export function setUiTransport(next: UiTransport | undefined): void {
  transport = next ?? defaultTransport;
}

export function uiTransport(): UiTransport {
  return transport;
}

/** Per-entry action context: the target plus the keyed form values registry. */
export type EntryActions = {
  readonly entryId: string;
  readonly target: UiTarget;
  /** Fires an action. `collect` attaches the entry's form `values` (submit-style actions). */
  send(actionId: string | undefined, value?: unknown, path?: NodePath, collect?: boolean): void;
  /** Registers a keyed form control's current value; returns the unregister. */
  registerValue(key: string, read: () => unknown): () => void;
  values(): Record<string, unknown>;
  /** Reports the entry's visibility to core (§6.5). */
  visible(visible: boolean): void;
};

export type ActionHook = (payload: UiActionPayload) => void;

export function createEntryActions(entryId: string, target: UiTarget, hook?: ActionHook): EntryActions {
  const readers = new Map<string, () => unknown>();
  const values = () => {
    const out: Record<string, unknown> = {};
    for (const [k, read] of readers) out[k] = read();
    return out;
  };
  return {
    entryId,
    target,
    send(actionId, value, path, collect) {
      if (!actionId) return;
      const payload: UiActionPayload = { entryId, actionId };
      if (value !== undefined) payload.value = value;
      if (collect && readers.size) payload.values = values();
      if (path && path.length) payload.path = [...path];
      hook?.(payload);
      void transport.action(target, payload).catch((error) => console.warn('[ui] action failed', actionId, error));
    },
    registerValue(key, read) {
      readers.set(key, read);
      return () => {
        if (readers.get(key) === read) readers.delete(key);
      };
    },
    values,
    visible(visible) {
      void transport.visible(target, entryId, visible).catch(() => {});
    },
  };
}

export const EntryContext = createContext<EntryActions>();

/** The enclosing entry's actions; a detached no-op outside an entry (gallery fragments, tests). */
export function useEntry(): EntryActions {
  return useContext(EntryContext) ?? DETACHED;
}

const DETACHED: EntryActions = createEntryActions('detached', { scope: 'session', hostId: '' });

/** Registers a keyed control's value with the entry for the lifetime of the component. */
export function useFormValue(key: string | undefined, read: () => unknown): void {
  if (!key) return;
  const entry = useEntry();
  onCleanup(entry.registerValue(key, read));
}
