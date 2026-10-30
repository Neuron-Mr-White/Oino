/**
 * Turn-tail timestamps (WP-A7, Z-26): the four graduations the owner ruled
 * (`HH:MM` / `Yesterday HH:MM` / `M/D, HH:MM` / `Y/M/D, HH:MM`) and the
 * locale's clock format.
 */
import { afterEach, describe, expect, it } from 'vitest';
import { clearTimeLabelCache, formatMessageTimeLabel } from './timeLabel';

// Local wall-clock instants: the label is the reader's local time (not UTC).
const NOW = new Date(2026, 9, 30, 12, 0).getTime();
const at = (y: number, m: number, d: number, h: number, min: number) => new Date(y, m - 1, d, h, min).getTime();

afterEach(() => clearTimeLabelCache());

describe('formatMessageTimeLabel', () => {
  it('shows only the time today', () => {
    expect(formatMessageTimeLabel(at(2026, 10, 30, 9, 41), 'en-GB', NOW)).toBe('09:41');
  });

  it('prefixes yesterday', () => {
    expect(formatMessageTimeLabel(at(2026, 10, 29, 9, 41), 'en-GB', NOW)).toBe('Yesterday 09:41');
  });

  it('shows month/day and time this year', () => {
    expect(formatMessageTimeLabel(at(2026, 3, 2, 9, 41), 'en-GB', NOW)).toBe('02/03, 09:41');
  });

  it('shows the year for older messages', () => {
    expect(formatMessageTimeLabel(at(2025, 12, 31, 23, 5), 'en-GB', NOW)).toBe('31/12/2025, 23:05');
  });

  it('follows the locale clock (12-hour locales keep AM/PM)', () => {
    expect(formatMessageTimeLabel(at(2026, 10, 30, 9, 41), 'en-US', NOW)).toBe('09:41 AM');
    expect(formatMessageTimeLabel(at(2026, 10, 30, 21, 41), 'de-DE', NOW)).toBe('21:41');
  });

  it('returns null for missing / invalid timestamps', () => {
    expect(formatMessageTimeLabel(undefined, 'en-GB', NOW)).toBeNull();
    expect(formatMessageTimeLabel(0, 'en-GB', NOW)).toBeNull();
    expect(formatMessageTimeLabel(Number.NaN, 'en-GB', NOW)).toBeNull();
  });

  it('caches a label until the day rolls over', () => {
    const ts = at(2026, 10, 30, 9, 41);
    expect(formatMessageTimeLabel(ts, 'en-GB', NOW)).toBe('09:41');
    expect(formatMessageTimeLabel(ts, 'en-GB', new Date(2026, 10, 1, 12, 0).getTime())).toBe('30/10, 09:41');
  });
});
