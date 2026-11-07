import { z } from 'zod';
import { hostCall, hostClient } from '../../lib/host/factory';
import type { HostClient } from '../../lib/host/client';
import { toAppError } from '@unipi/protocol';

export const AutonomySchema = z.enum(['low', 'medium', 'high']);
const WorkerSchema = z.object({
  projectId: z.string(), taskId: z.string(), sessionId: z.string(), autonomy: AutonomySchema,
  step: z.string().nullable(), tool: z.string().nullable(), startedAt: z.string().nullable(), cost: z.number().nullable(),
});
export const StatusSchema = z.object({
  enabled: z.boolean(), concurrencyCap: z.number().int().min(1), workers: z.array(WorkerSchema),
  needsYou: z.array(z.object({ projectId: z.string(), taskId: z.string(), status: z.enum(['blocked', 'in_review', 'waiting_for_approval']), question: z.string().nullish(), workerSessionId: z.string().optional(), requestId: z.string().optional(), sessionId: z.string().optional(), tool: z.string().optional(), command: z.string().nullish(), summary: z.string().nullish(), risk: AutonomySchema.nullish(), ceiling: AutonomySchema.optional(), reason: z.string().optional() })),
});
export type SuperagentStatus = z.infer<typeof StatusSchema>;
export type Worker = z.infer<typeof WorkerSchema>;
export const ConfigSchema = z.object({ enabled: z.boolean(), concurrencyCap: z.number().int().min(1) });
export const RelayTextSchema = z.string().trim().min(1, 'Enter a message to relay.');
export const SuperagentSessionSchema = z.object({ sessionId: z.string(), title: z.string(), createdAt: z.union([z.string(), z.number()]), updatedAt: z.union([z.string(), z.number()]), path: z.string() });
const SessionsSchema = z.union([z.array(SuperagentSessionSchema), z.object({ sessions: z.array(SuperagentSessionSchema) }).transform((value) => value.sessions)]);
export const superagentApi = {
  sessions: (host: string) => hostCall(host, 'superagent.sessions', {}, SessionsSchema),
  status: (host: string) => hostCall(host, 'superagent.status', {}, StatusSchema),
  configure: (host: string, config: z.infer<typeof ConfigSchema>) => hostCall(host, 'superagent.configure', ConfigSchema.parse(config), ConfigSchema),
  start: (host: string, projectId: string, taskId: string) => hostCall(host, 'superagent.start', { projectId, taskId }, z.object({ sessionId: z.string() })),
  stop: (host: string, projectId: string, taskId: string) => hostCall(host, 'superagent.stop', { projectId, taskId }, z.object({ stopped: z.boolean() })),
  approvalResponse: (host: string, projectId: string, taskId: string, workerSessionId: string, requestId: string, decision: 'allow_once' | 'always' | 'deny') => hostCall(host, 'superagent.approval_response', { projectId, taskId, workerSessionId, requestId, decision }, z.object({ accepted: z.literal(true) })),
  relay: (host: string, projectId: string, taskId: string, text: string) => hostCall(host, 'superagent.relay', { projectId, taskId, text: RelayTextSchema.parse(text) }, z.object({ commentId: z.string(), sessionId: z.string(), delivered: z.boolean(), error: z.unknown().optional() })),
};

/** Creation is a session stream, never an ordinary session_new or a shell command. */
export function openSuperagentSession(hostId: string, client: HostClient = hostClient(hostId)): Promise<string> {
  return new Promise((resolve, reject) => {
    let closed = false;
    const timer = setTimeout(() => { closed = true; stream.close(); reject(new Error('The host did not open Superagent. Retry.')); }, 20_000);
    const stream = client.openSession({ kind: 'superagent' }, {
      snapshot(snapshot) {
        clearTimeout(timer);
        if (closed) return;
        closed = true;
        // The host materializes the restricted session before answering. Keep the
        // creating stream alive briefly while Conversation attaches by exact id.
        setTimeout(() => stream.close(), 6000);
        resolve(snapshot.session_id);
      },
      events() {},
      closed(error) {
        clearTimeout(timer);
        if (closed) return;
        closed = true;
        reject(toAppError(error ?? new Error('The host closed Superagent. Retry.')));
      },
    });
  });
}
