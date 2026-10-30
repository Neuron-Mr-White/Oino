/**
 * Desktop shell registries (UNI-117): panes and palette commands.
 *
 * ── Panes (list | detail) ──────────────────────────────────────────────
 * A screen's LIST is its route component (rendered in `.app-content`). Its
 * DETAIL is a full-screen fixed overlay on the phone (chat layer, task
 * page, editor, terminal…). On the desktop shell the detail is placed to
 * the right of the list instead, with a resizable splitter between them:
 *
 *   registerPane({ screen: 'chat', open: () => chatVisible(),
 *                  placeholder: () => <PanePlaceholder title="Pick a chat" /> });
 *
 * and the overlay element gets the class `shell-detail` (or is one of the
 * legacy overlay classes listed in style.css). While `open()` is true the
 * shell sets `html[data-detail]`, narrows the list to `--list-w` and puts
 * the overlay at `left: var(--detail-left)`. `placeholder` (optional) fills
 * the detail area while nothing is open; without one the list takes the
 * whole width. `screen` is the route's first path segment (`''` → sessions).
 *
 * ── Commands (Ctrl/⌘+K palette) ────────────────────────────────────────
 *   onCleanup(registerCommands([{ id: 'board.new', title: 'New task',
 *     group: 'Board', keys: 'N', run: () => … , when: () => !!slug() }]));
 *
 * Screens register their actions while mounted; the shell always has the
 * "Go to …" commands. `when` hides a command that can't run right now.
 */
import { createSignal, type JSX } from 'solid-js';

export type PaneSpec = {
  /** Route's first segment: 'sessions' | 'chat' | 'board' | 'files' | 'preview' | … */
  screen: string;
  /** Is the detail pane showing something? */
  open: () => boolean;
  /** Fills the detail area while nothing is open (desktop only). */
  placeholder?: () => JSX.Element;
};

const [panes, setPanes] = createSignal<Record<string, PaneSpec>>({});

/** Declares a screen's list | detail split. Returns an unregister. */
export function registerPane(spec: PaneSpec): () => void {
  setPanes((current) => ({ ...current, [spec.screen]: spec }));
  return () =>
    setPanes((current) => {
      if (current[spec.screen] !== spec) return current;
      const next = { ...current };
      delete next[spec.screen];
      return next;
    });
}

export function paneFor(screen: string): PaneSpec | undefined {
  return panes()[screen];
}

/** Route path → screen key. */
export function screenOf(pathname: string): string {
  const first = pathname.split('/').filter(Boolean)[0] ?? '';
  if (first === '' || first === 'term') return 'sessions';
  return first;
}

export type Command = {
  id: string;
  title: string;
  /** Section in the palette ("Go to", "Board", "Chat"…). */
  group: string;
  /** Extra words matched by the search (not shown). */
  keywords?: string;
  /** Shortcut hint shown on the right, e.g. "Ctrl 1". */
  keys?: string;
  icon?: () => JSX.Element;
  /** Hidden when false. */
  when?: () => boolean;
  run: () => void;
};

const [commands, setCommands] = createSignal<Command[]>([]);

/** Adds commands (replacing any with the same id). Returns a remover. */
export function registerCommands(list: Command[]): () => void {
  const ids = new Set(list.map((command) => command.id));
  setCommands((current) => [...current.filter((command) => !ids.has(command.id)), ...list]);
  return () => setCommands((current) => current.filter((command) => !list.includes(command)));
}

export function allCommands(): Command[] {
  return commands().filter((command) => command.when?.() ?? true);
}

/**
 * Palette ranking (pure): every query word must match the title, group or
 * keywords (substring or in-order letters); title-prefix and word-start
 * hits rank first. Empty query keeps registration order.
 */
export function rankCommands(list: Command[], query: string): Command[] {
  const words = query.toLowerCase().trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) return list;
  const scored: Array<{ command: Command; score: number; index: number }> = [];
  list.forEach((command, index) => {
    const title = command.title.toLowerCase();
    const hay = `${title} ${command.group.toLowerCase()} ${(command.keywords ?? '').toLowerCase()}`;
    let score = 0;
    for (const word of words) {
      if (title.startsWith(word)) score += 100;
      else if (new RegExp(`\\b${word.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}`).test(title)) score += 60;
      else if (title.includes(word)) score += 40;
      else if (hay.includes(word)) score += 20;
      else if (subsequence(word, title)) score += 5;
      else return;
    }
    scored.push({ command, score, index });
  });
  return scored.sort((a, b) => b.score - a.score || a.index - b.index).map((entry) => entry.command);
}

function subsequence(needle: string, hay: string): boolean {
  let at = 0;
  for (const char of hay) if (char === needle[at]) at += 1;
  return at === needle.length;
}

const [paletteOpen, setPaletteOpenSignal] = createSignal(false);
export { paletteOpen };
export function setPaletteOpen(open: boolean): void {
  setPaletteOpenSignal(open);
}

/** "Ctrl" or "⌘" for shortcut hints. */
export function modKey(): string {
  return typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent) ? '⌘' : 'Ctrl';
}
