/**
 * The composer's "deep thinking" glow (owner): at thinking level `max` or
 * `xhigh` the composer card wears a slow animated gradient ring and a soft
 * outer glow — stronger for `max`. Pure CSS (`thinkGlow.css`): the value here
 * only names the tier on the card (`data-think-glow`).
 */
export type ThinkGlow = 'max' | 'xhigh';

/** The glow tier for a thinking level (none below X-high, none when read-only). */
export function thinkGlow(level: string | undefined, readOnly?: boolean): ThinkGlow | undefined {
  if (readOnly || !level) return undefined;
  const l = level.trim().toLowerCase();
  if (l === 'max') return 'max';
  if (l === 'xhigh' || l === 'x-high') return 'xhigh';
  return undefined;
}
