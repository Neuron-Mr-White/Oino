/** Slash-command suggestions (UNI-113): subsequence ranking, prefix first. */
import type { CommandInfo } from '../session/types';
import { shownInApp } from '../surfaces';

/** The command sources the rows label (`SOURCE_LABEL`). */
export type CommandSource = 'extension' | 'prompt' | 'skill' | 'builtin';

/** `source` narrowed to the label set (unknown sources fall back to `extension`). */
export function sourceOf(c: CommandInfo): CommandSource {
  return c.source === 'prompt' || c.source === 'skill' || c.source === 'builtin' ? c.source : 'extension';
}

/**
 * UNI-276 / P0-02: the built-in commands the engine itself has. oino's
 * `get_commands` lists only extension / prompt / skill commands, so the app
 * supplies these; every one either runs an app action or the oino RPC it maps
 * to (`lib/composer/builtins.ts`: `SESSION_BUILTINS` acts on the chat,
 * `BUILTIN_ROUTES` opens an app surface). A built-in with neither is not
 * listed — nothing typed as a command ever reaches the model as text.
 * `/reload` re-reads settings, extension packages, skills / prompts / context files,
 * `models.json` and the credentials of the chat it runs in (the host refuses it while
 * the chat is working); Settings › General has the host-wide one.
 */
export const PI_BUILTINS: readonly CommandInfo[] = [
  { name: 'model', description: 'Pick the model (or /model <name>)', source: 'builtin' },
  { name: 'thinking', description: 'Thinking level (or /thinking <level>)', source: 'builtin' },
  { name: 'compact', description: 'Compact the conversation now (optional focus text)', source: 'builtin' },
  { name: 'reload', description: 'Re-read settings, models, extensions, skills and context files', source: 'builtin' },
  { name: 'new', description: 'Start a new chat in this folder', source: 'builtin' },
  { name: 'resume', description: 'Open another chat', source: 'builtin' },
  { name: 'fork', description: 'Fork this chat into a new one', source: 'builtin' },
  { name: 'tree', description: 'Show the conversation tree', source: 'builtin' },
  { name: 'name', description: 'Rename this chat (or /name <title>)', source: 'builtin' },
  { name: 'copy', description: 'Copy the last reply', source: 'builtin' },
  { name: 'session', description: 'Session info and stats', source: 'builtin' },
  { name: 'settings', description: 'Open settings', source: 'builtin' },
  { name: 'scoped-models', description: 'Choose the models to cycle through', source: 'builtin' },
  { name: 'export', description: 'Export this chat (JSONL)', source: 'builtin' },
  { name: 'import', description: 'Import a session from a JSONL file', source: 'builtin' },
  // Dead in both clients (no share service): typed, it still points at Export; never suggested.
  { name: 'share', description: 'Share this chat', source: 'builtin', surfaces: [] },
  { name: 'clone', description: 'Duplicate this chat at the current point', source: 'builtin' },
  { name: 'changelog', description: "What's new", source: 'builtin' },
  { name: 'hotkeys', description: 'Show all keyboard shortcuts', source: 'builtin' },
  // The terminal saves the trust decision; the app has no trust list (typed, a notice says so).
  { name: 'trust', description: 'Project trust', source: 'builtin', surfaces: ['tui'] },
  { name: 'login', description: 'Sign in to a provider', source: 'builtin' },
  { name: 'logout', description: 'Sign out of a provider', source: 'builtin' },
  { name: 'bug', description: 'Report a bug', source: 'builtin' },
  { name: 'quit', description: 'Quit Unipi', source: 'builtin' },
];
/**
 * The built-ins that act on the chat itself (`SESSION_BUILTINS` in
 * `lib/composer/builtins.ts`, run by the composer). With `BUILTIN_ROUTES`
 * they cover exactly `PI_BUILTINS` (asserted by a test): none silently
 * becomes a prompt.
 */
export const APP_HANDLED_BUILTINS = new Set(['model', 'thinking', 'compact', 'reload', 'new', 'resume', 'fork', 'tree', 'name', 'copy', 'session']);

/** pi's built-ins by name (the rows a bare `/` lists after the session's own). */
const PI_ONLY = new Set(PI_BUILTINS.map((c) => c.name));

/**
 * Owner 2026-11-05: terminal-era commands whose job the app does with its own UI (the
 * model dialog, the thinking chip, Settings, the session modal, the composer's plug
 * menu). They stay out of `/` and the palette; typed, they still run.
 */
export const GUI_HIDDEN_COMMANDS: ReadonlySet<string> = new Set([
  'model', 'thinking', 'agents', 'bg', 'bg-detach', 'bg-tasks', 'compact-doctor', 'compact-help', 'compact-stats',
  'cron', 'doctor', 'fusion', 'fusion-stats', 'global-memory-list', 'global-memory-search', 'info', 'mcp-status',
  'move', 'notify-event', 'permission', 'readme', 'skills', 'subagents', 'bug', 'visualize-progress', 'changelog',
]);

