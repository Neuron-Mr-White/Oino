/**
 * Where `HostClient`s come from (app spec §3.2/§3.15). One client per host id,
 * cached. Order: a test-injected factory (`setHostClientFactory`), then the
 * capture harness's mock (`window.__UNIPI_HOST__`, set by
 * `vite.config.capture.ts` when the URL has `scenario=`), then Tauri.
 */
import { requestParsed, APP_HOST, type HostClient, type HostResult } from './client';
import { createTauriHost } from './tauri';
import type { z } from 'zod';

export type HostClientFactory = (hostId: string) => HostClient;

declare global {
  interface Window {
    /** Capture harness: the mock host world's client factory. */
    __UNIPI_HOST__?: HostClientFactory;
  }
}

let override: HostClientFactory | undefined;
const cache = new Map<string, HostClient>();

/** Tests inject a factory (e.g. `createMockHost(scenario).client`); `undefined` restores the default. */
export function setHostClientFactory(factory: HostClientFactory | undefined): void {
  override = factory;
  cache.clear();
}

function injected(): HostClientFactory | undefined {
  return typeof window !== 'undefined' ? window.__UNIPI_HOST__ : undefined;
}

/** The client for a host (`local` = This computer). */
export function hostClient(hostId: string): HostClient {
  const factory = override ?? injected();
  if (factory) return factory(hostId);
  let client = cache.get(hostId);
  if (!client) {
    client = createTauriHost(hostId);
    cache.set(hostId, client);
  }
  return client;
}

/** The client for app-level ops (no `hostId`: paired hosts, local forwards, save-as…). */
export function appClient(): HostClient {
  return hostClient(APP_HOST);
}

/** `hostClient(hostId).request(op, args)` parsed with `schema`, as a Result. */
export function hostCall<T>(hostId: string, op: string, args: Record<string, unknown> | undefined, schema: z.ZodType<T>): Promise<HostResult<T>> {
  return requestParsed(hostClient(hostId), op, args, schema);
}

/** An app-level op parsed with `schema`, as a Result. */
export function appCall<T>(op: string, args: Record<string, unknown> | undefined, schema: z.ZodType<T>): Promise<HostResult<T>> {
  return requestParsed(appClient(), op, args, schema);
}
