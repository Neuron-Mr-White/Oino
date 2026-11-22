/**
 * App-level op helper: `call(op, args, schema)` → the app `HostClient`
 * (`lib/host`, app spec §3.2), parsed with `schema`, as a Result. Host ops use
 * `hostCall(hostId, …)` from `lib/host` instead; legacy `call(op, {hostId, …})`
 * sites still reach the right host through the same seam.
 */
import type { AppError } from '@unipi/protocol';
import type { z } from 'zod';
import { appCall } from './host/factory';
export type Result<T, E = AppError> = { ok: true; value: T } | { ok: false; error: E };
export function call<T>(cmd: string, args: Record<string, unknown> | undefined, schema: z.ZodType<T>): Promise<Result<T>> {
  return appCall(cmd, args, schema);
}

/** Marker for test-setup: present on the real module, absent when a suite mocks `lib/ipc`. */
export const IPC_REAL = true;
