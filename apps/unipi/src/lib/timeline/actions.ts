import { toasts } from '../toasts';
/**
 * Turn actions (app spec WP-A7, Z-17/Z-26/Z-27/Z-28/Z-49/Z-93): the pure
 * helpers behind the turn tail, plus the two small stores the actions need —
 * the composer handoff (text/quotes/images the composer shows) and the
 * message / edit menus (one instance per timeline).
 *
 * The composer's draft store lives in `lib/composer/drafts.ts` (WP-A12);
 * everything the actions put there goes through this module's re-exports so
 * the timeline never imports the composer's internals:
 *
 *   tail Quote / selection Quote / swipe Quote → quoteIntoComposer()
 *   user bubble Edit / row menu Edit          → editMessage() (tree jump)
 *   last-turn Retry                          → retryTurn() (tree jump + prompt)
 *   tail Fork / row menu Fork                → forkFrom() (`session_fork`)
 *
 * File changes (Z-28's "Also undo file changes") stay hidden until WP-A27
 * provides `TimelineCtx.undoFiles`; nothing here guesses at the extension.
 */
import { createSignal } from 'solid-js';
import { copyText } from '../board/share';
import { desktopShell, menuAnchor } from '../layout';
import { imagesOf, textOf, type Part } from '../chat/model';
import type { SessionEntry, SessionState, OutboxImage } from '../session/types';
import type { SessionHandle } from '../session/store';
import type { Turn } from '../session/turns';
import { addComposerQuote, focusComposer, restoreComposer } from '../composer/drafts';
import { hostClient } from '../host/factory';

/* ── turn text ──────────────────────────────────────────────────── */

/** A message entry's plain text (quote / edit / retry payloads). */
export const messageText = (entry: SessionEntry | undefined): string => textOf(entry?.message?.content as string | Part[] | undefined);
const entryText = messageText;

/** A user message's images, in the outbox shape the composer sends. */
export function entryImages(entry: SessionEntry | undefined): OutboxImage[] {
  return imagesOf(entry?.message?.content as string | Part[] | undefined).flatMap((image) =>
    image.mime && typeof image.data === 'string' && image.data ? [{ mimeType: image.mime, data: image.data }] : [],
  );
}

/** The settle time of a turn: its end, else its last entry, else its query. */
export function turnTime(turn: Turn): number | undefined {
  if (turn.endedAt !== undefined) return turn.endedAt;
  for (let i = turn.entries.length - 1; i >= 0; i--) {
    const ts = Date.parse(turn.entries[i]!.timestamp ?? '');
    if (Number.isFinite(ts)) return ts;
  }
  const userTs = Date.parse(turn.user?.timestamp ?? '');
  return Number.isFinite(userTs) ? userTs : undefined;
}

/** What Copy puts on the clipboard: the query, each steer and every answer of the turn. */
export function turnText(turn: Turn): string {
  const parts: string[] = [];
  const query = entryText(turn.user).trim();
  if (query) parts.push(query);
  for (const segment of turn.segments) {
    const steer = entryText(segment.steer).trim();
    if (steer) parts.push(steer);
    const answer = segment.answer?.text.trim();
    if (answer) parts.push(answer);
  }
  return parts.join('\n\n');
}

/** The answer a quote should carry: the last answer text, else the query. */
export function turnQuoteText(turn: Turn): string {
  for (let i = turn.segments.length - 1; i >= 0; i--) {
    const answer = turn.segments[i]!.answer?.text.trim();
    if (answer) return answer;
  }
  return entryText(turn.user).trim();
}

/** One line, whitespace collapsed, cut at `max` characters (tooltips, chips). */
export function excerpt(text: string, max = 140): string {
  const flat = text.replace(/\s+/g, ' ').trim();
  return flat.length > max ? `${flat.slice(0, max - 1).trimEnd()}…` : flat;
}

/** The settled entry id behind a prose item key (`<entryId>-<partIndex>`). */
export function entryIdOfKey(state: Pick<SessionState, 'byId'>, key: string | undefined): string | undefined {
  if (!key) return undefined;
  const id = key.slice(0, key.lastIndexOf('-'));
  return id && state.byId[id] !== undefined ? id : undefined;
}

/* ── composer handoff (the store lives in lib/composer/drafts.ts) ── */

export type { QuoteChip, ComposerChip, ComposerDraft } from '../composer/drafts';
export {
  composerDraft,
  setComposerText,
  addComposerQuote,
  removeComposerQuote,
  removeComposerImage,
  setComposerImages,
  restoreComposer,
  clearComposer,
  addComposerChip,
  removeComposerChip,
  composerSendText,
  registerComposerFocus,
  focusComposer,
} from '../composer/drafts';

