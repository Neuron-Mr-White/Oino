/**
 * Composer drafts (app spec §3.3 "composer", WP-A12, Z-59): per session
 * (`hostId/sessionId`, or `draft:<hostId>:<cwd>` for a new-chat draft) the
 * composer keeps
 *
 *   text     — persisted (localStorage, debounced 500 ms, removed when
 *              emptied: "没有输入文字就不算草稿", ZCode draft persistence);
 *   quotes   — in-memory quote chips (A7's tail/selection quote handoff);
 *   images   — **derived**, not stored here: the pending attachments live in
 *              `lib/composer/attachments.ts`'s per-chat store (D-6 — the
 *              composer's `images` signal became that store, upload states
 *              and all), and `composerDraft` merges the ready ones in so
 *              every existing reader keeps working;
 *   chips    — in-memory paste chips (Z-61) and the public chip API's
 *              B8 element/screenshot chips, flattened inline on send.
 *
 * Everything the timeline's turn actions put here (quotes, Edit restores)
 * goes through this module, so the composer is the only consumer.
 */
import { createSignal } from 'solid-js';
import type { OutboxImage } from '../session/types';
import { clearAttachments, markPlaced, pendingAttachmentsFor, readyImages, removeAttachment, resetAttachments, setImages } from './attachments';
import { restoredImageText, stripAttachmentTokens } from './chips';

export type QuoteChip = { id: string; text: string };

/**
 * A composer chip outside the text: a long paste (Z-61) or an entry added
 * through the public chip API (B8's element/screenshot chips). Chips hold
 * their own payload; `composerSendText` flattens `markdown` into the prompt.
 */
export type ComposerChip = {
  id: string;
  /** Chip family for styling/behaviour: `paste`, `element`, `screenshot`, … */
  kind: string;
  /** What the chip shows: "Pasted text · 42 lines". */
  label: string;
  /** A tooltip / preview head. */
  detail?: string;
  /** The markdown the chip expands to in the sent prompt. */
  markdown?: string;
};

/** The composer's state for one chat. `images` is derived from the attachments
 *  store (D-6) — never persisted and never stored in this module. */
export type ComposerDraft = { text: string; quotes: QuoteChip[]; images: OutboxImage[]; chips: ComposerChip[] };

const EMPTY: ComposerDraft = { text: '', quotes: [], images: [], chips: [] };

type DraftStorage = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

const STORE_PREFIX = 'unipi:composer:draft:';
/** §3.3: text persisted debounced 500 ms. */
const PERSIST_DEBOUNCE_MS = 500;

let storage: DraftStorage | undefined = typeof localStorage === 'undefined' ? undefined : localStorage;

/** Tests: swap persistence (or drop it with `undefined`). */
export function setDraftStorage(next: DraftStorage | undefined): void {
  storage = next;
}

const readStored = (key: string): string => {
  try {
    const raw = storage?.getItem(STORE_PREFIX + key);
    return typeof raw === 'string' ? raw : '';
  } catch {
    return '';
  }
};

const [drafts, setDrafts] = createSignal<Record<string, ComposerDraft>>({});
const timers = new Map<string, ReturnType<typeof setTimeout>>();
let quoteSeq = 0;
let chipSeq = 0;

/** A stored, non-empty text counts as the draft after a reload (Z-59). */
function load(key: string): ComposerDraft {
  // The attachments behind inline image/file chips were memory only: a reload drops their tokens.
  const text = stripAttachmentTokens(readStored(key));
  return text.trim() ? { ...EMPTY, text } : EMPTY;
}

const persist = (key: string, text: string): void => {
  const pending = timers.get(key);
  if (pending) {
    clearTimeout(pending);
    timers.delete(key);
  }
  const write = () => {
    timers.delete(key);
    try {
      // Only text persists; emptied → the record is removed (ZCode rule).
      if (text.trim()) storage?.setItem(STORE_PREFIX + key, text);
      else storage?.removeItem(STORE_PREFIX + key);
    } catch {
      // storage full / blocked: drafts just aren't remembered
    }
  };
  if (!text.trim()) write(); // emptied: removed at once, not 500 ms later
  else timers.set(key, setTimeout(write, PERSIST_DEBOUNCE_MS));
};

/** Flushes pending debounced writes (tests, pagehide). */
export function flushComposerDrafts(): void {
  for (const timer of timers.values()) clearTimeout(timer);
  timers.clear();
  for (const [key, draft] of Object.entries(drafts())) {
    try {
      if (draft.text.trim()) storage?.setItem(STORE_PREFIX + key, draft.text);
      else storage?.removeItem(STORE_PREFIX + key);
    } catch {
      // ignore
    }
  }
}

if (typeof window !== 'undefined') {
  // A reload inside the 500 ms window must not lose the text.
  window.addEventListener('pagehide', flushComposerDrafts);
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'hidden') flushComposerDrafts();
  });
}

/** Whether this app run has the chat's draft in memory (edited or restored here), as opposed to
 *  only the stored text a reload reads back. */
export function hasLiveDraft(key: string): boolean {
  return key in drafts();
}

export function composerDraft(key: string): ComposerDraft {
  const stored = drafts()[key] ?? load(key);
  return { ...stored, images: readyImages(key) };
}

const update = (key: string, fn: (draft: ComposerDraft) => ComposerDraft): void => {
  setDrafts((all) => {
    const cur = all[key] ?? load(key);
    const next = fn(cur);
    return next === cur ? all : { ...all, [key]: next };
  });
};

export function setComposerText(key: string, text: string): void {
  update(key, (draft) => ({ ...draft, text }));
  persist(key, text);
}

