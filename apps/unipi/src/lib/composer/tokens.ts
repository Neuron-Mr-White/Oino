/**
 * Editor token insertion API (P0-08, ui-protocol `insertEditorToken`):
 * bridges UI v2 `insertEditorToken` effects into a session's composer without
 * coupling `ui/slots/effects.ts` to `components/composer/**`.
 *
 * - Live composers subscribe per `sessionKey` (`hostId/sessionId` or
 *   `draft:hostId`) via `subscribeEditorTokens` / `connectEditorTokens` so
 *   text/path tokens land at the live caret and record an undo step.
 * - When no live subscriber is mounted for `sessionKey`, `applyEditorToken`
 *   writes directly to the session's reactive draft/attachment stores
 *   (`drafts.ts` / `attachments.ts`) so nothing is ever dropped.
 */
import { createRenderEffect, onCleanup } from 'solid-js';
import { addReadyImage } from './attachments';
import { addComposerChip, composerDraft, focusComposer, setComposerText } from './drafts';

export type EditorTokenImage = { mimeType: string; data: string };

export type EditorToken = {
  tokenId?: string;
  /** `image` | `file` | `text` (or custom chip kind). */
  kind: string;
  label: string;
  data?: EditorTokenImage;
  path?: string;
  markdown?: string;
};

export type EditorTokenListener = (token: EditorToken) => void;

const listeners = new Map<string, Set<EditorTokenListener>>();

const optStr = (v: unknown): string | undefined => (typeof v === 'string' && v.length > 0 ? v : undefined);

function parseImageData(raw: unknown): EditorTokenImage | undefined {
  if (!raw || typeof raw !== 'object') return undefined;
  const r = raw as Record<string, unknown>;
  const data = optStr(r.data);
  if (!data) return undefined;
  const mimeType = optStr(r.mimeType) ?? optStr(r.mime) ?? 'image/png';
  return { mimeType, data };
}

/** Formats a path token the same way `@` mentions do (quoted when containing whitespace/quotes). */
export function formatPathToken(path: string): string {
  const clean = path.startsWith('@') ? path.slice(1) : path;
  const token = /[\s"']/.test(clean) ? `@"${clean.replace(/^"|"$/g, '')}"` : `@${clean}`;
  return `${token} `;
}

/** Appends `addition` to `base` with a separating space when `base` doesn't end in whitespace. */
export function appendTokenText(base: string, addition: string): string {
  if (!addition) return base;
  if (!base) return addition;
  const needsSpace = !/\s$/.test(base) && !/^\s/.test(addition);
  return `${base}${needsSpace ? ' ' : ''}${addition}`;
}

/** Parses an `insertEditorToken` UI request (`{token: {...}}` or flat fields). */
export function parseEditorToken(req: Record<string, unknown>): EditorToken | undefined {
  const src = req.token && typeof req.token === 'object' && !Array.isArray(req.token) ? (req.token as Record<string, unknown>) : req;
  const kind = optStr(src.kind) ?? 'text';
  const label = typeof src.label === 'string' ? src.label : '';
  const tokenId = optStr(src.tokenId) ?? optStr(src.id);
  const path = optStr(src.path);
  const markdown = optStr(src.markdown);
  const data = parseImageData(src.data);
  if (!label && !path && !data && !markdown) return undefined;
  return {
    ...(tokenId ? { tokenId } : {}),
    kind,
    label,
    ...(data ? { data } : {}),
    ...(path ? { path } : {}),
    ...(markdown ? { markdown } : {}),
  };
}

/**
 * Applies `token` to `sessionKey`'s composer stores (`attachments.ts` for
 * images, `drafts.ts` for chips/text). When `insertText` is supplied by a
 * mounted composer, text/path tokens are inserted at the live caret instead of
 * appended to the end of the draft.
 */
export function applyEditorToken(sessionKey: string, token: EditorToken, insertText?: (text: string) => void): void {
  if (token.kind === 'image' && token.data) {
    addReadyImage(sessionKey, {
      name: token.label || undefined,
      mime: token.data.mimeType,
      data: token.data.data,
    });
    focusComposer(sessionKey);
    return;
  }
  if (token.markdown) {
    addComposerChip(sessionKey, {
      ...(token.tokenId ? { id: token.tokenId } : {}),
      kind: token.kind || 'token',
      label: token.label || 'Token',
      markdown: token.markdown,
    });
    focusComposer(sessionKey);
    return;
  }
  const text = token.path ? formatPathToken(token.path) : token.label;
  if (!text) return;
  if (insertText) {
    insertText(text);
  } else {
    const cur = composerDraft(sessionKey).text;
    setComposerText(sessionKey, appendTokenText(cur, text));
  }
  focusComposer(sessionKey);
}

/** Dispatches `token` to `sessionKey`'s subscribed composer, or applies it directly to its draft store. */
export function insertEditorToken(sessionKey: string, token: EditorToken): void {
  const subs = listeners.get(sessionKey);
  if (subs && subs.size > 0) {
    for (const fn of subs) fn(token);
    return;
  }
  applyEditorToken(sessionKey, token);
}

/** Subscribes to `insertEditorToken` events for `sessionKey`; returns the unsubscribe callback. */
export function subscribeEditorTokens(sessionKey: string, listener: EditorTokenListener): () => void {
  const set = listeners.get(sessionKey) ?? new Set<EditorTokenListener>();
  set.add(listener);
  listeners.set(sessionKey, set);
  return () => {
    set.delete(listener);
    if (!set.size) listeners.delete(sessionKey);
  };
}

/**
 * One-line reactive hook for `Composer.tsx`:
 * `connectEditorTokens(key, (t) => commit(appendTokenText(draft().text, t), ...))`
 */
export function connectEditorTokens(sessionKey: () => string, insertText: (text: string) => void): void {
  createRenderEffect(() => {
    const k = sessionKey();
    onCleanup(subscribeEditorTokens(k, (token) => applyEditorToken(k, token, insertText)));
  });
}

/** Test seam: clears registered token listeners. */
export function resetEditorTokens(): void {
  listeners.clear();
}
