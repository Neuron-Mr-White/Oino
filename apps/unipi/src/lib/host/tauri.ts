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
 * `{kind:"hub_event", event}`. Terminals (`term_open`, WP-A31) and language
 * servers (`lsp_open`) stream kind-1 bytes on a raw `onData` channel and
 * their lifecycle on `onEvent`.
 */
import { Channel, invoke } from '@tauri-apps/api/core';
import { createSignal } from 'solid-js';
import { appError, toAppError, type AppError } from '@unipi/protocol';
import { LspFrameParser, encodeMessage } from '../lsp/framing';
import { localHostId, localIdOf, setLocalHostId } from './localId';
import {
  APP_HOST,
  LOCAL_HOST,
  isUnreachableCode,
  unavailableError,
  type LspRequest,
  type Closable,
  type EventsStream,
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
  type TermRequest,
  type TermSink,
  type TermStream,
} from './client';

let lookingUp: Promise<string | undefined> | undefined;
/** The real id of This computer (asks the core once when nothing recorded it yet). */
function lookUpLocal(): Promise<string | undefined> {
  lookingUp ??= invoke<Array<{ id: string; transport?: string }>>('list_hosts', {})
    .then((list) => {
      const id = localIdOf(Array.isArray(list) ? list : []);
      setLocalHostId(id);
      return id;
    })
    .catch(() => undefined)
    .finally(() => (lookingUp = undefined));
  return lookingUp;
}

/**
 * Runs `f` with the id the core knows the host by: `local` → This computer's
 * real id (synchronously when already known, so a real id never waits).
 */
export function withWireId<T>(hostId: string, f: (id: string) => Promise<T>): Promise<T> {
  if (hostId !== LOCAL_HOST) return f(hostId);
  const known = localHostId();
  if (known) return f(known);
  return lookUpLocal().then((id) => f(id ?? hostId));
}

export function createTauriHost(hostId: string): HostClient {
  const [conn, setConn] = createSignal<HostConn>({ state: hostId === APP_HOST ? 'connected' : 'connecting' });
  const request = async <T,>(op: string, args?: Record<string, unknown>): Promise<T> => {
    try {
      const command = op.startsWith('superagent.') ? op.replace('.', '_') : op;
      const value = (await (hostId === APP_HOST ? invoke(command, args) : withWireId(hostId, (id) => invoke(command, { hostId: id, ...(args ?? {}) })))) as T;
      if (conn().state !== 'connected') setConn({ state: 'connected' });
      return value;
    } catch (error) {
      const app = toAppError(error);
      if (isUnreachableCode(app.code)) setConn({ state: 'offline', message: app.message, hint: app.hint });
      throw error;
    }
  };
  // The supervisor's liveness watch on the host it runs (`host-liveness`): a frozen
  // host shows Offline (with Restart agent host) before any request has to time out,
  // and the state clears as soon as it answers again.
  if (hostId === LOCAL_HOST || (localHostId() !== undefined && hostId === localHostId())) {
    void import('@tauri-apps/api/event')
      .then(({ listen }) =>
        listen<{ answering: boolean; message: string | null }>('host-liveness', (e) => {
          if (e.payload.answering) {
            if (conn().state !== 'connected') setConn({ state: 'connected' });
          } else {
            setConn({ state: 'offline', message: e.payload.message ?? 'The PC stopped answering' });
          }
        }),
      )
      .catch(() => undefined);
  }
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
    openTerm(req: TermRequest, sink: TermSink): TermStream {
      return openTermChannel(hostId, req, sink);
    },
    openLsp(req: LspRequest, sink: LspSink): LspStream {
      return openLspChannel(hostId, req, sink);
    },
  };
}

/** One message of a `chat.rs` session / events channel (`SessionMsg`). */
export type SessionMsg =
  | { kind: 'snapshot'; snapshot: SessionSnapshot }
  | { kind: 'events'; items: SessionEventDto[] }
  | { kind: 'hub_event'; event: HubEvent }
  | { kind: 'resync'; reason?: string }
  | { kind: 'closed'; message?: string }
  | { kind: 'error'; message: string; code?: string };

/** `SessionMsg` `error` → an `AppError` (transport trouble stays retryable). */
export function channelError(msg: { message: string; code?: string }): AppError {
  const code = msg.code ?? 'E_TRANSPORT';
  return { code, message: msg.message, retryable: code === 'E_TRANSPORT' || code === 'E_TIMEOUT' } as AppError;
}

