/**
 * One-shot ui effects + registry meta the app applies app-wide (WP-U10,
 * ui-protocol §3.2 "Rendering"): `notify` → the one toast stack (A19),
 * `set_editor_text` → the composer draft (undoable there), turnFooter
 * invalidations (§6.7) → the per-entry footer rev the transcript caches by,
 * and `setWorkingMessage` → the live fold's working text.
 *
 * Installed once per app (`installUiEffects`, idempotent — Conversation's
 * module scope calls it beside `installUiSinks`); every feed is keyed by the
 * UiTarget it arrived from, so hosts/sessions never cross wires.
 */
import { batch, createSignal } from 'solid-js';
import { toasts } from '../../lib/toasts';
import { setComposerText } from '../../lib/composer/drafts';
import { sessions } from '../../lib/session/store';
import { onUiEffect, onUiInvalidate } from '../registries';
import type { UiTarget } from '../actions';
import type { Invalidation } from '../registry';

/** `notifyType` (pi) → the toast stack's kind. */
export function toastKindOf(notifyType: unknown): 'info' | 'warning' | 'error' {
  return notifyType === 'error' ? 'error' : notifyType === 'warning' || notifyType === 'warn' ? 'warning' : 'info';
}

function applyEffect(target: UiTarget, req: { method: string; [k: string]: unknown }): void {
  switch (req.method) {
    case 'notify': {
      const message = typeof req.message === 'string' ? req.message : '';
      if (message) toasts().push(message, toastKindOf(req.notifyType));
      return;
    }
    case 'set_editor_text': {
      // Only the session scope has a composer; text lands in its draft store
      // (undoable, A12), exactly as if the user typed it.
      if (target.scope !== 'session' || !target.sessionId) return;
      const handle = sessions().peek(target.hostId, target.sessionId);
      if (handle) setComposerText(handle.key, typeof req.text === 'string' ? req.text : '');
      return;
    }
    default:
      // `openUrl` and friends have no app surface yet (degradation table §10).
      return;
  }
}

// ── turnFooter invalidations (§6.7): rev bumps the transcript cache keys ──

const [footerRevs, setFooterRevs] = createSignal<Record<string, number>>({});

/** The current invalidation rev of a turnFooter entry (cache key part). */
export function footerRev(entryId: string): number {
  return footerRevs()[entryId] ?? 0;
}

function applyInvalidate(inv: Invalidation): void {
  batch(() => setFooterRevs((cur) => ({ ...cur, [inv.id]: (cur[inv.id] ?? 0) + 1 })));
}

// ── working message (§3.2 `setWorkingMessage`): the live fold's text ─────

const [working, setWorking] = createSignal<Record<string, string | undefined>>({});

/** The session's working message (`undefined` ⇒ the default "Working for"). */
export function workingMessageOf(sessionKey: string): string | undefined {
  return working()[sessionKey];
}

/** Publishes (or clears) a session's working message; returns the clear. */
export function publishWorkingMessage(sessionKey: string, message: string | undefined): () => void {
  setWorking((cur) => ({ ...cur, [sessionKey]: message }));
  return () =>
    setWorking((cur) => {
      if (cur[sessionKey] !== message) return cur;
      return { ...cur, [sessionKey]: undefined };
    });
}

let installed = false;

/** Wires the app-wide effect/invalidate handlers (idempotent). */
export function installUiEffects(): void {
  if (installed) return;
  installed = true;
  const offEffect = onUiEffect((target, req) => applyEffect(target, req));
  const offInvalidate = onUiInvalidate((_target, inv) => applyInvalidate(inv));
  if (!offEffect || !offInvalidate) return; // keeps `installed` true: listeners are global
  // The listeners live for the app's lifetime; no teardown in production.
}

/** Test seam: drop the handlers and revs. */
export function resetUiEffects(): void {
  installed = false;
  setFooterRevs({});
  setWorking({});
}
