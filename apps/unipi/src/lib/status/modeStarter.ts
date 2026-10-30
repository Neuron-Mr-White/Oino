/**
 * Working-mode starters (WP-A14, Z-74): the header chip starts Goal / Ralph /
 * Graph by putting long-horizon's own command in the composer — the modes
 * begin with an objective (`/goal <objective>`, `/ralph start <name> …`,
 * `/graph <prompt>`); there is no promptless switch, and `/regular` is the
 * one command that leaves a mode. Ctrl/Cmd+Shift+M cycles the starter
 * through Goal → Ralph → Graph → plain without opening the menu.
 *
 * Pure text rules so the chip, the shortcut and the tests share them.
 */

/** The starters the chip offers (Z-74 names these three). */
export const STARTER_ORDER = ['goal', 'ralph', 'graph'] as const;
export type ModeStarter = (typeof STARTER_ORDER)[number];
/** Mode or plain. */
export type ModeChoice = ModeStarter | 'none';

/** The command a starter writes, with its trailing space. */
export const STARTER_COMMAND: Record<ModeStarter, string> = {
  goal: '/goal ',
  ralph: '/ralph start ',
  graph: '/graph ',
};

/** One line per starter for the chip's menu. */
export const STARTER_HINT: Record<ModeStarter, string> = {
  goal: 'One objective, until it is verifiably true',
  ralph: 'A task file worked one item at a time',
  graph: 'Steps whose results feed the later ones',
};

/** Strip the leading whitespace and return the text after it. */
const bodyOf = (text: string): string => text.replace(/^\s+/, '');

/** Which starter the draft already carries (a leading long-horizon command). */
export function starterOf(text: string): ModeChoice {
  const body = bodyOf(text);
  if (body === '/goal' || body.startsWith('/goal ')) return 'goal';
  if (body === '/ralph start' || body.startsWith('/ralph start ')) return 'ralph';
  if (body === '/graph' || body.startsWith('/graph ')) return 'graph';
  return 'none';
}

/** The draft without its leading starter command — the user's own words. */
export function stripStarter(text: string): string {
  const body = bodyOf(text);
  const mode = starterOf(body);
  if (mode === 'none') return body;
  const command = STARTER_COMMAND[mode].trimEnd();
  const rest = body.slice(command.length);
  return rest.replace(/^\s+/, '').trimEnd();
}

/**
 * The draft rewritten for `mode`: the starter command in front of the same
 * words, or just the words for plain. Idempotent, and never drops text.
 */
export function withStarter(text: string, mode: ModeChoice): string {
  const body = stripStarter(text);
  if (mode === 'none') return body;
  return `${STARTER_COMMAND[mode]}${body}`;
}

/**
 * The Ctrl/Cmd+Shift+M step: the next starter after the current one, then
 * plain. A draft that carries no starter starts at Goal.
 */
export function cycleStarter(text: string): { text: string; mode: ModeChoice } {
  const current = starterOf(text);
  const at = current === 'none' ? -1 : STARTER_ORDER.indexOf(current);
  const next = at + 1;
  const mode: ModeChoice = next >= STARTER_ORDER.length ? 'none' : STARTER_ORDER[next]!;
  return { text: withStarter(text, mode), mode };
}
