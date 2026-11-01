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
import { copyText, openExternal } from '../../lib/board/share';
import { composerDraft, setComposerText } from '../../lib/composer/drafts';
import { appendTokenText, insertEditorToken, parseEditorToken } from '../../lib/composer/tokens';
import { desktopNotifyOn, nativeNotify } from '../../lib/desktop';
import { can } from '../../lib/platform';
import { sessions } from '../../lib/session/store';
import { openSettingsFor } from '../../lib/settings/open';
import { toasts } from '../../lib/toasts';
import { routeNotify } from '../../lib/reports';
import { onUiEffect, onUiInvalidate } from '../registries';
import type { UiTarget } from '../actions';
import type { Invalidation } from '../registry';

/** `notifyType` (pi) → the toast stack's kind. */
export function toastKindOf(notifyType: unknown): 'info' | 'warning' | 'error' {
  return notifyType === 'error' ? 'error' : notifyType === 'warning' || notifyType === 'warn' ? 'warning' : 'info';
}

let osNotifySeq = 2000;

function sessionKeyFor(target: UiTarget): string | undefined {
  if (target.scope !== 'session' || !target.sessionId) return undefined;
  return sessions().peek(target.hostId, target.sessionId)?.key ?? `${target.hostId}/${target.sessionId}`;
}

function applyEffect(target: UiTarget, req: { method: string; [k: string]: unknown }): void {
  switch (req.method) {
    case 'notify': {
      const message = typeof req.message === 'string' ? req.message : '';
      // P1-02: a multi-line / long report opens the report panel, never a 3 s toast.
      if (message) routeNotify(message, toastKindOf(req.notifyType), sessionKeyFor(target));
      return;
    }
    case 'set_editor_text': {
      // Only the session scope has a composer; text lands in its draft store
      // (undoable, A12), exactly as if the user typed it.
      const key = sessionKeyFor(target);
      if (key) setComposerText(key, typeof req.text === 'string' ? req.text : '');
      return;
    }
    case 'insertEditorText':
    case 'insert_editor_text': {
      const key = sessionKeyFor(target);
      const text = typeof req.text === 'string' ? req.text : '';
      if (key && text) setComposerText(key, appendTokenText(composerDraft(key).text, text));
      return;
    }
    case 'insertEditorToken':
    case 'insert_editor_token': {
      const key = sessionKeyFor(target);
      if (!key) return;
      const token = parseEditorToken(req);
      if (token) insertEditorToken(key, token);
      return;
    }
    case 'openSettings':
    case 'open_settings': {
      openSettingsFor({
        namespace: typeof req.namespace === 'string' ? req.namespace : undefined,
        section: typeof req.section === 'string' ? req.section : undefined,
        query: typeof req.query === 'string' ? req.query : undefined,
      });
      return;
    }
    case 'openUrl':
    case 'open_url': {
      const url = typeof req.url === 'string' ? req.url.trim() : '';
      if (!url) return;
      if (!openExternal(url) && /^(https?|mailto):/i.test(url)) {
        void copyText(url).then((ok) => {
          if (ok) toasts().push('Link copied to clipboard', 'info');
        });
      }
      return;
    }
    case 'clipboardWrite':
    case 'clipboard_write': {
      if (typeof req.text === 'string') void copyText(req.text);
      return;
    }
    case 'osNotify':
    case 'os_notify': {
      const rawTitle = typeof req.title === 'string' ? req.title.trim() : '';
      const body = typeof req.body === 'string' ? req.body : typeof req.message === 'string' ? req.message : '';
      if (!rawTitle && !body) return;
      const title = rawTitle || 'UniPi';
      if (can('nativeNotifications')) {
        if (desktopNotifyOn()) void nativeNotify(title, body);
        return;
      }
      const bridge = (window as unknown as { UniPiNotify?: { show?: (id: number, title: string, body: string, ongoing: boolean, actionsJson: string | null) => void } }).UniPiNotify;
      bridge?.show?.(++osNotifySeq, title, body, false, null);
      return;
    }
    default:
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
