/**
 * Client surfaces ([gui-cleanup], owner 2026-11-03): slash commands and
 * settings fields may say which clients they are meant for — `surfaces:
 * ["tui"]` on a field (`oino_ui::settings::Field.surfaces`) or on a
 * `get_commands` row (from the extension manifest's `ui.commands.<name>`).
 * Absent = every client. The app is the `gui` client: it hides what is not
 * meant for it and leaves the terminal UI / CLI to show the rest.
 *
 * Classification table: docs/dev/v2/qa/gui-tui-commands.md.
 */

/** The client kinds (`oino_ui::settings::Surface`). */
export const SURFACES = ['gui', 'tui'] as const;
export type Surface = (typeof SURFACES)[number];

/** This client. */
export const THIS_CLIENT: Surface = 'gui';

/** The wire `surfaces` list, or undefined when absent / not a list (= every client). Unknown kinds are dropped. */
export function parseSurfaces(raw: unknown): Surface[] | undefined {
  if (!Array.isArray(raw)) return undefined;
  return raw.filter((s): s is Surface => (SURFACES as readonly string[]).includes(s as string));
}

/** Whether something carrying `surfaces` shows in the app (absent = yes). */
export function shownInApp(surfaces: unknown): boolean {
  const list = parseSurfaces(surfaces);
  return !list || list.includes(THIS_CLIENT);
}

/**
 * [ask-user] Registry entries and one-shot effects carry the same `surfaces` (core copies the
 * producer's `surfaces` onto the entry: `setWidget`, `setStatus`, `setHeader`, `setFooter`,
 * `openPanel`). Builds of the extensions from before that sent these terminal-only entries
 * without it; they are hidden by id.
 */
const LEGACY_TUI_ENTRIES = new Set([
  // utility: "N questions in the reply — /answer …"
  'unipi.utility/widget:unipi-answer-hint',
  // info-screen: the ASCII Unicrab splash header
  'unipi.info-screen/setHeader:splash',
  // subagents: the tray's `f` foreground watch cards
  'unipi.subagents/widget:subagents-foreground',
  // core / btw: the terminal's editor-replacing work tray and /btw panel (the app has the
  // Running tab and the Side chat tab)
  'unipi.core/tray',
  'unipi.btw/panel',
]);

/** Whether a registry entry shows in the app. */
export function entryShownInApp(entry: { id: string; surfaces?: unknown }): boolean {
  return shownInApp(entry.surfaces) && !LEGACY_TUI_ENTRIES.has(entry.id);
}
