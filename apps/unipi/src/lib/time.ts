/**
 * Shared time formatting (WP-A4): the sidebar rows' relative time (Z-10).
 * Board keeps its own ISO-based `relativeTime` and git its unix-seconds one;
 * this one takes epoch milliseconds (session cards' `updated_at`).
 */

/** "3m ago", "2h ago", "5d ago" — short, tabular-friendly. */
export function relativeTimeMs(ms: number | undefined, now = Date.now()): string {
  if (!ms) return '';
  const seconds = Math.max(0, Math.round((now - ms) / 1000));
  if (seconds < 45) return 'just now';
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.round(hours / 24)}d ago`;
}
