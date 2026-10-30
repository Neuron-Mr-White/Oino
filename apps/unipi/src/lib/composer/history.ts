/**
 * Prompt history (app spec WP-A12, Z-58; ZCode `composer-prompt-history.md`):
 * every sent prompt, per **workspace** (`${hostId}:${cwd}`), capped at 30,
 * consecutive duplicates skipped. `↑`/`↓` on an empty composer (or one
 * already walking the history) recall entries; typing leaves the walk.
 *
 * Boundary decisions (ZCode clarification log, ruled):
 * - write path: a new entry equal (after `trim()`) to the **last** entry is
 *   skipped — `A,A` keeps one, `A,B,A` keeps all three;
 * - read path: old stored duplicates are never migrated or cleaned;
 * - a send that fails does not keep its history entry: `rollback` undoes the
 *   append, but only while that text is still the last entry (later sends
 *   win).
 */

export const PROMPT_HISTORY_LIMIT = 30;

/** Pure: `text` appended to `list`, skipping a consecutive duplicate, capped. */
export function appendPrompt(list: readonly string[], text: string): string[] {
  const trimmed = text.trim();
  if (!trimmed) return [...list];
  if (list.length && list[list.length - 1]!.trim() === trimmed) return [...list];
  const next = [...list, trimmed];
  return next.length > PROMPT_HISTORY_LIMIT ? next.slice(next.length - PROMPT_HISTORY_LIMIT) : next;
}

/**
 * The walk: `index === null` means "not browsing" (the composer shows the
 * draft being typed). `↑` enters at the newest entry and moves back; `↓`
 * moves forward and `null` past the oldest-newest edge returns to the draft.
 */
export function walkUp(list: readonly string[], index: number | null): number | null {
  if (!list.length) return null;
  return index === null ? list.length - 1 : Math.max(0, index - 1);
}

/** `null` when the walk leaves the list (back to the draft being typed). */
export function walkDown(list: readonly string[], index: number | null): number | null {
  if (index === null) return null;
  const next = index + 1;
  return next >= list.length ? null : next;
}

export type PromptHistoryStorage = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

const storeKey = (workspace: string): string => `unipi:composer:history:${encodeURIComponent(workspace)}`;

/** Reads a stored history (bad data → empty; never deduplicated). */
export function readPromptHistory(workspace: string, storage: PromptHistoryStorage): string[] {
  try {
    const raw = storage.getItem(storeKey(workspace));
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((x): x is string => typeof x === 'string');
  } catch {
    return [];
  }
}

/** One workspace's prompt history (`add` on send, `rollback` on send failure). */
export function promptHistory(workspace: string, storage: PromptHistoryStorage = localStorage) {
  const key = storeKey(workspace);
  const write = (list: string[]): void => {
    try {
      if (list.length) storage.setItem(key, JSON.stringify(list));
      else storage.removeItem(key);
    } catch {
      // storage full / blocked: history just isn't remembered
    }
  };
  return {
    list(): string[] {
      return readPromptHistory(workspace, storage);
    },
    /** Records a sent prompt (skips a consecutive duplicate). */
    add(text: string): void {
      write(appendPrompt(readPromptHistory(workspace, storage), text));
    },
    /** Undoes `add` for a send that failed — only while it is still the last entry. */
    rollback(text: string): void {
      const list = readPromptHistory(workspace, storage);
      if (!list.length || list[list.length - 1]!.trim() !== text.trim()) return;
      write(list.slice(0, -1));
    },
    /** Tests. */
    clear(): void {
      try {
        storage.removeItem(key);
      } catch {
        // ignore
      }
    },
  };
}
