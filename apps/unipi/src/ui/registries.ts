/**
 * Registry instances fed by the host streams (ui-protocol §13.2 step 1; app
 * spec §3.3 "ui registry: per session + per host scope"):
 *
 * - session scope: `SessionSnapshot.ui` → `oino_ui_snapshot`, `oino_ui_patch`
 *   events, `extension_ui_request` / `_dismiss` — routed here by the session
 *   store's ui sink (`lib/session/store.ts` `setSessionUiSink`, WP-A2);
 * - host scope: `HubEvent::Ui{snapshot|patch}` from `events_open` via the hub
 *   store's sink (`setHubUiSink`).
 *
 * Session registries follow the session store's LRU (kept for twice its
 * size so a quick reopen keeps local state); `installUiSinks` is idempotent.
 */
import { setHubUiSink } from '../lib/session/hub';
import { SESSION_LRU, setSessionUiSink } from '../lib/session/store';
import { uiTransport, type UiTarget } from './actions';
import { createUiRegistry, type Invalidation, type UiRegistry } from './registry';
import type { UiEntry, UiRequest } from './types';

type Listener<T> = (target: UiTarget, value: T) => void;

const sessionRegs = new Map<string, UiRegistry>();
const hostRegs = new Map<string, UiRegistry>();
const effectListeners = new Set<Listener<UiRequest>>();
const invalidateListeners = new Set<Listener<Invalidation>>();
const KEEP = SESSION_LRU * 2;

/** The snapshot inside a resync reply (`UiSnapshot`, or wrapped as `{snapshot}` / `{ui}`). */
function snapshotOf(reply: unknown): unknown {
  if (reply && typeof reply === 'object') {
    const r = reply as Record<string, unknown>;
    if ('rev' in r && 'entries' in r) return r;
    if (r.snapshot) return r.snapshot;
    if (r.ui) return r.ui;
  }
  return undefined;
}

function make(target: UiTarget): UiRegistry {
  const reg: UiRegistry = createUiRegistry(target.scope, {
    resync: (rev) => {
      uiTransport()
        .resync(target, rev)
        .then((reply) => {
          const snap = snapshotOf(reply);
          if (snap) reg.applySnapshot(snap);
          else reg.endResync();
        })
        .catch(() => reg.endResync());
    },
    onEffect: (req) => effectListeners.forEach((l) => l(target, req)),
    onInvalidate: (inv) => invalidateListeners.forEach((l) => l(target, inv)),
  });
  return reg;
}

/** Looks an entry up across the open registries (U10's `/x/:entryId` page, `ext` tabs). */
export function findUiEntry(target: UiTarget, entryId: string): UiEntry | undefined {
  const reg = target.scope === 'session' && target.sessionId ? sessionRegs.get(`${target.hostId}\u0000${target.sessionId}`) : target.scope === 'host' ? hostRegs.get(target.hostId) : undefined;
  return reg?.entry(entryId);
}

/** The entry + its target wherever an open registry holds it (the `/x/:id` page's lookup). */
export function findUiEntryAnywhere(entryId: string): { entry: UiEntry; target: UiTarget } | undefined {
  for (const [key, reg] of sessionRegs) {
    const entry = reg.entry(entryId);
    if (entry) {
      const [hostId, sessionId] = key.split('\u0000');
      return { entry, target: { scope: 'session', hostId, sessionId } };
    }
  }
  for (const [hostId, reg] of hostRegs) {
    const entry = reg.entry(entryId);
    if (entry) return { entry, target: { scope: 'host', hostId } };
  }
  return undefined;
}

/** The session-scope registry of `(hostId, sessionId)` (created on first use). */
export function sessionUi(hostId: string, sessionId: string): UiRegistry {
  const key = `${hostId}\u0000${sessionId}`;
  let reg = sessionRegs.get(key);
  if (reg) {
    sessionRegs.delete(key);
    sessionRegs.set(key, reg);
    return reg;
  }
  reg = make({ scope: 'session', hostId, sessionId });
  sessionRegs.set(key, reg);
  while (sessionRegs.size > KEEP) {
    const [oldest, old] = sessionRegs.entries().next().value as [string, UiRegistry];
    sessionRegs.delete(oldest);
    old.dispose();
  }
  return reg;
}

/** The host-scope registry of `hostId` (reserved screens, settings namespaces). */
export function hostUi(hostId: string): UiRegistry {
  let reg = hostRegs.get(hostId);
  if (!reg) hostRegs.set(hostId, (reg = make({ scope: 'host', hostId })));
  return reg;
}

/** One-shot effects (`notify`, `openUrl`, `set_editor_text`, …) for U10's handlers. */
export function onUiEffect(listener: Listener<UiRequest>): () => void {
  effectListeners.add(listener);
  return () => effectListeners.delete(listener);
}

/** turnFooter cache invalidations (§6.7) for the transcript. */
export function onUiInvalidate(listener: Listener<Invalidation>): () => void {
  invalidateListeners.add(listener);
  return () => invalidateListeners.delete(listener);
}

/** Routes every session-stream ui message (`oino_ui_snapshot` / `oino_ui_patch` / `extension_ui_*`). */
export function routeSessionUi(hostId: string, sessionId: string, msg: Record<string, unknown>): void {
  const reg = sessionUi(hostId, sessionId);
  switch (msg.type) {
    case 'oino_ui_snapshot': {
      const { type: _t, ...snap } = msg;
      reg.applySnapshot({ scope: 'session', ...snap });
      return;
    }
    case 'oino_ui_patch': {
      const { type: _t, ...patch } = msg;
      reg.applyPatch({ scope: 'session', ...patch });
      return;
    }
    case 'extension_ui_request':
    case 'extension_ui_dismiss':
      reg.applyRequest(msg);
      return;
  }
}

let installed = false;

/** Points the session and hub stores' ui sinks at these registries (idempotent). */
export function installUiSinks(): void {
  if (installed) return;
  installed = true;
  setSessionUiSink(routeSessionUi);
  setHubUiSink((hostId, event) => {
    const reg = hostUi(hostId);
    if (event.snapshot) reg.applySnapshot({ ...event.snapshot, scope: 'host' });
    if (event.patch) reg.applyPatch({ ...event.patch, scope: 'host' });
  });
}

/** Drops every registry and unhooks the sinks (tests). */
export function resetUiRegistries(): void {
  for (const r of sessionRegs.values()) r.dispose();
  for (const r of hostRegs.values()) r.dispose();
  sessionRegs.clear();
  hostRegs.clear();
  effectListeners.clear();
  invalidateListeners.clear();
  if (installed) {
    setSessionUiSink(undefined);
    setHubUiSink(undefined);
    installed = false;
  }
}