/** Quote into the composer and put the caret there (selection + tail actions). */
export function quoteIntoComposer(key: string, text: string): void {
  if (!text.trim()) return;
  addComposerQuote(key, text);
  focusComposer(key);
}

/* ── transient action feedback (A19 owns real toasts) ─────────────── */

const [notice, setNotice] = createSignal<string | undefined>();
let noticeTimer: ReturnType<typeof setTimeout> | undefined;

export function actionNotice(): string | undefined {
  return notice();
}

/** Transient action feedback goes to the one toast stack (bottom-right, above the composer). */
export function showActionNotice(text: string): void {
  const kind = /^(could not|couldn't|failed|error)/i.test(text) ? 'error' : 'info';
  toasts().push(text, kind);
}

export function clearActionNotice(): void {
  if (noticeTimer) clearTimeout(noticeTimer);
  setNotice(undefined);
}

const describe = (error: unknown): string =>
  (error as { message?: unknown } | null)?.message !== undefined ? String((error as { message: unknown }).message) : String(error);

/* ── host actions ────────────────────────────────────────────────── */

type TreeReply = { cancelled?: boolean; editorText?: string };

/** pi `navigateTree`: move the leaf to `entryId` (a user entry ⇒ leaf = its parent). */
export async function treeJump(session: SessionHandle, entryId: string): Promise<void> {
  const reply = await session.cmd<TreeReply>({ type: '_oino/navigate_tree', targetId: entryId });
  if (reply?.cancelled) throw new Error('The jump was cancelled.');
}

/** Edit: tree-jump to the user message, then put its text + images back in the composer. */
export async function editMessage(session: SessionHandle, entry: SessionEntry): Promise<void> {
  await treeJump(session, entry.id);
  restoreComposer(session.key, { text: entryText(entry), images: entryImages(entry) });
  focusComposer(session.key);
}

/** Retry (Z-49): re-run a turn from its user message, on a fresh branch. */
export async function retryTurn(session: SessionHandle, turn: Turn): Promise<void> {
  const user = turn.user;
  if (!user) throw new Error('This turn has no message to retry from.');
  const text = entryText(user);
  const images = entryImages(user);
  if (!text.trim() && !images.length) throw new Error('This turn has no message to retry from.');
  await treeJump(session, user.id);
  session.send(text, images.length ? { images, mode: 'prompt' } : { mode: 'prompt' });
}

/**
 * Fork before a user message (`session_fork`, pi's `fork`): the host keeps the
 * original and streams the fork; its first snapshot names the new session,
 * which `onOpen` receives so the app can route there.
 */
export function forkFrom(hostId: string, sessionId: string, entryId: string | undefined, onOpen?: (sessionId: string) => void): Promise<void> {
  return new Promise((resolve, reject) => {
    let settled = false;
    let stream: { close(): void } | undefined;
    const finish = (error?: unknown) => {
      if (settled) return;
      settled = true;
      stream?.close();
      if (error === undefined) resolve();
      else reject(error);
    };
    try {
      stream = hostClient(hostId).openSession(
        { kind: 'fork', session: sessionId, ...(entryId ? { fromEntry: entryId } : {}) },
        {
          snapshot(snapshot) {
            const id = (snapshot as { session_id?: string }).session_id;
            if (id && id !== sessionId) onOpen?.(id);
            finish();
          },
          events() {
            // The fork's history is irrelevant here; the app opens it properly.
          },
          closed(error) {
            finish(error ?? new Error('The fork closed before it opened.'));
          },
        },
      );
    } catch (error) {
      finish(error);
    }
  });
}

/** Share the turn text (OS share sheet; copy as the fallback). */
export async function shareTurn(turn: Turn): Promise<void> {
  const text = turnText(turn);
  const nav = navigator as Navigator & { share?: (data: { text: string }) => Promise<void> };
  if (typeof nav.share === 'function') {
    try {
      await nav.share({ text });
      return;
    } catch {
      // dismissed / unsupported → copy instead
    }
  }
  await copyText(text);
}

/* ── per-message row menu (right-click / long-press) ─────────────── */

export type MessageTarget = {
  text: string;
  role: 'user' | 'assistant';
  /** This message's entry (absent for live / provisional text). */
  entryId?: string;
  /** The user entry Fork / Edit anchor to (the turn's query for answers). */
  userEntryId?: string;
  turnId: string;
};

export type MessageMenuState = { anchor: DOMRect; target: MessageTarget };

const [messageMenu, setMessageMenu] = createSignal<MessageMenuState | undefined>();

export function currentMessageMenu(): MessageMenuState | undefined {
  return messageMenu();
}

export function openMessageMenu(anchor: DOMRect, target: MessageTarget): void {
  setMessageMenu({ anchor, target });
}

export function closeMessageMenu(): void {
  setMessageMenu(undefined);
}

/**
 * Wires a message element's context menu (desktop) and long-press (phone).
 * A text selection keeps the native menu so Copy still works.
 */
export function attachMessageMenu(element: HTMLElement, target: () => MessageTarget | undefined): () => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let from: { x: number; y: number } | undefined;
  const clear = () => {
    if (timer) clearTimeout(timer);
    timer = undefined;
    from = undefined;
  };
  const selectionActive = () => !(window.getSelection()?.isCollapsed ?? true);

  const onContext = (event: MouseEvent) => {
    const t = target();
    if (!t || selectionActive()) return;
    event.preventDefault();
    openMessageMenu(menuAnchor(event, element), t);
  };
  const onDown = (event: PointerEvent) => {
    if (event.pointerType === 'mouse' || desktopShell()) return;
    from = { x: event.clientX, y: event.clientY };
    clearTimeout(timer);
    timer = setTimeout(() => {
      const t = target();
      timer = undefined;
      if (t && !selectionActive()) openMessageMenu(element.getBoundingClientRect(), t);
    }, 500);
  };
  const onMove = (event: PointerEvent) => {
    if (!from) return;
    if (Math.abs(event.clientX - from.x) > 10 || Math.abs(event.clientY - from.y) > 10) clear();
  };
  const onUp = () => clear();

  element.addEventListener('contextmenu', onContext);
  element.addEventListener('pointerdown', onDown);
  element.addEventListener('pointermove', onMove);
  element.addEventListener('pointerup', onUp);
  element.addEventListener('pointercancel', onUp);
  element.addEventListener('pointerleave', onUp);
  return () => {
    clear();
    element.removeEventListener('contextmenu', onContext);
    element.removeEventListener('pointerdown', onDown);
    element.removeEventListener('pointermove', onMove);
    element.removeEventListener('pointerup', onUp);
    element.removeEventListener('pointercancel', onUp);
    element.removeEventListener('pointerleave', onUp);
  };
}

