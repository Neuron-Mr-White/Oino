/**
 * Command reports (P1-02): a multi-line or long extension `notify`
 * (`/mcp-status`, `/memory status`, `/memory-search`, `/compact-stats`,
 * `/doctor`, `/cron list`…) is a report to read, not a 3-second toast.
 * Such a notify opens the report panel (`components/ReportSheet.tsx`) when
 * it answers a command the user just ran in that session; otherwise it
 * shows a one-line toast with "Show" that opens the same panel.
 * Short one-line notices stay toasts.
 */
import { createSignal } from 'solid-js';
import { toasts, type ToastKind } from './toasts';

export type Report = { id: number; title: string; body: string; kind: ToastKind; at: number };

/** A notify longer than this (or with a newline) is a report. */
export const REPORT_MIN_CHARS = 160;
/** A report arriving within this window of a command run in that session answers it. */
export const REPORT_COMMAND_WINDOW_MS = 30_000;

/** True when a notify message is a report, not a toast. Pure. */
export function isReport(message: string): boolean {
  const text = message.trim();
  return text.includes('\n') || text.length > REPORT_MIN_CHARS;
}

const [current, setCurrent] = createSignal<Report | null>(null);
const recent = new Map<string, { command: string; at: number }>();
let seq = 0;

/** The report the panel shows (null = closed). */
export const openReport = current;
export const closeReport = (): void => {
  setCurrent(null);
};

/** The composer notes the `/command args` line it just ran in a session (the report's title, no `/`). */
export function noteCommandRun(sessionKey: string, command: string, now = Date.now()): void {
  recent.set(sessionKey, { command, at: now });
}

/** First non-empty line, trimmed to fit a toast. Pure. */
export function reportHeadline(body: string): string {
  const line = body.split('\n').map((l) => l.trim()).find(Boolean) ?? '';
  return line.length > 90 ? `${line.slice(0, 89)}…` : line;
}

/**
 * Routes one notify message: a report opens the panel (answering a command)
 * or toasts its first line with "Show"; anything else is a plain toast.
 */
export function routeNotify(message: string, kind: ToastKind, sessionKey?: string, now = Date.now()): void {
  if (!isReport(message)) {
    toasts().push(message, kind);
    return;
  }
  const ran = sessionKey ? recent.get(sessionKey) : undefined;
  const answering = !!ran && now - ran.at <= REPORT_COMMAND_WINDOW_MS;
  const report: Report = {
    id: ++seq,
    title: answering ? `/${ran!.command}` : reportHeadline(message) || 'Report',
    body: message.replace(/\s+$/, ''),
    kind,
    at: now,
  };
  if (answering) {
    setCurrent(report);
    return;
  }
  toasts().push(reportHeadline(message), kind, { label: 'Show', run: () => void setCurrent(report) });
}

/** Tests: forget everything. */
export function resetReports(): void {
  recent.clear();
  setCurrent(null);
}
