/**
 * Time-of-day greeting for the new-chat empty state (Z-87, WP-A28), taken
 * from ZCode (`docs/ui/chat-empty-time-greeting.md`): the local hour of the
 * device picks one of six slots; the title never carries the workspace or
 * project name (so the empty state can't be read as "a task already exists").
 *
 * Pure, so the slots are unit-tested without faking the clock.
 */
export type GreetingSlot = 'early' | 'morning' | 'noon' | 'afternoon' | 'evening' | 'late';

/** The slot for a local hour (0–23). */
export function greetingSlot(hour: number): GreetingSlot {
  const h = Math.min(23, Math.max(0, Math.floor(hour)));
  if (h >= 5 && h < 9) return 'early';
  if (h >= 9 && h < 12) return 'morning';
  if (h >= 12 && h < 14) return 'noon';
  if (h >= 14 && h < 18) return 'afternoon';
  if (h >= 18 && h < 23) return 'evening';
  return 'late';
}

/** The greeting line per slot (ZCode's English copy, unipi punctuation). */
export const GREETING_TEXT: Record<GreetingSlot, string> = {
  early: 'Morning, ready when you are',
  morning: 'Morning, how can I help?',
  noon: 'Noon break?',
  afternoon: 'Good afternoon! Leave the rest to me.',
  evening: 'Evening, nice work today',
  late: "It's late — take care of yourself.",
};

/** `greetingFor(new Date())` — the empty state's title. */
export function greetingFor(date: Date): string {
  return GREETING_TEXT[greetingSlot(date.getHours())];
}
