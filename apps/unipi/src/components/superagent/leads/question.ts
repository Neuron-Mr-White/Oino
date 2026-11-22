import { toAppError } from '@unipi/protocol';
import { isUnsupportedOp, superagentApi, type NeedsItem } from '../api';
import { leadActionError, minutesText } from './lead';

/** "Asked 4 min ago · Becomes a blocked ticket in 26 min". A missing time leaves its half out; a passed deadline says so. */
export function questionTiming(item: Pick<NeedsItem, 'askedAt' | 'expiresAt'>, now: number): string | null {
  const parts: string[] = [];
  if (item.askedAt) parts.push(`Asked ${minutesText(now - item.askedAt)} ago`);
  if (item.expiresAt) parts.push(item.expiresAt > now ? `Becomes a blocked ticket in ${minutesText(item.expiresAt - now)}` : 'Becoming a blocked ticket now');
  return parts.length ? parts.join(' · ') : null;
}

export type LeadAnswer = { kind: 'sent' } | { kind: 'answered' | 'failed'; message: string };
export const ALREADY_ANSWERED = 'This question was already answered.';

const codeOf = (error: { code?: unknown; detail?: unknown }): string[] => [String(error.code), String((error.detail as { code?: unknown } | undefined)?.code)];

/**
 * Answers a lead's question:
 * - OPEN (`in_progress`, `kind: "question"`): `superagent.relay` into the same lead turn.
 * - BLOCKED with claim (`status: "blocked"`, `kind: "question"`, `sessionId` present): `superagent.answer` to wake the same lead.
 */
export async function answerLeadQuestion(
  hostId: string,
  item: Pick<NeedsItem, 'projectId' | 'taskId'> & Partial<Pick<NeedsItem, 'kind' | 'status' | 'sessionId' | 'requestId'>>,
  text: string,
): Promise<LeadAnswer> {
  const answer = text.trim();
  if (!answer) return { kind: 'failed', message: 'Enter a message to relay.' };
  try {
    const claimedBlocked = item.kind === 'question' && item.status === 'blocked' && !!item.sessionId;
    const result = claimedBlocked
      ? await superagentApi.answer(hostId, item.sessionId!, item.taskId, answer)
      : await superagentApi.relay(hostId, {
          projectId: item.projectId,
          taskIds: [item.taskId],
          text: answer,
          delivery: 'steer',
          ...(item.sessionId ? { sessionId: item.sessionId } : {}),
          ...(item.requestId ? { requestId: item.requestId } : {}),
        });
    if (!result.ok) {
      const codes = codeOf(result.error);
      if (codes.includes('E_ALREADY_ANSWERED')) return { kind: 'answered', message: ALREADY_ANSWERED };
      if (claimedBlocked && (codes.some(isUnsupportedOp) || /unknown (method|op|command)/i.test(result.error.message ?? ''))) {
        return { kind: 'failed', message: leadActionError(result.error) };
      }
      return { kind: 'failed', message: result.error.message || 'The host refused the answer.' };
    }
    if (!result.value.delivered) {
      if (claimedBlocked || !result.value.saved) {
        const detail = result.value.error;
        const msg = typeof detail === 'string' ? detail : (detail as { message?: string } | undefined)?.message;
        return { kind: 'failed', message: result.value.saved ? (msg || 'Answer saved, but could not wake the lead.') : `Saved, but the lead did not receive it. ${msg ?? 'Open the lead before retrying.'}` };
      }
    }
    return { kind: 'sent' };
  } catch (e) {
    const app = toAppError(e);
    if (item.kind === 'question' && item.status === 'blocked' && !!item.sessionId && (isUnsupportedOp(app.code) || /unknown (method|op|command)/i.test(app.message))) {
      return { kind: 'failed', message: leadActionError(app) };
    }
    return { kind: 'failed', message: app.message };
  }
}
