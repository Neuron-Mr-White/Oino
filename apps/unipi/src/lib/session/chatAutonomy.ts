/**
 * The autonomy level each open chat last reported, kept after the session registry (LRU 3) lets go of its handle.
 * The chat itself is still open on the host with its project's services at that level, so Settings › MCP (N2) must
 * still know it: a reload at a lower level would drop servers that only start at High (`needs Autonomy High`).
 * Whether the chat still exists is decided by the hub cards, not here.
 */
import { createSignal } from 'solid-js';
import { resolveHostId } from '../host/localId';
import type { AutonomyLevel } from './autonomy';

const known = new Map<string, AutonomyLevel>();
const [rev, setRev] = createSignal(0);
const keyOf = (hostId: string, sessionId: string): string => `${resolveHostId(hostId)}/${sessionId}`;

export function rememberAutonomy(hostId: string, sessionId: string, level: AutonomyLevel): void {
  const key = keyOf(hostId, sessionId);
  if (known.get(key) === level) return;
  known.set(key, level);
  setRev((n) => n + 1);
}

export function rememberedAutonomy(hostId: string, sessionId: string): AutonomyLevel | undefined {
  rev();
  return known.get(keyOf(hostId, sessionId));
}

/** Test seam. */
export function resetRememberedAutonomy(): void {
  known.clear();
  setRev((n) => n + 1);
}
