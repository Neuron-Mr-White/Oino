/**
 * Nested-agent refs of timeline entries (WP-A10, Z-42): which agent an entry
 * talks about, so the step rows and the completion cards open **one**
 * surface — the `subagent` pane tab — instead of a second "Open session" path.
 *
 * Pure over the entry JSON (no imports at runtime): the timeline rows and the
 * card wrapper both read it.
 */
import type { SessionEntry } from '../../../lib/session/types';

/** A nested agent of the root session (`_oino/agents` `AgentInfo.id` is its child session id). */
export type AgentRef = { agentId: string; name?: string; owner?: string; profile?: string; state?: string };

/** Entry types the subagents / fusion extensions write for a delegated step. */
export const AGENT_STEP_TYPES: ReadonlySet<string> = new Set(['subagent-step', 'sidekick-step']);
/** Message types a finished (or backgrounded) child delivers into the timeline. */
export const AGENT_CARD_TYPES: ReadonlySet<string> = new Set(['subagent-completion', 'sidekick-completion']);
/** Owners whose cards/entries name a nested agent. */
const AGENT_OWNERS: ReadonlySet<string> = new Set(['subagents', 'fusion']);

const str = (v: unknown): string | undefined => (typeof v === 'string' && v.trim() ? v : undefined);
const obj = (v: unknown): Record<string, unknown> | undefined => (v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : undefined);

/** A step entry's `data` (`{agentId, id, title, profile, label, kind, name, arg}`). */
export function agentOfStepData(data: unknown): AgentRef | undefined {
  const d = obj(data);
  if (!d) return undefined;
  const agentId = str(d.agentId) ?? str(d.id);
  if (!agentId) return undefined;
  return {
    agentId,
    ...(str(d.title) ? { name: str(d.title) } : str(d.label) ? { name: str(d.label) } : {}),
    ...(str(d.profile) ? { profile: str(d.profile) } : {}),
  };
}

/** The agent a timeline entry belongs to: a delegated step, or a completion card. */
export function agentOfEntry(entry: SessionEntry): AgentRef | undefined {
  const type = str(entry.customType);
  if (type && AGENT_STEP_TYPES.has(type)) return agentOfStepData(entry.data);
  if (!type || !AGENT_CARD_TYPES.has(type)) return undefined;
  const details = obj(entry.message?.details) ?? obj((entry as { details?: unknown }).details) ?? obj(entry.data);
  if (!details) return undefined;
  const agentId = str(details.id) ?? str(details.agentId);
  if (!agentId) return undefined;
  if (str(details.id) && str(details.owner) && !AGENT_OWNERS.has(str(details.owner)!)) return undefined;
  return {
    agentId,
    ...(str(details.title) ? { name: str(details.title) } : {}),
    ...(str(details.owner) ? { owner: str(details.owner) } : {}),
    ...(str(details.profile) ? { profile: str(details.profile) } : {}),
    ...(str(details.status) ? { state: str(details.status) } : {}),
  };
}
