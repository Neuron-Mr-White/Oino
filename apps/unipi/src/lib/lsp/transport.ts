/**
 * The `LSPClient` `Transport` over the `LspStream` seam (WP-A21): JSON
 * strings out (`stream.send`), message strings in (`dispatch` — queued
 * until `connect` subscribes so a synchronous mock message can't be
 * lost). The sink may hand us objects (the mock does), so the caller
 * normalizes both directions to strings.
 */
import type { Transport } from '@codemirror/lsp-client';

export type DispatchTransport = Transport & {
  /** Delivers one server message (JSON string) to the client — or queues
   *  it until `connect` subscribes. */
  dispatch(value: string): void;
};

export function streamTransport(stream: { send(json: unknown): void }): DispatchTransport {
  const pending: string[] = [];
  const handlers = new Set<(value: string) => void>();
  return {
    send(message: string): void {
      stream.send(message);
    },
    subscribe(handler: (value: string) => void): void {
      handlers.add(handler);
      for (const value of pending.splice(0)) handler(value);
    },
    unsubscribe(handler: (value: string) => void): void {
      handlers.delete(handler);
    },
    dispatch(value: string): void {
      if (handlers.size === 0) pending.push(value);
      else for (const handler of [...handlers]) handler(value);
    },
  };
}
