import type { SessionHandle } from './store';
import type { SessionEntry } from './types';

export const AUTONOMY_LEVELS = ['low', 'medium', 'high'] as const;
export type AutonomyLevel = typeof AUTONOMY_LEVELS[number];
export type ApprovalDecision = 'allow_once' | 'always' | 'deny';
export type ApprovalRequest = { requestId: string; sessionId: string; tool: string; arguments: Record<string, unknown>; riskLevel: AutonomyLevel | null; ceiling: AutonomyLevel; reason: string };
export const AUTONOMY_HINT: Record<AutonomyLevel, string> = {
  low: 'Autonomy: Low (read-only commands)',
  medium: 'Autonomy: Medium (reversible commands)',
  high: 'Autonomy: High (allow all commands)',
};
export const isAutonomyLevel = (v: unknown): v is AutonomyLevel => AUTONOMY_LEVELS.includes(v as AutonomyLevel);
export function autonomyLevel(session: SessionHandle): AutonomyLevel {
  if (isAutonomyLevel(session.state.info.autonomy)) return session.state.info.autonomy;
  for (let i = session.state.entries.length - 1; i >= 0; i--) {
    const e: SessionEntry = session.state.entries[i]!;
    const data = e.data as { level?: unknown } | undefined;
    if (e.customType === 'oino:autonomy' && isAutonomyLevel(data?.level)) return data.level;
  }
  return 'medium';
}
