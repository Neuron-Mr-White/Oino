import { z } from 'zod';
import { hostCall, hostClient } from '../../lib/host/factory';
import type { HostClient } from '../../lib/host/client';
import { toAppError } from '@unipi/protocol';

export const AutonomySchema = z.enum(['low', 'medium', 'high']);
export const MODES = ['none', 'goal', 'ralph', 'swarm', 'graph'] as const;
export type LeadMode = (typeof MODES)[number];
export const REVIEWS = ['none', 'light', 'full'] as const;
export const CAP_MIN = 1;
export const CAP_MAX = 3;

const ProgressSchema = z.object({ running: z.number(), queued: z.number(), done: z.number() });
const LeadSchema = z.object({
  projectId: z.string(), sessionId: z.string(),
  presetId: z.string().nullish(), model: z.string().nullish(), thinking: z.string().nullish(),
  autonomy: AutonomySchema, mode: z.string().nullish(),
  ticketIds: z.array(z.string()).default([]),
  step: z.string().nullish(), tool: z.string().nullish(), startedAt: z.string().nullish(), cost: z.number().nullish(),
  progress: ProgressSchema.nullish(),
});
/** An older host reports one worker per ticket. */
const WorkerSchema = z.object({
  projectId: z.string(), taskId: z.string(), sessionId: z.string(), autonomy: AutonomySchema,
  step: z.string().nullish(), tool: z.string().nullish(), startedAt: z.string().nullish(), cost: z.number().nullish(),
});
const NeedsSchema = z.object({
  projectId: z.string(), taskId: z.string(), status: z.string(), question: z.string().nullish(),
  workerSessionId: z.string().optional(), requestId: z.string().optional(), sessionId: z.string().optional(), tool: z.string().optional(),
  command: z.string().nullish(), summary: z.string().nullish(), risk: AutonomySchema.nullish(), ceiling: AutonomySchema.optional(), reason: z.string().optional(),
});
const ReviewSchema = z.object({ projectId: z.string(), taskId: z.string(), status: z.string().optional() });
const ProjectSchema = z.object({ projectId: z.string(), name: z.string().nullish(), autoStart: z.boolean(), lead: z.string().nullish(), ready: z.number().int().nonnegative().default(0) });

const RawStatusSchema = z.object({
  enabled: z.boolean(), concurrencyCap: z.number().int().min(1),
  leads: z.array(LeadSchema).optional(), workers: z.array(WorkerSchema).optional(),
  needsYou: z.array(NeedsSchema).default([]), review: z.array(ReviewSchema).optional(), projects: z.array(ProjectSchema).optional(),
});

export type Lead = z.infer<typeof LeadSchema> & { legacy?: boolean };
export type NeedsItem = z.infer<typeof NeedsSchema>;
export type ApprovalItem = NeedsItem & { workerSessionId: string; requestId: string; sessionId: string; tool: string; ceiling: z.infer<typeof AutonomySchema>; reason: string };
export type ReviewItem = z.infer<typeof ReviewSchema>;
export type ProjectRow = z.infer<typeof ProjectSchema>;
export type SuperagentStatus = {
  enabled: boolean; concurrencyCap: number; leads: Lead[]; needsYou: NeedsItem[]; review: ReviewItem[];
  /** null: the host predates per-project controls. */
  projects: ProjectRow[] | null;
  /** The host still reports one worker per ticket. */
  legacy: boolean;
};

export function isApproval(item: NeedsItem): item is ApprovalItem {
  return !!(item.requestId && item.workerSessionId && item.sessionId && item.tool && item.ceiling && item.reason);
}
export const isQuestion = (item: NeedsItem) => !isApproval(item) && item.status === 'blocked' && !!item.question?.trim();

/** Accepts the lead-shaped status and degrades the old worker-shaped one. */
export function normalizeStatus(raw: z.infer<typeof RawStatusSchema>): SuperagentStatus {
  const legacy = raw.leads === undefined;
  const leads: Lead[] = raw.leads ?? (raw.workers ?? []).map((w) => ({
    projectId: w.projectId, sessionId: w.sessionId, presetId: null, model: null, thinking: null, autonomy: w.autonomy, mode: null,
    ticketIds: [w.taskId], step: w.step, tool: w.tool, startedAt: w.startedAt, cost: w.cost, progress: null, legacy: true,
  }));
  const oldReview = raw.needsYou.filter((n) => n.status === 'in_review').map((n) => ({ projectId: n.projectId, taskId: n.taskId, status: n.status }));
  return {
    enabled: raw.enabled, concurrencyCap: raw.concurrencyCap, leads, legacy,
    needsYou: raw.needsYou.filter((n) => n.status !== 'in_review'),
    review: raw.review ?? oldReview,
    projects: raw.projects ?? null,
  };
}
export const StatusSchema = RawStatusSchema.transform(normalizeStatus);

