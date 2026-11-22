/**
 * Turn-tail timestamps (app spec WP-A7, Z-26; ZCode `v4/messageTimeLabel.ts`).
 *
 * One line, graduated by distance from today:
 *
 *   today         09:41
 *   yesterday     Yesterday 09:41
 *   this year     10/29, 09:41      (locale numeric month/day)
 *   older         10/29/2025, 09:41 (locale numeric year/month/day)
 *
 * The clock part is always the locale's `Intl` 2-digit hour + minute (so
 * `en-US` says `09:41 AM`); the day labels use the same locale's numeric
 * fields. Labels are cached per `(locale, timestamp, today)` — a turn row
 * remounts on every virtual-scroll pass and must not build a formatter each
 * time.
 */

const CACHE_LIMIT = 4_000;
const labels = new Map<string, string>();
const formatters = new Map<string, Intl.DateTimeFormat>();

type Kind = 'time' | 'monthDayTime' | 'yearMonthDayTime';

function formatter(locale: string, kind: Kind): Intl.DateTimeFormat {
  const key = `${locale}:${kind}`;
  const cached = formatters.get(key);
  if (cached) return cached;
  const options: Intl.DateTimeFormatOptions =
    kind === 'time'
      ? { hour: '2-digit', minute: '2-digit' }
      : kind === 'monthDayTime'
        ? { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' }
        : { year: 'numeric', month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' };
  const made = new Intl.DateTimeFormat(locale, options);
  formatters.set(key, made);
  return made;
}

const sameDay = (a: Date, b: Date): boolean =>
  a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();

const dayKey = (d: Date): string => `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;

/** The locale labels are formatted in: the document's language, else the runtime's. */
export function defaultLocale(): string {
  if (typeof document !== 'undefined' && document.documentElement.lang) return document.documentElement.lang;
  if (typeof navigator !== 'undefined' && navigator.language) return navigator.language;
  return 'en';
}

/**
 * `HH:MM` / `Yesterday HH:MM` / `M/D, HH:MM` / `Y/M/D, HH:MM` for a message
 * timestamp; `null` for a timestamp the session file doesn't have (history
 * entries written before timestamps were kept).
 */
export function formatMessageTimeLabel(timestamp: number | undefined, locale = defaultLocale(), nowTimestamp = Date.now()): string | null {
  if (timestamp === undefined || !Number.isFinite(timestamp) || timestamp <= 0) return null;
  const date = new Date(timestamp);
  const now = new Date(nowTimestamp);
  if (Number.isNaN(date.getTime()) || Number.isNaN(now.getTime())) return null;

  const cacheKey = `${locale}:${timestamp}:${dayKey(now)}`;
  const cached = labels.get(cacheKey);
  if (cached !== undefined) return cached;

  const time = formatter(locale, 'time').format(date);
  let label: string;
  if (sameDay(date, now)) label = time;
  else {
    const yesterday = new Date(now);
    yesterday.setDate(now.getDate() - 1);
    if (sameDay(date, yesterday)) label = `Yesterday ${time}`;
    else if (date.getFullYear() === now.getFullYear()) label = formatter(locale, 'monthDayTime').format(date);
    else label = formatter(locale, 'yearMonthDayTime').format(date);
  }

  if (labels.size >= CACHE_LIMIT) labels.clear();
  labels.set(cacheKey, label);
  return label;
}

/** Test hook. */
export function clearTimeLabelCache(): void {
  labels.clear();
  formatters.clear();
}
