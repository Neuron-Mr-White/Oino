import { afterEach, describe, expect, it, vi } from 'vitest';
import { APP_LOG_LINE_MAX, APP_LOG_PER_MINUTE, appLog, installGlobalErrorLog, resetGlobalErrorLog, setAppLogSink } from './appLog';

describe('appLog (chat-vanish: webview lines reach the app log)', () => {
  afterEach(() => {
    setAppLogSink(undefined);
    resetGlobalErrorLog();
    vi.restoreAllMocks();
  });

  it('sends one plugin:log line with the Rust log level', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    const sent: Array<[string, Record<string, unknown>]> = [];
    setAppLogSink(async (cmd, args) => void sent.push([cmd, args]));
    appLog('warn', 'chat-heal', 'reopened s1 (stream_lost)', 1_000);
    await Promise.resolve();
    expect(sent).toEqual([['plugin:log|log', { level: 4, message: '[webview] chat-heal: reopened s1 (stream_lost)', location: 'webview' }]]);
  });

  it('is bounded: lines are cut and at most APP_LOG_PER_MINUTE leave per minute', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    const sent: string[] = [];
    setAppLogSink(async (_cmd, args) => void sent.push(String(args.message)));
    for (let i = 0; i < APP_LOG_PER_MINUTE + 25; i++) appLog('error', 'loop', 'x'.repeat(5_000), 10_000 + i);
    expect(sent).toHaveLength(APP_LOG_PER_MINUTE);
    expect(sent[0]!.length).toBe(APP_LOG_LINE_MAX);
    // the next minute reports what was dropped
    appLog('error', 'loop', 'again', 80_000);
    expect(sent).toContain('[webview] app-log: 25 lines dropped (rate limit)');
  });

  it('a rejecting bridge never throws', async () => {
    vi.spyOn(console, 'info').mockImplementation(() => {});
    setAppLogSink(() => Promise.reject(new Error('no bridge')));
    expect(() => appLog('info', 'x', 'y')).not.toThrow();
    await Promise.resolve();
  });

  it('uncaught errors and unhandled rejections are logged', () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    const sent: string[] = [];
    setAppLogSink(async (_cmd, args) => void sent.push(String(args.message)));
    const target = new EventTarget();
    installGlobalErrorLog(target as unknown as Window);
    const err = new ErrorEvent('error', { error: new Error('boom'), message: 'boom' });
    target.dispatchEvent(err);
    const rej = new Event('unhandledrejection') as Event & { reason?: unknown };
    rej.reason = 'nope';
    target.dispatchEvent(rej);
    expect(sent[0]).toMatch(/^\[webview\] uncaught: Error: boom/);
    expect(sent[1]).toBe('[webview] unhandled-rejection: nope');
  });
});
