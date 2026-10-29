import { invoke } from '@tauri-apps/api/core';
import { type AppError, toAppError } from '@unipi/protocol';
import { z } from 'zod';
export type Result<T, E = AppError> = { ok: true; value: T } | { ok: false; error: E };
export async function call<T>(cmd: string, args: Record<string, unknown> | undefined, schema: z.ZodType<T>): Promise<Result<T>> {
  try {
    const parsed = schema.safeParse(await invoke(cmd, args));
    return parsed.success ? { ok: true, value: parsed.data } : { ok: false, error: toAppError(parsed.error) };
  } catch (error) { return { ok: false, error: toAppError(error) }; }
}
