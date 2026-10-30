/**
 * Board keyboard shortcuts (UNI-117 kanboard migration) — the kanboard web
 * UI's App.tsx keymap, on the desktop shell only (a phone has no keyboard
 * worth binding; the phone app never registers these).
 *
 *   C          New task (Backlog; Todo when the Todo column is selected)
 *   /          Search tasks
 *   J / ↓      Next task        K / ↑   Previous task
 *   Enter      Open the selected task
 *   B / L      Board / List view
 *   R / X      Review queue / Blocked
 *   ?          This list
 *   Esc        Close (shell: top sheet, menu, task pane), else clear the search filter
 *
 * Ctrl/⌘+K (palette) and Ctrl/⌘+1…6 (screens) belong to the shell.
 */

export type BoardKeyAction =
  | { kind: 'new' }
  | { kind: 'search' }
  | { kind: 'select'; delta: 1 | -1 }
  | { kind: 'open' }
  | { kind: 'view'; view: 'board' | 'list' }
  | { kind: 'scope'; scope: 'in_review' | 'blocked' }
  | { kind: 'help' }
  | { kind: 'clear' };

export type KeyContext = {
  /** Focus is in a text field / contenteditable. */
  typing: boolean;
  /** A sheet, menu, dialog or the palette is open. */
  overlay: boolean;
  /** A task is open in the detail pane. */
  taskOpen: boolean;
  /** A project is open. */
  project: boolean;
  /** The text filter is set. */
  filtered: boolean;
};

export const BOARD_SHORTCUTS: Array<[string, string[]]> = [
  ['New task', ['C']],
  ['Search tasks', ['/']],
  ['Next / previous task', ['J', 'K']],
  ['Open selected task', ['↵']],
  ['Board / list', ['B', 'L']],
  ['Review queue / Blocked', ['R', 'X']],
  ['Command palette', ['Ctrl', 'K']],
  ['Switch screen', ['Ctrl', '1…6']],
  ['Close / back', ['Esc']],
  ['This list', ['?']],
];

/** Pure: what a key press means on the board (undefined = not ours). */
export function boardKey(event: Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'metaKey' | 'altKey'>, ctx: KeyContext): BoardKeyAction | undefined {
  if (event.ctrlKey || event.metaKey || event.altKey) return undefined;
  if (ctx.typing || ctx.overlay) return undefined;
  if (event.key === 'Escape') return !ctx.taskOpen && ctx.filtered ? { kind: 'clear' } : undefined;
  if (event.key === '?') return { kind: 'help' };
  if (!ctx.project) return undefined;
  switch (event.key) {
    case 'c':
    case 'C':
      return { kind: 'new' };
    case '/':
      return { kind: 'search' };
    case 'b':
      return { kind: 'view', view: 'board' };
    case 'l':
      return { kind: 'view', view: 'list' };
    case 'r':
      return { kind: 'scope', scope: 'in_review' };
    case 'x':
      return { kind: 'scope', scope: 'blocked' };
  }
  if (ctx.taskOpen) return undefined;
  switch (event.key) {
    case 'j':
    case 'ArrowDown':
      return { kind: 'select', delta: 1 };
    case 'k':
    case 'ArrowUp':
      return { kind: 'select', delta: -1 };
    case 'Enter':
      return { kind: 'open' };
  }
  return undefined;
}

/** Pure: the next selected id walking `ids` (visible order) by `delta`; starts at the first/last. */
export function stepSelection(ids: string[], current: string | null | undefined, delta: 1 | -1): string | undefined {
  if (ids.length === 0) return undefined;
  const at = current ? ids.indexOf(current) : -1;
  if (at === -1) return delta > 0 ? ids[0] : ids[ids.length - 1];
  return ids[Math.max(0, Math.min(ids.length - 1, at + delta))];
}

/** Is the event target a text field (keys must type, not act)? */
export function isTyping(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  return !!el && (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName));
}

/** Any sheet / menu / dialog / palette open? (keys belong to it then) */
export function overlayOpen(doc: Document = document): boolean {
  return !!doc.querySelector('.sheet, [role=menu], [role=dialog]:not(.board-page), [role=alertdialog]');
}
