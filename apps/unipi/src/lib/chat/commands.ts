/** Slash-command suggestions (UNI-113): subsequence ranking, prefix first. */
import type { CommandInfo } from '../session/types';

/** The command sources the rows label (`SOURCE_LABEL`). */
export type CommandSource = 'extension' | 'prompt' | 'skill' | 'builtin';

/** `source` narrowed to the label set (unknown sources fall back to `extension`). */
export function sourceOf(c: CommandInfo): CommandSource {
  return c.source === 'prompt' || c.source === 'skill' || c.source === 'builtin' ? c.source : 'extension';
}

/**
 * UNI-276: pi's own built-in commands the bridge doesn't list (it sends the
 * handful the app acts on: model, thinking, compact, new, resume, fork, tree).
 * Typing `/` suggests every command pi has; these are the rest. The app acts
 * on name / copy / session / reload itself; every other one is routed to an
 * app surface by `lib/composer/builtins.ts` (WP-A25, G-15) — there is no
 * "terminal only" leftover.
 */
export const PI_BUILTINS: readonly CommandInfo[] = [
  { name: 'name', description: 'Rename this session', source: 'builtin' },
  { name: 'copy', description: 'Copy the last reply', source: 'builtin' },
  { name: 'session', description: 'Session info and stats', source: 'builtin' },
  { name: 'settings', description: 'Open settings menu', source: 'builtin' },
  { name: 'scoped-models', description: 'Enable/disable models for Ctrl+P cycling', source: 'builtin' },
  { name: 'export', description: 'Export session (HTML default, or .html/.jsonl path)', source: 'builtin' },
  { name: 'import', description: 'Import and resume a session from a JSONL file', source: 'builtin' },
  { name: 'share', description: 'Share session as a secret GitHub gist', source: 'builtin' },
  { name: 'clone', description: 'Duplicate the session at the current position', source: 'builtin' },
  { name: 'changelog', description: 'Show changelog entries', source: 'builtin' },
  { name: 'hotkeys', description: 'Show all keyboard shortcuts', source: 'builtin' },
  { name: 'trust', description: 'Save project trust decision', source: 'builtin' },
  { name: 'login', description: 'Configure provider authentication', source: 'builtin' },
  { name: 'logout', description: 'Remove provider authentication', source: 'builtin' },
  { name: 'reload', description: 'Reload extensions, skills, prompts and context files', source: 'builtin' },
  { name: 'bug', description: 'Report a bug to the Pi developers', source: 'builtin' },
  { name: 'quit', description: 'Quit pi', source: 'builtin' },
];
/**
 * UNI-308: built-ins the app acts on itself — `copy` here, the rest in the
 * session store/shell. Kept here, not just inline in the composer, so a test
 * can assert `APP_HANDLED_BUILTINS \u222a BUILTIN_ROUTES === PI_BUILTINS` (WP-A25:
 * every built-in has somewhere to go; none silently becomes a prompt).
 */
export const APP_HANDLED_BUILTINS = new Set(['name', 'copy', 'session', 'reload']);

/** pi's built-ins by name (the rows a bare `/` lists after the session's own). */
const PI_ONLY = new Set(PI_BUILTINS.map((c) => c.name));

/** Everything `/` suggests: the session's list plus pi built-ins it doesn't carry (a same-named extension command wins). */
export function allCommands(session: readonly CommandInfo[]): CommandInfo[] {
  const have = new Set(session.map((c) => c.name));
  return [...session, ...PI_BUILTINS.filter((c) => !have.has(c.name))];
}

/**
 * UNI-308: true when `name` (no leading `/`) is something pi or the app
 * actually knows about — the session's own list (extension/prompt/skill
 * commands, which pi expands or runs itself) or one of `PI_BUILTINS` (pi's
 * own built-ins — each one routes to an app surface (`lib/composer/builtins.ts`)).
 *
 * A `/name` that fails this check must never be sent as a plain prompt —
 * typing a slash is a command, not an opinion about pi's design; the app
 * must say "Unknown command /name" instead.
 */
export function isKnownCommand(name: string, session: readonly CommandInfo[]): boolean {
  return allCommands(session).some((c) => c.name === name);
}

/** The `/query` being typed at the caret, or null. Only at the start of the draft or after whitespace. */
export function slashQuery(text: string, caret: number): { start: number; query: string } | null {
  const before = text.slice(0, caret);
  const m = /(^|\s)\/([^\s/]*)$/.exec(before);
  if (!m) return null;
  // Commands only make sense as the first word.
  const start = before.length - m[2]!.length - 1;
  if (text.slice(0, start).trim()) return null;
  return { start, query: m[2]! };
}

