/**
 * Board transport — the ONE seam between the Board UI and a PC's
 * unipi-kanboard daemon (DESIGN.md §3.9 data path). The UI never calls
 * fetch/EventSource/invoke directly; it goes through `boardTransport()`.
 *
 * - Tauri (phone/desktop app): Rust core → iroh → unipi-host → local
 *   daemon. Commands: `board_availability`, `board_request`,
 *   `board_events` (+ `board_events_close`). Implemented by UNI-129.
 * - Web mode (no Tauri, lib/platform.ts `web`): fetch/EventSource to the
 *   kanboard daemon's JSON API as ONE synthetic PC, id `dev`:
 *   - `vite dev`: base `/kb`, which vite proxies to `UNIPI_KB_DEV_URL`
 *     (default http://127.0.0.1:7788 — e.g. an ssh tunnel to a PC's daemon);
 *   - the production web build served BY the kanboard daemon: same origin,
 *     base `` (the daemon's own `/api/…` + `/events`). A remote (token-gated)
 *     daemon is opened once with `?t=<token>`; it answers with an HttpOnly
 *     `kb_token` cookie that every same-origin fetch/EventSource then sends.
 *   `window.__UNIPI_KB_BASE__` overrides the base (tests, odd deployments).
 *
 * Paths are daemon paths (`/api/projects`, `/api/tasks/<slug>/<id>/move`,
 * `/api/files/…`). Writes are always user-actor (the daemon's default).
 */
import { Channel, invoke } from '@tauri-apps/api/core';

export type BoardMethod = 'GET' | 'POST' | 'PUT';

export type BoardRequest = {
  method: BoardMethod;
  /** Daemon path incl. query, e.g. `/api/projects/foo-1a2b3c/tasks`. */
  path: string;
  /** JSON text (content-type application/json) or raw bytes (octet-stream upload). */
  body?: string | Uint8Array;
};

export type BoardResponse = {
  status: number;
  contentType: string;
  body: Uint8Array;
};

export type BoardAvailability =
  | { state: 'ok'; version?: string }
  | { state: 'offline' }
  | { state: 'not_installed'; hint: string }
  | { state: 'error'; message: string };

/** One SSE message from `/events?project=<slug>`. */
export type BoardEvent = { event?: string; data: string };
export type BoardStreamState = 'open' | 'closed' | 'error';

export interface BoardTransport {
  /** Is the board reachable on this PC (daemon auto-started by the host if needed)? */
  availability(hostId: string): Promise<BoardAvailability>;
  request(hostId: string, req: BoardRequest): Promise<BoardResponse>;
  /** Subscribes to a project's revision stream. Returns an unsubscribe. */
  events(
    hostId: string,
    project: string,
    onEvent: (event: BoardEvent) => void,
    onState: (state: BoardStreamState, message?: string) => void,
  ): () => void;
}

export const DEV_HOST_ID = 'dev';

/** @deprecated use `isTauri` from lib/platform.ts (kept for old imports). */
export { isTauri } from '../platform';
import { isTauri } from '../platform';

/**
 * Where the web-mode board talks to: `/kb` under `vite dev` (proxied to a
 * daemon), `` (same origin) in a production web build served by the
 * kanboard daemon, or `window.__UNIPI_KB_BASE__`.
 */
export function webBoardBase(): string {
  const forced = typeof window !== 'undefined' ? (window as { __UNIPI_KB_BASE__?: string }).__UNIPI_KB_BASE__ : undefined;
  if (typeof forced === 'string') return forced.replace(/\/$/, '');
  return import.meta.env.DEV ? '/kb' : '';
}

/** Is the web build being served by the kanboard daemon itself (not vite)? */
export function servedByDaemon(): boolean {
  return webBoardBase() === '';
}

export const TOKEN_HINT = 'This board needs its access token. Open the link printed by `/unipi:kanboard open` (it ends in ?t=…) once in this browser.';

const textEncoder = new TextEncoder();