export function addComposerQuote(key: string, text: string): QuoteChip {
  const quote = { id: `q${++quoteSeq}`, text: text.trim() };
  update(key, (draft) => ({ ...draft, quotes: [...draft.quotes, quote] }));
  return quote;
}

export function removeComposerQuote(key: string, id: string): void {
  update(key, (draft) => ({ ...draft, quotes: draft.quotes.filter((quote) => quote.id !== id) }));
}

/** Removes the `index`-th pending image (attach order) from the chat's store. */
export function removeComposerImage(key: string, index: number): void {
  const image = pendingAttachmentsFor(key).filter((a) => a.kind === 'image')[index];
  if (image) removeAttachment(key, image.id);
}

/** Replaces every pending image (Edit / tree jump restore; tests). */
export function setComposerImages(key: string, images: OutboxImage[]): void {
  setImages(key, images);
}

/** Edit / tree jump (A7): replaces the composer's text and images. */
export function restoreComposer(key: string, draft: { text: string; images?: OutboxImage[] }): void {
  // The images come back as inline chips where the sent text named them (`[image N]`), the rest after it.
  const ids = setImages(key, draft.images ?? []);
  const text = ids.length ? restoredImageText(draft.text, ids) : draft.text;
  markPlaced(key, ids);
  setDrafts((all) => ({ ...all, [key]: { text, quotes: [], images: [], chips: all[key]?.chips ?? [] } }));
  persist(key, text);
}

/** A draft from the host (another device, or before a restart): text, quotes and chips. The
 *  attachments are restored by the caller (`draftSync`). */
export function applyHostDraft(key: string, draft: { text: string; quotes?: QuoteChip[]; chips?: ComposerChip[] }): void {
  setDrafts((all) => ({ ...all, [key]: { text: draft.text, quotes: draft.quotes ?? [], images: [], chips: draft.chips ?? [] } }));
  persist(key, draft.text);
}

/** Starts the composer over (a send): text, quotes, chips and attachments. */
export function clearComposer(key: string): void {
  clearAttachments(key);
  // D-11: the stored copy goes first — dropping the in-memory draft makes readers fall back to
  // storage, which would still hold the text just sent and leave Send enabled on an empty field.
  persist(key, '');
  setDrafts((all) => {
    if (!(key in all)) return all;
    const next = { ...all };
    delete next[key];
    return next;
  });
}

/** Adds a chip (paste, B8 element/screenshot) to a session's draft. */
export function addComposerChip(key: string, chip: Omit<ComposerChip, 'id'> & { id?: string }): ComposerChip {
  const full: ComposerChip = { id: chip.id ?? `c${++chipSeq}`, kind: chip.kind, label: chip.label, ...(chip.detail !== undefined ? { detail: chip.detail } : {}), ...(chip.markdown !== undefined ? { markdown: chip.markdown } : {}) };
  update(key, (draft) => ({ ...draft, chips: [...draft.chips, full] }));
  return full;
}

export function removeComposerChip(key: string, id: string): void {
  update(key, (draft) => ({ ...draft, chips: draft.chips.filter((chip) => chip.id !== id) }));
}

/**
 * The message text a send produces: quote chips as blockquotes, chip
 * markdown blocks (a long paste expands inline, Z-61 — pi parity: the model
 * receives the text), then the draft text.
 */
export function composerSendText(draft: ComposerDraft): string {
  const quoted = draft.quotes
    .map((quote) =>
      quote.text
        .split('\n')
        .map((line) => `> ${line}`)
        .join('\n'),
    )
    .join('\n\n');
  const chipBlocks = draft.chips.map((chip) => chip.markdown?.trim() ?? '').filter(Boolean);
  return [quoted, ...chipBlocks, draft.text].filter((part) => part.trim()).join('\n\n');
}

/**
 * Public chip API (app spec WP-A12) for other surfaces — WP-B8's element
 * picker and screenshot button drop chips into a session's composer this
 * way; they flatten into the sent prompt from there.
 */
export const composerChips = {
  add(sessionKey: string, chip: Omit<ComposerChip, 'id'> & { id?: string }): string {
    return addComposerChip(sessionKey, chip).id;
  },
  remove(sessionKey: string, id: string): void {
    removeComposerChip(sessionKey, id);
  },
};

/* ── focus (A7 handoff: quote, Edit, mention requests focus the field) ── */

const focusers = new Map<string, Set<() => void>>();

/** Wires a composer input to focus requests (returns the cleanup). */
export function registerComposerFocus(key: string, focus: () => void): () => void {
  const set = focusers.get(key) ?? new Set();
  set.add(focus);
  focusers.set(key, set);
  return () => {
    set.delete(focus);
    if (!set.size) focusers.delete(key);
  };
}

export function focusComposer(key: string): void {
  for (const focus of focusers.get(key) ?? []) focus();
}

/** Tests: simulate an app reload — in-memory drafts die (the per-chat
 *  attachments are memory too, D-6), storage stays. */
export function reloadComposerDrafts(): void {
  flushComposerDrafts();
  setDrafts({});
  resetAttachments();
}

/** Tests: drop every draft (memory + storage). */
export function resetComposerDrafts(): void {
  flushComposerDrafts();
  setDrafts({});
  resetAttachments();
  for (const key of [...timers.keys()]) {
    clearTimeout(timers.get(key)!);
    timers.delete(key);
  }
  const st = storage as (DraftStorage & Pick<Storage, 'key' | 'length'>) | undefined;
  try {
    if (st?.key && typeof st.length === 'number') {
      for (let i = st.length - 1; i >= 0; i--) {
        const k = st.key(i);
        if (k?.startsWith(STORE_PREFIX)) st.removeItem(k);
      }
    }
  } catch {
    // ignore
  }
}
