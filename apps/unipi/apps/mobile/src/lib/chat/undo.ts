/**
 * Undo/redo for the chat composer (UNI-270). The composer is a
 * contenteditable whose DOM is rebuilt whenever chips form or break, paste
 * and Enter are handled by hand and picks/attachments rewrite the draft —
 * the browser's own undo stack can't follow any of that (it was empty or
 * pointed at nodes that no longer exist). So the composer keeps its own
 * history of draft strings:
 *
 * - typing coalesces into word-ish groups: a group ends when a new word
 *   starts after whitespace, the caret jumped, the edit kind changed or
 *   the user paused (`groupMs`);
 * - deletes (Backspace/Delete runs) coalesce the same way;
 * - everything else (paste, a chip/@file/slash pick, an attachment, a
 *   quote, restored queue messages, chip ✕) is one step of its own;
 * - a new edit after an undo clears the redo stack.
 *
 * Histories are kept per chat (`composerHistory(key)`) so switching chats
 * and back keeps them, like the drafts.
 */

export type EditKind = 'type' | 'delete' | 'other';
export type UndoState = { text: string; caret: number };

const LIMIT = 200;

/** Where the change from `from` to `to` ends in `to` (the caret after an undo/redo). */
export function changeEnd(from: string, to: string): number {
  let p = 0;
  const max = Math.min(from.length, to.length);
  while (p < max && from[p] === to[p]) p++;
  let s = 0;
  while (s < max - p && from[from.length - 1 - s] === to[to.length - 1 - s]) s++;
  return to.length - s;
}

export class EditHistory {
  private past: UndoState[] = [];
  private future: UndoState[] = [];
  present: UndoState | undefined;
  private lastKind: EditKind | null = null;
  private lastAt = 0;

  constructor(private readonly opts: { groupMs?: number; now?: () => number; limit?: number } = {}) {}

  private now() {
    return this.opts.now?.() ?? Date.now();
  }

  /** Records the draft after an edit. `caret` is where the caret ended up. */
  record(text: string, caret: number, kind: EditKind): void {
    const cur = this.present;
    if (!cur) {
      this.present = { text, caret };
      return;
    }
    if (text === cur.text) {
      cur.caret = caret;
      return;
    }
    const now = this.now();
    const merge = kind !== 'other' && kind === this.lastKind && now - this.lastAt <= (this.opts.groupMs ?? 1500) && this.continues(cur, text, caret, kind);
    if (merge) {
      this.present = { text, caret };
    } else {
      this.past.push(cur);
      const limit = this.opts.limit ?? LIMIT;
      if (this.past.length > limit) this.past.splice(0, this.past.length - limit);
      this.present = { text, caret };
      this.lastKind = kind;
    }
    this.future = [];
    this.lastAt = now;
  }

  /** Whether `text` continues the current group (a contiguous insert/delete at the caret). */
  private continues(cur: UndoState, text: string, caret: number, kind: EditKind): boolean {
    if (kind === 'type') {
      const n = text.length - cur.text.length;
      const start = caret - n;
      if (n <= 0 || start !== cur.caret) return false;
      if (text.slice(0, start) !== cur.text.slice(0, start) || text.slice(caret) !== cur.text.slice(start)) return false;
      const inserted = text.slice(start, caret);
      // A new word after whitespace starts a new group ("hello |world").
      return !(/^\S/.test(inserted) && start > 0 && /\s/.test(text[start - 1]!));
    }
    if (kind === 'delete') {
      const n = cur.text.length - text.length;
      if (n <= 0) return false;
      const back = caret === cur.caret - n;
      const fwd = caret === cur.caret;
      if (!back && !fwd) return false;
      return text === cur.text.slice(0, caret) + cur.text.slice(caret + n);
    }
    return false;
  }

  canUndo(): boolean {
    return this.past.length > 0;
  }
  canRedo(): boolean {
    return this.future.length > 0;
  }

  undo(): UndoState | undefined {
    const cur = this.present;
    const prev = this.past.pop();
    if (!cur || !prev) return undefined;
    this.future.push(cur);
    this.present = { text: prev.text, caret: changeEnd(cur.text, prev.text) };
    this.lastKind = null;
    return { ...this.present };
  }

  redo(): UndoState | undefined {
    const cur = this.present;
    const next = this.future.pop();
    if (!cur || !next) return undefined;
    this.past.push(cur);
    this.present = { text: next.text, caret: changeEnd(cur.text, next.text) };
    this.lastKind = null;
    return { ...this.present };
  }

  /** A fresh start at `text` (after a send): nothing to undo or redo. */
  reset(text: string, caret = text.length): void {
    this.past = [];
    this.future = [];
    this.present = { text, caret };
    this.lastKind = null;
  }

  /** Every draft the history holds or can still bring back (attachments in them are kept alive). */
  reachable(): string[] {
    return [...this.past, ...(this.present ? [this.present] : []), ...this.future].map((s) => s.text);
  }
}

const histories = new Map<string, EditHistory>();

/** The composer history of one chat (`hostId:pid`). */
export function composerHistory(key: string): EditHistory {
  let h = histories.get(key);
  if (!h) {
    h = new EditHistory();
    histories.set(key, h);
  }
  return h;
}

/** Ctrl/⌘+Z → undo, Ctrl/⌘+Shift+Z or Ctrl/⌘+Y → redo (by key or, on other layouts, by physical key). */
export function undoKey(e: Pick<KeyboardEvent, 'key' | 'code' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey'>): 'undo' | 'redo' | null {
  if (!(e.ctrlKey || e.metaKey) || e.altKey) return null;
  const k = e.key.length === 1 && /[a-z]/i.test(e.key) ? e.key.toLowerCase() : e.code === 'KeyZ' ? 'z' : e.code === 'KeyY' ? 'y' : '';
  if (k === 'z') return e.shiftKey ? 'redo' : 'undo';
  if (k === 'y' && !e.shiftKey) return 'redo';
  return null;
}

/** Forgets a chat's history (tests; a closed chat). */
export function clearComposerHistory(key?: string): void {
  if (key === undefined) histories.clear();
  else histories.delete(key);
}