/** `/memory`, `/memory-search`, `/memory-forget`… (the Memory browser and the chat ⋯ menu cover them). */
export function guiHiddenCommand(name: string): boolean {
  const n = name.replace(/^\//, '');
  return GUI_HIDDEN_COMMANDS.has(n) || n === 'memory' || n.startsWith('memory-');
}

/** Whether the app lists a command ([gui-cleanup]): rows marked for other clients (`surfaces`, e.g. `/answer` → `["tui"]`) stay out of `/` and the palette. */
export function shownInMenu(c: CommandInfo): boolean {
  return shownInApp(c.surfaces) && !guiHiddenCommand(c.name);
}

/** The session's commands plus pi built-ins it doesn't carry (a same-named extension command wins), unfiltered. */
function everyCommand(session: readonly CommandInfo[]): CommandInfo[] {
  const have = new Set(session.map((c) => c.name));
  return [...session, ...PI_BUILTINS.filter((c) => !have.has(c.name))];
}

/** Everything `/` suggests: the session's list plus pi built-ins it doesn't carry (a same-named extension command wins),
 *  without the commands meant for the terminal only (`shownInMenu`).
 *  `builtin` filters the built-ins (the new-chat draft lists only those that work without a chat). */
export function allCommands(session: readonly CommandInfo[], builtin: (name: string) => boolean = () => true): CommandInfo[] {
  return everyCommand(session).filter((c) => shownInMenu(c) && (!PI_ONLY.has(c.name) || session.some((s) => s.name === c.name) || builtin(c.name)));
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
  // Terminal-only rows are not suggested but still run when typed (`/answer` keeps working).
  return everyCommand(session).some((c) => c.name === name);
}

/** A typed `/name` that the session does not run itself (an engine built-in the app handles).
 *  A session row with `source: 'builtin'` is the engine naming its own built-in, not an extension.
 *  Only an extension command of the same name takes it over: a prompt template or skill never
 *  shadows a built-in (pi parity — OIN-19: a project `prompts/compact.md` turned `/compact` into
 *  a chat message carrying the summarizer prompt instead of compacting). */
export function isAppBuiltin(name: string, session: readonly CommandInfo[]): boolean {
  return PI_ONLY.has(name) && !session.some((c) => c.name === name && sourceOf(c) === 'extension');
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
  // The chat built-ins (/model, /compact…) are what a bare `/` is often typed for: they keep the top group.
  const rank = (c: CommandInfo) => (c.source === 'builtin' && PI_ONLY.has(c.name) && !APP_HANDLED_BUILTINS.has(c.name) ? 9 : SOURCE_RANK[sourceOf(c)]);
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

/** A `$skill` being typed at the caret, anywhere in the text (owner spec 2026-11-03): after the
 *  start, whitespace or an opening bracket. `$5` or `a$b` never open it. */
export function skillQuery(text: string, caret: number): { start: number; query: string } | null {
  const before = text.slice(0, caret);
  const m = /(^|[\s(\[{])\$([A-Za-z][\w.-]*|)$/.exec(before);
  if (!m) return null;
  return { start: before.length - m[2]!.length - 1, query: m[2]! };
}

/** The session's skills as `$` menu rows (`skill:review` → `review`), ranked like `/` rows. */
export function rankSkills(commands: readonly CommandInfo[], query: string, limit = 40): CommandInfo[] {
  const skills = commands.filter((c) => c.name.startsWith('skill:')).map((c) => ({ ...c, name: c.name.slice(6) }));
  return rankCommands(skills, query, limit);
}

/** Puts a picked `$` skill in place of `$query`: `$name ` (the trailing space makes it a chip). */
export function applySkill(text: string, caret: number, at: { start: number }, name: string): { text: string; caret: number } {
  const after = text.slice(caret).replace(/^[^\s]*/, '');
  const insert = `$${name} `;
  const next = `${text.slice(0, at.start)}${insert}${after.replace(/^ /, '')}`;
  return { text: next, caret: at.start + insert.length };
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

/** F-05: commands whose bare form has nothing to do — Enter in the menu completes `/graph ` and waits for the task. */
const TASK_COMMANDS = new Set(['graph', 'goal', 'ralph', 'swarm', 'unipi:graph', 'unipi:goal', 'unipi:ralph', 'unipi:swarm']);
export function commandNeedsTask(c: CommandInfo): boolean {
  if (TASK_COMMANDS.has(c.name)) return true;
  const hint = c.argumentHint ?? c.argument_hint;
  return typeof hint === 'string' && hint.trim().startsWith('<');
}
