/**
 * Unicrab lore lines (DESIGN.md §1.5): the muted one-liner under an empty
 * state where there is genuinely nothing to do. Never the same line under
 * every state — callers pass a seed (the day) so a screen keeps its line for
 * the whole day instead of changing on every render.
 *
 * Ported from unipi-app 1.0.0 (`src/lore.ts`).
 */
export const LORE = [
  'Unicrab walks sideways so it can read your diffs from both ends.',
  'Unicrab has two claws: one for git add, one for git restore. It rarely mixes them up.',
  'Unicrab molts its shell every release. The changelog is where it keeps the old ones.',
  'Unicrab once compacted a whole ocean into one summary. The summary said: "wet".',
  'Unicrab is not a lobster. Please stop asking about the butter.',
  "Unicrab's favourite exit code is 0. Its second favourite is also 0.",
] as const;

/** The lore line for a seed (default: the day of the month, so it is stable all day). */
export function loreLine(seed = new Date().getDate()): string {
  const i = Math.abs(Math.floor(seed)) % LORE.length;
  return LORE[i]!;
}