/** The `invoke` command + args that open a session stream (host.md §7.3). */
export function openArgs(hostId: string, req: OpenSessionRequest): { command: string; args: Record<string, unknown> } {
  if (req.kind === 'superagent') return { command: 'superagent_session', args: { hostId } };
  if (req.kind === 'new') return { command: 'session_new', args: { hostId, ...(req.cwd ? { cwd: req.cwd } : {}), ...(req.name ? { name: req.name } : {}) } };
  if (req.kind === 'fork') {
    return {
      command: 'session_fork',
      args: { hostId, session: req.session, ...(req.fromEntry ? { entryId: req.fromEntry } : {}), ...(req.name ? { name: req.name } : {}) },
    };
  }
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
      case 'resync':
        // fix-frames: the reader skipped a frame it could not use (oversize/unreadable);
        // the stream stays open, the session reloads from a fresh snapshot.
        sink.resync?.(msg.reason);
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
  const opened = withWireId(hostId, (id) => {
    const { command, args } = openArgs(id, req);
    return invoke<string>(command, { ...args, onEvent: channel });
  }).then(
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

function openEventsChannel(hostId: string, sink: (event: HubEvent) => void, closed?: (error?: AppError) => void): EventsStream {
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
  // The focus of the moment, sent as soon as the stream id resolves (§3.14).
  let pendingFocus: string | null | undefined;
  const sendFocus = (id: string, session: string | null) =>
    void invoke('events_focus', { streamId: id, session }).catch(() => {});
  void withWireId(hostId, (id) => invoke<string>('events_open', { hostId: id, onEvent: channel })).then(
    (id) => {
      streamId = id;
      if (pendingFocus !== undefined) sendFocus(id, pendingFocus);
      if (closing) hangUp('events_close', id);
    },
    (error: unknown) => end(toAppError(error)),
  );
  return {
    focus(session: string | null) {
      // §3.14: the focus rides this connection; a lost frame just means the
      // host keeps last known state until the next route change reports again.
      pendingFocus = session;
      if (!streamId || ended) return;
      sendFocus(streamId, session);
    },
    close() {
      closing = true;
      ended = true;
      if (streamId) hangUp('events_close', streamId);
    },
  };
}

/** Events the `term.rs` reader task emits on `onEvent` (src-tauri term.rs `TermEvent`). */
type TermEventMsg = { type: 'title'; title: string } | { type: 'exit'; code: number | null } | { type: 'closed'; error?: AppError };

const clampDim = (n: number, fallback: number): number => (Number.isFinite(n) ? Math.max(1, Math.min(1000, Math.round(n))) : fallback);

function bytesOf(raw: ArrayBuffer | Uint8Array | number[]): Uint8Array {
  return raw instanceof Uint8Array ? raw : new Uint8Array(raw as ArrayBuffer);
}

/**
 * One terminal stream (`term_open`, host.md §9.3): output arrives as raw bytes
 * on `onData`, `title`/`exit`/`closed` on `onEvent` (exactly one of the last two
 * ends it). Input and resizes go through `term_input` / `term_resize` one at a
 * time — concurrent invokes could reorder keystrokes — with input coalesced and
 * the size last-wins while a call is in flight. `close()` detaches
 * (`term_close`; the host keeps the shell), `kill()` ends it (`term_kill`).
 */
function openTermChannel(hostId: string, req: TermRequest, sink: TermSink): TermStream {
  let termId: string | undefined;
  let ended = false;
  let closing = false;
  let queued = '';
  let size: { cols: number; rows: number } | undefined;
  let busy = false;
  let killAfterOpen = false;
  const kill = (id: string) => withWireId(hostId, (wire) => invoke('term_kill', { hostId: wire, termId: id })).catch(() => {});
  const finish = (error?: AppError) => {
    if (ended) return;
    ended = true;
    if (sink.closed) sink.closed(error);
    else sink.exit();
  };
  const pump = () => {
    if (busy || !termId || ended || closing) return;
    let call: Promise<unknown> | undefined;
    if (queued) {
      const data = queued;
      queued = '';
      call = invoke('term_input', { termId, data });
    } else if (size) {
      const { cols, rows } = size;
      size = undefined;
      call = invoke('term_resize', { termId, cols, rows });
    }
    if (!call) return;
    busy = true;
    void call
      .catch((error: unknown) => {
        // The app lost this stream's writer (term.rs keys streams by terminal id, and a
        // stale stream from before a reload can drop the newer one): end it, so the view re-attaches.
        const app = toAppError(error);
        if (app.code === 'E_NOT_FOUND') finish(app);
      })
      .finally(() => {
        busy = false;
        pump();
      });
  };

  const onData = new Channel<ArrayBuffer | Uint8Array | number[]>();
  onData.onmessage = (raw) => {
    if (!ended && !closing) sink.data(bytesOf(raw));
  };
  const onEvent = new Channel<TermEventMsg>();
  onEvent.onmessage = (event) => {
    if (ended || closing) return;
    if (event.type === 'title') sink.title?.(event.title);
    else if (event.type === 'exit') {
      ended = true;
      sink.exit(event.code ?? undefined);
    } else if (event.type === 'closed') finish(event.error);
  };

  void withWireId(hostId, (id) =>
    invoke<{ termId: string; shell: string }>('term_open', {
      hostId: id,
      cwd: req.id ? undefined : (req.cwd ?? '~'),
      cols: clampDim(req.cols, 80),
      rows: clampDim(req.rows, 24),
      termId: req.id,
      shell: req.shell,
      onData,
      onEvent,
    }),
  ).then(
    (result) => {
      termId = result.termId;
      if (closing) {
        void (killAfterOpen ? kill(result.termId) : invoke('term_close', { termId }).catch(() => {}));
        return;
      }
      sink.opened?.({ termId: result.termId, shell: result.shell });
      pump();
    },
    (error: unknown) => {
      if (!closing) finish(toAppError(error));
    },
  );

  return {
    write(data: string) {
      if (ended || closing || !data) return;
      queued += data;
      pump();
    },
    resize(cols: number, rows: number) {
      if (ended || closing) return;
      size = { cols: clampDim(cols, 80), rows: clampDim(rows, 24) };
      pump();
    },
    close() {
      if (closing) return;
      closing = true;
      if (termId) void invoke('term_close', { termId }).catch(() => {});
    },
    async kill() {
      const id = termId ?? req.id;
      closing = true;
      if (!id) {
        killAfterOpen = true;
        return;
      }
      await kill(id);
    },
  };
}

/** Events the `lsp.rs` reader task emits on `onEvent` (src-tauri lsp.rs `LspEvent`). */
type LspEventMsg =
  | { type: 'taken'; code: string; message: string }
  | { type: 'exit'; code: number | null }
  | { type: 'closed'; error?: AppError };

let lspSeq = 0;

/**
 * One LSP stream (`lsp_open`): kind-1 bytes → `Content-Length` framed
 * messages → `sink.message` as JSON strings; `send` frames the client's
 * JSON-RPC and goes via `lsp_write`; `lsp_close` detaches (the host keeps
 * the server warm). The `lsp_open` reply's attach data reaches
 * `sink.attached` once (§3.12).
 */
function openLspChannel(hostId: string, req: LspRequest, sink: LspSink): LspStream {
  const lspId = `lsp-${++lspSeq}`;
  const parser = new LspFrameParser();
  let ended = false;
  let closing = false;
  const pending: string[] = [];
  let openedOk = false;
  const end = (error?: AppError) => {
    if (ended) return;
    ended = true;
    sink.closed(error);
  };

  const onData = new Channel<Uint8Array>();
  onData.onmessage = (bytes) => {
    if (ended) return;
    try {
      for (const message of parser.push(bytes)) sink.message(message);
    } catch (error) {
      end(toAppError(error));
    }
  };
  const onEvent = new Channel<LspEventMsg>();
  onEvent.onmessage = (event) => {
    if (ended) return;
    switch (event.type) {
      case 'taken':
        end(appError('E_LSP_TAKEN', event.message));
        return;
      case 'exit':
        end(appError('E_TRANSPORT', `The language server exited${event.code === null ? '' : ` (${event.code})`}`, { retryable: false }));
        return;
      case 'closed':
        end(event.error);
        return;
      default:
        return;
    }
  };

  const write = (body: string) => invoke('lsp_write', { lspId, data: Array.from(encodeMessage(body)) }).catch(() => {});
  const opened = withWireId(hostId, (id) =>
    invoke<{ server: string; attached: boolean; initializeResult: unknown }>('lsp_open', {
      hostId: id,
      lspId,
      root: req.root,
      language: req.language,
      onData,
      onEvent,
    }),
  ).then(
    (result) => {
      openedOk = true;
      sink.attached?.({ server: result.server, attached: result.attached, initialize_result: result.initializeResult });
      for (const body of pending.splice(0)) void write(body);
      if (closing) void invoke('lsp_close', { lspId }).catch(() => {});
    },
    (error: unknown) => end(toAppError(error)),
  );

  return {
    send(json: unknown) {
      if (ended) return;
      const body = typeof json === 'string' ? json : JSON.stringify(json);
      if (openedOk) void write(body);
      else {
        pending.push(body);
        void opened;
      }
    },
    close() {
      closing = true;
      ended = true;
      if (openedOk) void invoke('lsp_close', { lspId }).catch(() => {});
    },
  };
}