/* ── phone swipe-to-quote (Z-30 keeps it on the phone) ───────────── */

/** Swipe right: mostly horizontal and far enough to count (v1: 64 px / 40 px). */
export function swipeRight(start: { x: number; y: number }, end: { x: number; y: number }): boolean {
  return end.x - start.x > 64 && Math.abs(end.y - start.y) < 40;
}

/** Wires swipe-right-to-quote on a message element (phone shell only). */
export function attachQuoteSwipe(element: HTMLElement, text: () => string, key: () => string): () => void {
  let from: { x: number; y: number } | undefined;
  const onStart = (event: TouchEvent) => {
    if (desktopShell() || event.touches.length !== 1) return;
    const touch = event.touches[0]!;
    from = { x: touch.clientX, y: touch.clientY };
  };
  const onEnd = (event: TouchEvent) => {
    if (!from) return;
    const start = from;
    from = undefined;
    const touch = event.changedTouches[0];
    if (!touch || !swipeRight(start, { x: touch.clientX, y: touch.clientY })) return;
    quoteIntoComposer(key(), text());
  };
  element.addEventListener('touchstart', onStart, { passive: true });
  element.addEventListener('touchend', onEnd, { passive: true });
  return () => {
    element.removeEventListener('touchstart', onStart);
    element.removeEventListener('touchend', onEnd);
  };
}

/* ── edit menu (draft guard + A27's undo-files option) ──────────── */

export type EditRequest = {
  anchor: DOMRect;
  turnId: string;
  entryId: string;
  text: string;
  images: OutboxImage[];
  /** WP-A27's file-summary capability says this turn has undoable files. */
  canUndo: boolean;
};

const [editRequest, setEditRequest] = createSignal<EditRequest | undefined>();

export function currentEditRequest(): EditRequest | undefined {
  return editRequest();
}

export function openEditRequest(request: EditRequest): void {
  setEditRequest(request);
}

export function closeEditRequest(): void {
  setEditRequest(undefined);
}

/** The failure text shown under a menu / next to the tail (A19: toasts). */
export const actionError = describe;
