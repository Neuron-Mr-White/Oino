/**
 * Test helpers for the Board (v2): an in-memory fake board transport
 * (`setBoardTransport(fake.transport)`) shaped like the CLI seam — handlers
 * keyed by subcommand argv prefix, every call recorded — plus task fixtures.
 * Used by vitest only. Live-update tests drive hub events through a fake
 * `HostClient` (`setHostClientFactory`) instead; see live.test.ts.
 */
import type { BoardAvailability, BoardCliOutcome, BoardTransport } from './transport';
import type { ProjectSummary, Task } from './api';

export type CliCall = { args: string[]; project?: string; hostId: string };

export type CliHandler = (call: CliCall) => unknown;

export type FakeBoardOptions = {
  /** Keyed by `args.join(' ')` up to and including the subcommand's positional ids — see `key()`. */
  handlers?: Record<string, CliHandler>;
  availability?: Record<string, BoardAvailability>;
  /** Fallback for unmatched calls (default: a `{ok:false, kind:'usage'}` refusal). */
  fallback?: CliHandler;
};

/** A handler's return: a payload (wrapped as success) or a ready outcome (`ok()`/`refused()`/a mock board's `BoardCliData`). */
function wrap(payload: unknown): BoardCliOutcome {
  if (payload && typeof payload === 'object') {
    const candidate = payload as { ok?: unknown; kind?: unknown; payload?: unknown };
    if (candidate.ok === false && typeof candidate.kind === 'string') return candidate as BoardCliOutcome;
    if (candidate.ok === true && 'payload' in candidate) return candidate as BoardCliOutcome;
  }
  return ok(payload);
}

/** The CLI's §5.5 shapes. */
export const ok = (payload: unknown): BoardCliOutcome => ({ ok: true, payload });
export const refused = (error: string, kind: 'usage' | 'not_found' | 'rule' | 'io' = 'rule'): BoardCliOutcome => ({
  ok: false,
  error,
  kind,
});

export function fakeBoard(options: FakeBoardOptions = {}) {
  const handlers: Record<string, CliHandler> = { ...(options.handlers ?? {}) };
  const calls: CliCall[] = [];
  const availability: Record<string, BoardAvailability> = { ...(options.availability ?? {}) };
  const transport: BoardTransport = {
    async availability(hostId) {
      return availability[hostId] ?? { state: 'ok', home: '/tmp/kanboard' };
    },
    async cli(hostId, args, project): Promise<BoardCliOutcome> {
      const call = { args, project, hostId };
      calls.push(call);
      const joined = args.join(' ');
      const handler =
        handlers[joined] ??
        Object.entries(handlers)
          .filter(([pattern]) => joined === pattern || joined.startsWith(`${pattern} `))
          .sort((a, b) => b[0].length - a[0].length)[0]?.[1];
      if (handler) return wrap(handler(call));
      if (options.fallback) return wrap(options.fallback(call));
      return refused(`no handler for ${joined}`, 'usage');
    },
  };
  /** Calls whose argv starts with `prefix`. */
  const sent = (prefix: string) => calls.filter((call) => call.args.join(' ').startsWith(prefix));
  return { transport, handlers, calls, availability, sent };
}

let seq = 0;
export function task(partial: Partial<Task> & { id: string }): Task {
  seq += 1;
  return {
    title: `Task ${partial.id}`,
    status: 'todo',
    priority: 'none',
    order: seq * 1000,
    deps: [],
    labels: [],
    created: `2026-10-0${1 + (seq % 8)}T10:00:00Z`,
    updated: '2026-10-07T10:00:00Z',
    activity: [],
    ready: true,
    ...partial,
  };
}

export function project(partial: Partial<ProjectSummary> & { slug: string }): ProjectSummary {
  return { name: partial.slug, counts: {}, ...partial };
}
