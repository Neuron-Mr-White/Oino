/**
 * The picker's zoxide seam: what `zoxide query -ls <words>` ranks on a PC (empty words = its top
 * folders). One path for every host — the host's read-only `fs_zoxide` op, which only returns folders
 * the asking peer may list (so the phone never sees what it may not browse). The desktop's own PC
 * falls back to the Tauri `local_zoxide` command when the host cannot answer (an older host).
 * "Not installed" is a normal answer (`available:false`), never an error.
 */
import { fsZoxide } from './fs';
import { isOwnPcFile, localZoxide, type ZoxideHit } from './openLocal';

export type ZoxideAnswer = { available: boolean; hits: ZoxideHit[] };

const NONE: ZoxideAnswer = { available: false, hits: [] };

export async function zoxideQuery(hostId: string, query: string, opts: { limit?: number; home?: string } = {}): Promise<ZoxideAnswer> {
  const limit = opts.limit ?? 20;
  const res = await fsZoxide(hostId, query, limit).catch(() => undefined);
  if (res?.ok) return { available: res.value.available, hits: res.value.hits.slice(0, limit) };
  if (!isOwnPcFile(hostId)) return NONE;
  const local = await localZoxide(query);
  if (!local.ok) return NONE;
  // `local_zoxide` is not scope- or workspace-filtered: drop the home itself and scratch areas.
  const skip = (p: string) => p === opts.home || p === '/' || /^\/(var\/)?tmp(\/|$)/.test(p);
  return { available: true, hits: local.hits.filter((h) => !skip(h.path)).slice(0, limit) };
}
