/**
 * HostClient seam (app spec §3.2, WP-A1). Every call the UI makes to a host —
 * one-shot ops (fs, git, blob, push, forward, sharing, cron, pairing), the
 * session streams, the hub event stream, terminals and language servers —
 * goes through one `HostClient` per host, so the capture harness and the
 * component tests can stand in a mock host (`src/dev/mock/host.ts`).
 *
 * Components never call `invoke` directly. `lib/host/tauri.ts` implements
 * this over the src-tauri commands; `lib/host/mock.ts` re-exports the mock.
 *
 * Session/hub types are `@unipi/protocol` `session.ts` (WP-H4); event
 * payloads stay loosely typed and are narrowed by `type` in the session
 * reducer (`lib/session/reducer.ts`, WP-A2; ui-protocol D-U14).
 */
import type { Accessor } from 'solid-js';
import {
  appError,
  toAppError,
  type AppError,
  type HubEvent as ProtoHubEvent,
  type SessionCard as ProtoSessionCard,
  type SessionEventDto as ProtoSessionEventDto,
  type SessionSnapshot as ProtoSessionSnapshot,
} from '@unipi/protocol';
import type { z } from 'zod';

/** The pseudo host id for app-level ops (paired hosts list, local forwards,
 *  this PC's sharing switches, save-as dialogs): no `hostId` is sent. */
export const APP_HOST = 'app';
/** "This computer" (the PC app's own host over its local socket). */
export const LOCAL_HOST = 'local';

/** A host connection, as every host-dependent screen shows it (Z-90). */
export type HostConnState = 'connected' | 'connecting' | 'offline';
export type HostConn = { state: HostConnState; message?: string; hint?: string };

export interface Closable {
  close(): void;
}

/** `session_open|new|fork` (host.md §7.3). */
export type OpenSessionRequest = {
  kind: 'open' | 'new' | 'fork';
  session?: string;
  sinceSeq?: number;
  cwd?: string;
  fromEntry?: string;
  name?: string;
};

/** Any oino RPC command verbatim (`{type:'prompt', …}`, `{type:'_oino/render', …}`). */
export type RpcCommand = { type: string; [key: string]: unknown };

/** First frame of a session stream (host.md §7.3, `@unipi/protocol` `session.ts`, WP-H4). */
export type SessionSnapshot = ProtoSessionSnapshot;

/** `{session, seq, event: <ClientEvent JSON>}` (host.md §7.4). */
export type SessionEventDto = ProtoSessionEventDto;

export interface SessionSink {
  snapshot(snapshot: SessionSnapshot): void;
  events(items: SessionEventDto[]): void;
  /** The stream ended (`error` set when it failed; the store reconnects with `sinceSeq`). */
  closed(error?: AppError): void;
  /** The transport skipped a frame it could not use (oversize/unreadable): events may be
   *  missing, so the client reopens with a full snapshot. The stream itself is still open. */
  resync?(reason?: string): void;
}

export interface SessionStream extends Closable {
  cmd<T = unknown>(cmd: RpcCommand): Promise<T>;
}

/** Hub events on the per-host `events_open` stream (host.md §7.4). */
export type HubEvent = ProtoHubEvent;

/** One row of `sessions_list` / a `session_changed` card (host.md §4.2). */
export type SessionCard = ProtoSessionCard;

/** `term_open` (host.md §9.3): a new PTY in `cwd`, or — `id` set — a re-attach to that terminal. */
export type TermRequest = { cwd?: string; cols: number; rows: number; id?: string; shell?: string };
export interface TermSink {
  data(bytes: Uint8Array | string): void;
  /** The PTY's child exited (the stream ends). */
  exit(code?: number): void;
  /** The host answered: the terminal's id (keep it to re-attach) and shell. */
  opened?(info: { termId: string; shell: string }): void;
  title?(title: string): void;
  /** The stream ended without an exit (open failed, host gone, transport lost);
   *  the PTY may still be alive — re-attach with `id`. Falls back to `exit()`. */
  closed?(error?: AppError): void;
}
export interface TermStream extends Closable {
  /** Input; ordered, coalesced while a write is in flight. */
  write(data: string): void;
  /** Last-wins resize. */
  resize(cols: number, rows: number): void;
  /** `close()` only detaches (the host keeps the shell); `kill()` ends the PTY. */
  kill?(): Promise<void>;
}

export type LspRequest = { root: string; language: string };
export interface LspSink {
  /** The `lsp_open` reply's `LspAttachData` (`{server, attached,
   *  initialize_result}` — app spec §3.12): fires once when the attach
   *  succeeded, before any `message`. Optional for test doubles. */
  attached?(data: unknown): void;
  message(json: unknown): void;
  closed(error?: AppError): void;
}
export interface LspStream extends Closable {
  send(json: unknown): void;
}

/** The hub event stream of one host (`events_open`), plus the connection's focus. */
export interface EventsStream extends Closable {
  /** Reports which session this connection has focused (`null` = none, app spec §3.14);
  *  the host clears the session's `unread` and counts the focus for Z-86. Optional so a
  *  minimal test double (a bare `Closable`) still satisfies the seam. */
  focus?(session: string | null): void;
}

export interface HostClient {
  /** `local` = This computer, `app` = app-level ops, else a paired host id. */
  readonly hostId: string;
  /** Connection state for the host-dependent screens (NoHost/Offline states, Z-90). */
  readonly conn: Accessor<HostConn>;
  /** Re-checks the connection (the Offline state's Retry). */
  retry(): Promise<void>;
  /** One-shot ops; rejects with an `AppError`-shaped value. */
  request<T = unknown>(op: string, args?: Record<string, unknown>): Promise<T>;
  openSession(req: OpenSessionRequest, sink: SessionSink): SessionStream;
  /** The hub event stream; `closed` fires once when it ends (`error` when it failed). */
  openEvents(sink: (event: HubEvent) => void, closed?: (error?: AppError) => void): EventsStream;
  openTerm(req: TermRequest, sink: TermSink): TermStream;
  openLsp(req: LspRequest, sink: LspSink): LspStream;
}

export type HostResult<T, E = AppError> = { ok: true; value: T } | { ok: false; error: E };

/** `request()` + zod parse of the reply, as a Result (what the `lib/*` op wrappers return). */
export async function requestParsed<T>(client: HostClient, op: string, args: Record<string, unknown> | undefined, schema: z.ZodType<T>): Promise<HostResult<T>> {
  try {
    const parsed = schema.safeParse(await client.request(op, args));
    return parsed.success ? { ok: true, value: parsed.data } : { ok: false, error: toAppError(parsed.error) };
  } catch (error) {
    return { ok: false, error: toAppError(error) };
  }
}

/** Error codes that mean "the host can't be reached" (Offline state, not a generic error). */
export function isUnreachableCode(code: string | undefined): boolean {
  return code === 'E_TRANSPORT' || code === 'E_TIMEOUT';
}

/** An `AppError` for a stream/op the current host build does not offer yet. */
export function unavailableError(what: string): AppError {
  return appError('E_UNSUPPORTED', `${what} needs a newer host.`, { hint: 'Update Unipi on the PC, then try again.' });
}