/** Web transport: same-origin daemon (or the vite `/kb` proxy). */
export const devTransport: BoardTransport = {
  async availability() {
    const base = webBoardBase();
    try {
      const res = await fetch(`${base}/api/health`, { credentials: 'same-origin' });
      if (!res.ok) return { state: 'offline' };
      const json = (await res.json().catch(() => ({}))) as { version?: string };
      // /api/health is open even on a token-gated daemon: probe a gated route.
      const gate = await fetch(`${base}/api/rules`, { credentials: 'same-origin' }).catch(() => undefined);
      if (gate && gate.status === 401) return { state: 'error', message: TOKEN_HINT };
      return { state: 'ok', version: json.version };
    } catch {
      return { state: 'offline' };
    }
  },
  async request(_hostId, req) {
    const body = req.body;
    const res = await fetch(`${webBoardBase()}${req.path}`, {
      method: req.method,
      credentials: 'same-origin',
      headers:
        body === undefined
          ? undefined
          : { 'content-type': typeof body === 'string' ? 'application/json' : 'application/octet-stream' },
      body: body === undefined ? undefined : typeof body === 'string' ? body : new Blob([body as BlobPart]),
    });
    // The daemon's token gate answers an HTML page: say what to do instead
    // (the cookie expired, the token was rotated, or the tab never had it).
    if (res.status === 401) {
      return { status: 401, contentType: 'application/json', body: textEncoder.encode(JSON.stringify({ error: TOKEN_HINT })) };
    }
    return {
      status: res.status,
      contentType: res.headers.get('content-type') ?? '',
      body: new Uint8Array(await res.arrayBuffer()),
    };
  },
  events(_hostId, project, onEvent, onState) {
    const source = new EventSource(`${webBoardBase()}/events?project=${encodeURIComponent(project)}`, { withCredentials: true });
    source.onopen = () => onState('open');
    source.onerror = () => onState(source.readyState === EventSource.CLOSED ? 'closed' : 'error');
    source.onmessage = (message) => onEvent({ data: String(message.data) });
    // The daemon names its events `revision`; onmessage only sees unnamed ones.
    source.addEventListener('revision', (message) => onEvent({ event: 'revision', data: String((message as MessageEvent).data) }));
    return () => source.close();
  },
};

/** Tauri transport (Rust core → iroh → unipi-host). */
export const tauriTransport: BoardTransport = {
  async availability(hostId) {
    try {
      return (await invoke('board_availability', { hostId })) as BoardAvailability;
    } catch (error) {
      return { state: 'error', message: String((error as { message?: string })?.message ?? error) };
    }
  },
  async request(hostId, req) {
    const body = req.body === undefined ? undefined : typeof req.body === 'string' ? textEncoder.encode(req.body) : req.body;
    const res = (await invoke('board_request', {
      hostId,
      method: req.method,
      path: req.path,
      contentType: req.body === undefined ? null : typeof req.body === 'string' ? 'application/json' : 'application/octet-stream',
      body: body ? Array.from(body) : null,
    })) as { status: number; contentType: string; body: number[] };
    return { status: res.status, contentType: res.contentType, body: Uint8Array.from(res.body) };
  },
  events(hostId, project, onEvent, onState) {
    const channel = new Channel<{ kind: 'open' } | { kind: 'event'; event?: string; data: string } | { kind: 'closed' | 'error'; message?: string }>();
    let streamId: string | undefined;
    let closed = false;
    channel.onmessage = (message) => {
      if (message.kind === 'event') onEvent({ event: message.event, data: message.data });
      else onState(message.kind, 'message' in message ? message.message : undefined);
    };
    invoke('board_events', { hostId, project, onEvent: channel })
      .then((id) => {
        streamId = String(id);
        if (closed) void invoke('board_events_close', { streamId }).catch(() => {});
      })
      .catch((error) => onState('error', String((error as { message?: string })?.message ?? error)));
    return () => {
      closed = true;
      if (streamId) void invoke('board_events_close', { streamId }).catch(() => {});
    };
  },
};

let override: BoardTransport | undefined;
/** Tests inject a fake transport. */
export function setBoardTransport(transport: BoardTransport | undefined): void {
  override = transport;
}
export function boardTransport(): BoardTransport {
  return override ?? (isTauri() ? tauriTransport : devTransport);
}
