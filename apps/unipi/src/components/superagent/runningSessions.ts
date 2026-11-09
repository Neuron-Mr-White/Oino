import type { Lead, SuperagentSession, SuperagentStatus } from './api';

export type RunningSession = {
  sessionId: string;
  title: string;
  project: string | null;
  startedAt: string | number | null;
  open: boolean;
  lead?: Lead;
};

/** Merge the host's live leads and session history; older history rows may lack state. */
export function runningSessions(status: SuperagentStatus | undefined, history: readonly SuperagentSession[] | undefined, openId?: string): RunningSession[] {
  const rows = new Map<string, RunningSession>();
  for (const item of history ?? []) {
    if (item.state?.kind !== 'working' && !item.pendingWork) continue;
    rows.set(item.sessionId, { sessionId: item.sessionId, title: item.title || 'Superagent', project: item.project ?? null, startedAt: item.createdAt, open: item.sessionId === openId });
  }
  for (const lead of status?.leads ?? []) {
    const existing = rows.get(lead.sessionId);
    const project = status?.projects?.find((p) => p.projectId === lead.projectId)?.name || lead.projectId;
    rows.set(lead.sessionId, { sessionId: lead.sessionId, title: existing?.title || `Lead · ${project}`, project, startedAt: lead.startedAt ?? existing?.startedAt ?? null, open: lead.sessionId === openId, lead });
  }
  return [...rows.values()];
}
