/**
 * `HostClient` over the src-tauri commands (app spec §3.2). One-shot ops are
 * `invoke(op, {hostId, ...args})` (app-level ops omit `hostId`).
 *
 * Session streams (WP-A2 on H4's `chat.rs`): `session_open|new|fork` take a
 * `Channel<SessionMsg>` and return a stream id; the channel carries
 * `{kind:"snapshot"}` first, then `{kind:"events", items}`, and exactly one
 * `{kind:"closed"|"error"}`. `session_cmd{streamId, cmd, id}` resolves with
 * the command's data (chat.rs correlates by id and enforces 30 s / 2 s);
 * `session_stream_close` hangs up locally. `events_open` streams
 * `{kind:"hub_event", event}`. Terminal and LSP streams wait for H6 / A21
 * and report `E_UNSUPPORTED` through their sinks.
 */
import { Channel, invoke } from '@tauri-apps/api/core';
import { createSignal } from 'solid-js';
import { toAppError, type AppError } from '@unipi/protocol';
import {
  APP_HOST,
  isUnreachableCode,
  unavailableError,
  type Closable,
  type HostClient,
  type HostConn,
  type HubEvent,
  type OpenSessionRequest,
  type RpcCommand,
  type SessionEventDto,
  type SessionSnapshot,
  type LspSink,
  type LspStream,
  type SessionSink,
  type SessionStream,
  type TermSink,
  type TermStream,
} from './client';

export function createTauriHost(hostId: string): HostClient {
  const [conn, setConn] = createSignal<HostConn>({ state: hostId === APP_HOST ? 'connected' : 'connecting' });
  const request = async <T,>(op: string, args?: Record<string, unknown>): Promise<T> => {
    const payload = hostId === APP_HOST ? args : { hostId, ...(args ?? {}) };
    try {
      const value = (await invoke(op, payload)) as T;
      if (conn().state !== 'connected') setConn({ state: 'connected' });
      return value;
    } catch (error) {
      const app = toAppError(error);
      if (isUnreachableCode(app.code)) setConn({ state: 'offline', message: app.message, hint: app.hint });
      throw error;
    }
  };
  return {
    hostId,
    conn,
    async retry() {
      if (hostId === APP_HOST) return;
      setConn({ state: 'connecting' });
      try {
        await request('host_info');
      } catch {
        // `request` already recorded the state.
      }
    },
    request,
    openSession(req, sink: SessionSink): SessionStream {
      return openSessionChannel(hostId, req, sink);
    },
    openEvents(sink, closed): Closable {
      return openEventsChannel(hostId, sink, closed);
    },
    openTerm(_req, sink: TermSink): TermStream {
      queueMicrotask(() => sink.exit());
      return { write() {}, resize() {}, close() {} };
    },
    openLsp(_req, sink: LspSink): LspStream {
      queueMicrotask(() => sink.closed(unavailableError('Language servers')));
      return { send() {}, close() {} };
    },
  };
}

/** One message of a `chat.rs` session / events channel (`SessionMsg`). */
export type SessionMsg =
  | { kind: 'snapshot'; snapshot: SessionSnapshot }
  | { kind: 'events'; items: SessionEventDto[] }
  | { kind: 'hub_event'; event: HubEvent }
  | { kind: 'closed'; message?: string }
  | { kind: 'error'; message: string; code?: string };

/** `SessionMsg` `error` → an `AppError` (transport trouble stays retryable). */
export function channelError(msg: { message: string; code?: string }): AppError {
  const code = msg.code ?? 'E_TRANSPORT';
  return { code, message: msg.message, retryable: code === 'E_TRANSPORT' || code === 'E_TIMEOUT' } as AppError;
}

/** The `invoke` command + args that open a session stream (host.md §7.3). */
export function openArgs(hostId: string, req: OpenSessionRequest): { command: string; args: Record<string, unknown> } {
  if (req.kind === 'new') return { command: 'session_new', args: { hostId, ...(req.cwd ? { cwd: req.cwd } : {}) } };
  if (req.kind === 'fork') return { command: 'session_fork', args: { hostId, session: req.session, ...(req.fromEntry ? { entryId: req.fromEntry } : {}) } };
  return { command: 'session_open', args: { hostId, session: req.session, ...(req.sinceSeq !== undefined ? { sinceSeq: req.sinceSeq } : {}) } };
}

let cmdSeq = 0;

/** Local hang-up; a stream that is already gone is fine. */
function hangUp(command: 'session_stream_close' | 'events_close', streamId: string): void {
  void Promise.resolve()
    .then(() => invoke(command, { streamId }))
    .catch(() => {});
}

function openSessionChannel(hostId: string, req: OpenSessionRequest, sink: SessionSink): SessionStream {
  let streamId: string | undefined;
  let ended = false;
  let closing = false;
  const end = (error?: AppError) => {
    if (ended) return;
    ended = true;
    sink.closed(error);
  };
  const channel = new Channel<SessionMsg>();
  channel.onmessage = (msg) => {
    if (ended) return;
    switch (msg.kind) {
      case 'snapshot':
        sink.snapshot(msg.snapshot);
        return;
      case 'events':
        sink.events(msg.items);
        return;
      case 'closed':
        end(msg.message ? channelError({ message: msg.message }) : undefined);
        return;
      case 'error':
        end(channelError(msg));
        return;
      default:
        return;
    }
  };
  const { command, args } = openArgs(hostId, req);
  const opened = invoke<string>(command, { ...args, onEvent: channel }).then(
    (id) => {
      streamId = id;
      if (closing) hangUp('session_stream_close', id);
      return id;
    },
    (error: unknown) => {
      end(toAppError(error));
      return undefined;
    },
  );
  return {
    async cmd<T>(cmd: RpcCommand): Promise<T> {
      const id = streamId ?? (await opened);
      if (!id || ended) throw { code: 'E_TRANSPORT', message: 'The session is not connected', retryable: true };
      cmdSeq += 1;
      return invoke<T>('session_cmd', { streamId: id, cmd, id: cmdSeq });
    },
    close() {
      closing = true;
      ended = true;
      if (streamId) hangUp('session_stream_close', streamId);
    },
  };
}

function openEventsChannel(hostId: string, sink: (event: HubEvent) => void, closed?: (error?: AppError) => void): Closable {
  let streamId: string | undefined;
  let ended = false;
  let closing = false;
  const end = (error?: AppError) => {
    if (ended) return;
    ended = true;
    closed?.(error);
  };
  const channel = new Channel<SessionMsg>();
  channel.onmessage = (msg) => {
    if (ended) return;
    if (msg.kind === 'hub_event') sink(msg.event);
    else if (msg.kind === 'closed') end();
    else if (msg.kind === 'error') end(channelError(msg));
  };
  void invoke<string>('events_open', { hostId, onEvent: channel }).then(
    (id) => {
      streamId = id;
      if (closing) hangUp('events_close', id);
    },
    (error: unknown) => end(toAppError(error)),
  );
  return {
    close() {
      closing = true;
      ended = true;
      if (streamId) hangUp('events_close', streamId);
    },
  };
}
