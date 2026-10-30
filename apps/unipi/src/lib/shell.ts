/**
 * Palette commands (UNI-117; WP-A18 grows this into the command center).
 *
 *   onCleanup(registerCommands([{ id: 'board.new', title: 'New task',
 *     group: 'Board', keys: 'N', run: () => … , when: () => !!slug() }]));
 *
 * Screens register their actions while mounted; the shell registers its
 * "Go to" and "View" commands (AppShell). `when` hides a command that can't
 * run right now. The old list | detail pane registry is gone (WP-A3: side
 * pane + bottom panel, lib/panes.ts).
 */
import { createSignal, type JSX } from 'solid-js';

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