/** `/name <prefix>` at the caret (the command word done, typing its argument — one line, no newline). */
export function argQuery(text: string, caret: number): { command: string; start: number; prefix: string } | null {
  const before = text.slice(0, caret);
  const m = /^\s*\/(\S+) ([^\n]*)$/.exec(before);
  if (!m) return null;
  return { command: m[1]!, start: before.length - m[2]!.length, prefix: m[2]! };
}

/** Score: lower is better; null = no match. Ties (e.g. every command when the query is empty, or
 * several exact prefix hits) are broken by source group then name — see rankCommands. */
function score(name: string, query: string): number | null {
  const n = name.toLowerCase();
  const q = query.toLowerCase();
  if (!q) return 0;
  if (n.startsWith(q)) return 0;
  // Word-boundary prefix (after ':' '-' '_').
  const parts = n.split(/[:\-_]/);
  if (parts.some((p) => p.startsWith(q))) return 1;
  if (n.includes(q)) return 2 + n.indexOf(q) / 100;
  // Subsequence.
  let i = 0;
  let gaps = 0;
  for (const ch of n) {
    if (ch === q[i]) i++;
    else if (i > 0) gaps++;
    if (i === q.length) return 3 + gaps / 100;
  }
  return null;
}

/** Group order for ties (same match score): extension commands, then prompts, then skills; `builtin` (app commands like /model) sorts first — they're what a bare `/` is usually typed for. */
const SOURCE_RANK: Record<CommandSource, number> = { builtin: 0, extension: 1, prompt: 2, skill: 3 };

export function rankCommands(commands: readonly CommandInfo[], query: string, limit = 40): CommandInfo[] {
  const scored: Array<{ c: CommandInfo; s: number }> = [];
  commands.forEach((c) => {
    const s = score(c.name, query);
    if (s !== null) scored.push({ c, s });
  });
  // pi's own built-ins that the session does not list are filler for a bare
  // `/`: they sink below the session's commands (the old "terminal" rows did,
  // and they are what the user rarely wants mid-task). WP-A25.
  const rank = (c: CommandInfo) => (c.source === 'builtin' && PI_ONLY.has(c.name) ? 9 : SOURCE_RANK[sourceOf(c)]);
  scored.sort((a, b) => a.s - b.s || rank(a.c) - rank(b.c) || a.c.name.localeCompare(b.c.name));
  return scored.slice(0, limit).map((x) => x.c);
}

/** UNI-304: which command sources the generic `/cmd <arg>` completer is worth
 * asking for. Extension commands may register `getArgumentCompletions`;
 * skills (`/skill:name`) take free-text args so the bridge always answers
 * empty, but listing them here costs nothing and keeps the rule in one
 * place. Built-ins with their own args (`/model`, `/thinking`, `/resume`)
 * are driven by dedicated UI, not this generic completer. */
export function wantsArgCompletions(source: string | undefined): boolean {
  return source === 'extension' || source === 'skill';
}

export const SOURCE_LABEL: Record<CommandSource, string> = {
  builtin: 'app',
  extension: 'command',
  prompt: 'prompt',
  skill: 'skill',
};

/**
 * The `@query` being typed at the caret (pi's rule: an `@` that starts a
 * token, the query runs to the caret; `@"…` may hold spaces). null otherwise.
 */
export function atQuery(text: string, caret: number): { start: number; query: string } | null {
  const before = text.slice(0, caret);
  const quoted = /(^|[\s(\[{])@"([^"]*)$/.exec(before);
  if (quoted) return { start: before.length - quoted[2]!.length - 2, query: `"${quoted[2]}` };
  const m = /(^|[\s(\[{])@([^\s"]*)$/.exec(before);
  if (!m) return null;
  return { start: before.length - m[2]!.length - 1, query: m[2]! };
}

/** Puts a picked `@` suggestion in place of `@query`; directories keep the menu open (no trailing space). */
export function applyAt(text: string, caret: number, at: { start: number }, value: string, dir: boolean): { text: string; caret: number } {
  const after = text.slice(caret).replace(/^[^\s]*/, '');
  // A directory inside quotes ends with `/"`: keep the caret before the quote.
  const insert = dir ? value : `${value} `;
  const next = `${text.slice(0, at.start)}${insert}${after.replace(/^ /, dir ? ' ' : '')}`;
  const pos = at.start + insert.length - (dir && value.endsWith('"') ? 1 : 0);
  return { text: next, caret: pos };
}
