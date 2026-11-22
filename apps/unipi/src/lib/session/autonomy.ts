import type { SessionHandle } from './store';
import type { SessionEntry } from './types';

export const AUTONOMY_LEVELS = ['low', 'medium', 'high'] as const;
export type AutonomyLevel = typeof AUTONOMY_LEVELS[number];
export type ApprovalDecision = 'allow_once' | 'always' | 'deny';
export type ApprovalRequest = { requestId: string; sessionId: string; tool: string; arguments: Record<string, unknown>; riskLevel: AutonomyLevel | null; ceiling: AutonomyLevel; reason: string; alwaysAvailable?: boolean };
export const AUTONOMY_HINT: Record<AutonomyLevel, string> = {
  low: 'Autonomy: Low (read-only commands)',
  medium: 'Autonomy: Medium (reversible commands)',
  high: 'Autonomy: High (allow all commands)',
};
export const isAutonomyLevel = (v: unknown): v is AutonomyLevel => AUTONOMY_LEVELS.includes(v as AutonomyLevel);
/**
 * Is the chat's autonomy level known for real? Two cases leave it pending (the chip stays
 * neutral instead of guessing `medium`, which then rolls to the real level — often `high`):
 * - r5 D21: a waking chat's provisional state (read from the session file) carries no autonomy;
 * - r7 P2-1: no state at all yet (`info` is empty until the first snapshot — up to 3 s on a cold
 *   40k open). The model chip's rule (`modelResolved`): no info yet → pending.
 */
export function autonomyKnown(session: Pick<SessionHandle, 'state'>): boolean {
  const { info, waking } = session.state;
  if (isAutonomyLevel(info.autonomy)) return true;
  return !waking && Object.keys(info).length > 0;
}
export function autonomyLevel(session: SessionHandle): AutonomyLevel {
  if (isAutonomyLevel(session.state.info.autonomy)) return session.state.info.autonomy;
  for (let i = session.state.entries.length - 1; i >= 0; i--) {
    const e: SessionEntry = session.state.entries[i]!;
    const data = e.data as { level?: unknown } | undefined;
    if (e.customType === 'oino:autonomy' && isAutonomyLevel(data?.level)) return data.level;
  }
  return 'medium';
}
