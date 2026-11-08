import type { NeedsItem } from './api';

/** Things waiting on the owner: an approval request, or a blocked ticket with a question. Shared by the Needs-you tab, the phone badge and the sidebar rail. */
export function countNeeds(items: readonly NeedsItem[] | undefined): number {
  return (items ?? []).filter((n) => (n.requestId && n.workerSessionId) || (n.status === 'blocked' && n.question?.trim())).length;
}
