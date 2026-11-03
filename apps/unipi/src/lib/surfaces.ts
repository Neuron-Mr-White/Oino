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