export const ConfigSchema = z.object({ enabled: z.boolean(), concurrencyCap: z.number().int().min(CAP_MIN).max(CAP_MAX) });
const ConfigReplySchema = z.object({ enabled: z.boolean(), concurrencyCap: z.number().int().min(1) });
export const RelayTextSchema = z.string().trim().min(1, 'Enter a message to relay.');
export const SuperagentSessionSchema = z.object({ sessionId: z.string(), title: z.string(), createdAt: z.union([z.string(), z.number()]), updatedAt: z.union([z.string(), z.number()]), path: z.string() });
const SessionsSchema = z.union([z.array(SuperagentSessionSchema), z.object({ sessions: z.array(SuperagentSessionSchema) }).transform((value) => value.sessions)]);

export const PresetSchema = z.object({
  id: z.string().min(1), label: z.string().min(1), description: z.string().default(''),
  model: z.string().nullable().default(null), thinking: z.string().nullable().default(null),
  autonomy: AutonomySchema, mode: z.enum(MODES), maxParallel: z.number().int().nullable().default(null),
  review: z.enum(REVIEWS).default('none'), profiles: z.array(z.string()).nullable().default(null), priority: z.boolean().default(false),
});
export type Preset = z.infer<typeof PresetSchema>;
const PresetListSchema = z.object({ defaultPresetId: z.string().nullish(), presets: z.array(PresetSchema) });

export type RelayDelivery = 'queue' | 'steer';
export const superagentApi = {
  sessions: (host: string) => hostCall(host, 'superagent.sessions', {}, SessionsSchema),
  status: (host: string) => hostCall(host, 'superagent.status', {}, StatusSchema),
  configure: (host: string, config: z.infer<typeof ConfigSchema>) => hostCall(host, 'superagent.configure', ConfigSchema.parse(config), ConfigReplySchema),
  configureProject: (host: string, projectId: string, autoStart: boolean) => hostCall(host, 'superagent.configure_project', { projectId, autoStart }, z.object({ projectId: z.string(), autoStart: z.boolean() })),
  setMode: (host: string, projectId: string, mode: LeadMode) => hostCall(host, 'superagent.set_mode', { projectId, mode }, z.object({ sessionId: z.string().nullish(), mode: z.string() })),
  start: (host: string, projectId: string, taskIds: string[], presetId?: string | null) => hostCall(host, 'superagent.start', { projectId, taskIds, ...(presetId ? { presetId } : {}) }, z.object({ sessionId: z.string() })),
  stop: (host: string, projectId: string, taskId: string) => hostCall(host, 'superagent.stop', { projectId, taskId }, z.object({ stopped: z.boolean() })),
  approvalResponse: (host: string, projectId: string, taskId: string, workerSessionId: string, requestId: string, decision: 'allow_once' | 'always' | 'deny') => hostCall(host, 'superagent.approval_response', { projectId, taskId, workerSessionId, requestId, decision }, z.object({ accepted: z.literal(true) })),
  relay: (host: string, input: { projectId: string; text: string; delivery: RelayDelivery; taskIds?: string[]; priority?: boolean }) => hostCall(host, 'superagent.relay', { projectId: input.projectId, text: RelayTextSchema.parse(input.text), delivery: input.delivery, ...(input.taskIds ? { taskIds: input.taskIds } : {}), priority: input.priority ?? false }, z.object({ commentId: z.string().nullish(), sessionId: z.string().nullish(), delivered: z.boolean(), error: z.unknown().optional() })),
  presets: {
    list: (host: string) => hostCall(host, 'superagent.presets.list', {}, PresetListSchema),
    save: (host: string, preset: Preset) => hostCall(host, 'superagent.presets.save', { preset: PresetSchema.parse(preset) }, z.object({ preset: PresetSchema })),
    remove: (host: string, presetId: string) => hostCall(host, 'superagent.presets.delete', { presetId }, z.object({ deleted: z.boolean() })),
    setDefault: (host: string, presetId: string) => hostCall(host, 'superagent.presets.set_default', { presetId }, z.object({ defaultPresetId: z.string() })),
  },
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
