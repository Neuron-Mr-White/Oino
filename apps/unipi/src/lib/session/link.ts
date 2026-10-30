/**
 * Session link — WP-A1 placeholder for the v1 chat transport (herdr
 * `chat_list` / bridge `chat_open`, both deleted with the v1 host protocol).
 *
 * - `list(hostId)` is the host's `sessions_list` (host.md §7.2) through the
 *   HostClient seam (`lib/host`).
 * - `open(...)` is the line-oriented link the current chat store
 *   (`lib/chat/store.ts`, bridge-message reducer) still speaks. The real host
 *   session stream (`HostClient.openSession`, WP-H4) does not carry bridge
 *   lines, so by default it reports "not available yet"; WP-A2 replaces the
 *   store and this module with `lib/session/{store,stream}.ts`.
 * - Tests inject a fake with `setChatTransport()`.
 */
import { hostClient } from '../host/factory';
import type { SessionCard } from '../host/client';

export type AgentStatus = 'idle' | 'working' | 'blocked' | 'done' | 'unknown';

/** `sessions_list` reply (host.md §7.2). */
export type SessionListData = { sessions: SessionCard[] };

export type ChatStreamState = 'open' | 'closed' | 'error';

export interface ChatConnection {
  send(line: string): Promise<void>;
  close(): void;
}

export interface ChatTransport {
  list(hostId: string): Promise<SessionListData>;
  open(
    hostId: string,
    session: string,
    onLine: (line: string) => void,
    onState: (state: ChatStreamState, message?: string, code?: string) => void,
  ): ChatConnection;
}

/** The words the chat shows until the session stream (WP-H4/A2) lands. */
export const SESSION_LINK_PENDING = 'Opening a session needs the new host session stream (coming in this update).';

export const hostSessionTransport: ChatTransport = {
  async list(hostId) {
    const data = await hostClient(hostId).request<SessionListData | SessionCard[] | null>('sessions_list');
    if (Array.isArray(data)) return { sessions: data };
    return { sessions: data?.sessions ?? [] };
  },
  open(_hostId, _session, _onLine, onState) {
    queueMicrotask(() => onState('error', SESSION_LINK_PENDING, 'E_UNSUPPORTED'));
    return { send: async () => {}, close: () => {} };
  },
};

let override: ChatTransport | undefined;
export function setChatTransport(transport: ChatTransport | undefined): void {
  override = transport;
}
export function chatTransport(): ChatTransport {
  return override ?? hostSessionTransport;
}
