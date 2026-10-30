/**
 * `HostClient` over the src-tauri commands (app spec §3.2). One-shot ops are
 * `invoke(op, {hostId, ...args})` (app-level ops omit `hostId`). The session,
 * hub-event, terminal and LSP streams wait for WP-H4 / H6 / A21 (their
 * src-tauri commands are not there yet); until then they report
 * `E_UNSUPPORTED` through their sinks, so the UI shows a clear state instead
 * of hanging.
 */
import { invoke } from '@tauri-apps/api/core';
import { createSignal } from 'solid-js';
import { toAppError } from '@unipi/protocol';
import {
  APP_HOST,
  isUnreachableCode,
  unavailableError,
  type Closable,
  type HostClient,
  type HostConn,
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
    openSession(_req, sink: SessionSink): SessionStream {
      // WP-H4 adds the `session_open` command + Channel; WP-A2 wires it here.
      queueMicrotask(() => sink.closed(unavailableError('Opening sessions')));
      return { cmd: () => Promise.reject(unavailableError('Session commands')), close() {} };
    },
    openEvents(_sink): Closable {
      return { close() {} };
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
