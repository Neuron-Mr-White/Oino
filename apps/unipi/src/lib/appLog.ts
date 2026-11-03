/**
 * chat-vanish: the webview's own lines in the app log (`~/.local/share/dev.unipi.oino/logs`),
 * next to the shell's — so a chat self-heal, a crash boundary or an uncaught error leaves a
 * trace the owner's log copy shows. Goes through tauri-plugin-log's `plugin:log|log`
 * (`log:default` is granted); outside Tauri (web, tests) it only reaches the console.
 *
 * Bounded: one line is cut to 2 KB, and at most 60 lines a minute leave the webview (an error
 * thrown every frame must never flood the log or the IPC bridge).
 */
import { isTauri } from './platform';

export type AppLogLevel = 'info' | 'warn' | 'error';

/** tauri-plugin-log levels (the Rust `log` crate's). */
const LEVEL: Record<AppLogLevel, number> = { info: 3, warn: 4, error: 5 };
export const APP_LOG_LINE_MAX = 2_048;
export const APP_LOG_PER_MINUTE = 60;

type Invoke = (cmd: string, args: Record<string, unknown>) => Promise<unknown>;
let sink: Invoke | undefined;
let windowStart = 0;
let sentInWindow = 0;
let dropped = 0;

/** Tests: route lines somewhere observable (undefined = the default bridge). */
export function setAppLogSink(next: Invoke | undefined): void {
  sink = next;
  windowStart = 0;
  sentInWindow = 0;
  dropped = 0;
}

function bridge(): Invoke | undefined {
  if (sink) return sink;
  if (!isTauri()) return undefined;
  const internals = (globalThis as { __TAURI_INTERNALS__?: { invoke?: Invoke } }).__TAURI_INTERNALS__;
  return internals?.invoke ? (cmd, args) => internals.invoke!(cmd, args) : undefined;
}

/** One line in the app log (`[webview] <area>: <message>`). Never throws. */
export function appLog(level: AppLogLevel, area: string, message: string, now = Date.now()): void {
  const line = `[webview] ${area}: ${message}`.slice(0, APP_LOG_LINE_MAX);
  if (level === 'error') console.error(line);
  else if (level === 'warn') console.warn(line);
  else console.info(line);
  if (now - windowStart >= 60_000) {
    if (dropped > 0) {
      const note = `[webview] app-log: ${dropped} lines dropped (rate limit)`;
      dropped = 0;
      void send('warn', note);
    }
    windowStart = now;
    sentInWindow = 0;
  }
  if (sentInWindow >= APP_LOG_PER_MINUTE) {
    dropped++;
    return;
  }
  sentInWindow++;
  void send(level, line);
}

function send(level: AppLogLevel, message: string): Promise<void> {
  const invoke = bridge();
  if (!invoke) return Promise.resolve();
  return invoke('plugin:log|log', { level: LEVEL[level], message, location: 'webview' }).then(
    () => undefined,
    () => undefined,
  );
}

const describe = (value: unknown): string => {
  if (value instanceof Error) return `${value.name}: ${value.message}${value.stack ? `\n${value.stack.split('\n').slice(0, 6).join('\n')}` : ''}`;
  try {
    return typeof value === 'string' ? value : JSON.stringify(value);
  } catch {
    return String(value);
  }
};
export { describe as describeError };

let installed = false;
/** Uncaught errors and unhandled rejections → the app log (once per app). */
export function installGlobalErrorLog(target: Pick<Window, 'addEventListener'> | undefined = typeof window === 'undefined' ? undefined : window): void {
  if (installed || !target) return;
  installed = true;
  target.addEventListener('error', (e: Event) => {
    const ev = e as ErrorEvent;
    appLog('error', 'uncaught', describe(ev.error ?? ev.message));
  });
  target.addEventListener('unhandledrejection', (e: Event) => {
    appLog('error', 'unhandled-rejection', describe((e as PromiseRejectionEvent).reason));
  });
}
/** Tests only. */
export function resetGlobalErrorLog(): void {
  installed = false;
}
