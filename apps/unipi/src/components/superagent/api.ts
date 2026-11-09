import { z } from 'zod';
import { hostCall, hostClient } from '../../lib/host/factory';
import type { HostClient } from '../../lib/host/client';
import { toAppError } from '@unipi/protocol';

export const AutonomySchema = z.enum(['low', 'medium', 'high']);
export const MODES = ['none', 'goal', 'ralph', 'swarm', 'graph'] as const;
export type LeadMode = (typeof MODES)[number];
export const REVIEWS = ['none', 'light', 'full'] as const;
export const PAUSED_GRAPH_POLICIES = ['report', 'reuse'] as const;
export type PausedGraphPolicy = (typeof PAUSED_GRAPH_POLICIES)[number];
export const PausedGraphPolicySchema = z.enum(PAUSED_GRAPH_POLICIES);

const ProgressSchema = z.object({ running: z.number(), queued: z.number(), done: z.number() });
const PausedGraphSchema = z.object({ sessionId: z.string().min(1), title: z.string().nullish() });
export type PausedGraph = z.infer<typeof PausedGraphSchema>;
const LeadSchema = z.object({
  projectId: z.string(), sessionId: z.string(),
  presetId: z.string().nullish(), model: z.string().nullish(), thinking: z.string().nullish(),
  autonomy: AutonomySchema, mode: z.string().nullish(),
  ticketIds: z.array(z.string()).default([]),
  step: z.string().nullish(), tool: z.string().nullish(), startedAt: z.string().nullish(), cost: z.number().nullish(),
  progress: ProgressSchema.nullish(),
  pausedGraph: PausedGraphSchema.nullish(),
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
export const BLOCKER_CODES = ['not_registered', 'archived', 'project_auto_off', 'global_auto_off'] as const;
export const ENABLE_ACTIONS = ['register', 'unarchive', 'enable_project_auto', 'enable_auto_start'] as const;
export type EnableAction = (typeof ENABLE_ACTIONS)[number];
/** Why a project cannot take work, and the one action that clears it. */
const BlockerSchema = z.object({
  code: z.enum(BLOCKER_CODES), message: z.string(),
  action: z.object({ kind: z.enum(ENABLE_ACTIONS), label: z.string() }),
});
export type Blocker = z.infer<typeof BlockerSchema>;
/** `not_registered` and `archived` refuse any start; the auto blockers only stop automatic starts. */
export const isHardBlocker = (b: Blocker | undefined) => b?.code === 'not_registered' || b?.code === 'archived';
const ProjectSchema = z.object({
  projectId: z.string(), archived: z.boolean().optional(), name: z.string().nullish(), autoStart: z.boolean(), lead: z.string().nullish(), ready: z.number().int().nonnegative().default(0),
  root: z.string().nullish(), leadModel: z.string().nullish(), leadThinking: z.string().nullish(),
  /** Older hosts omit these: no blockers, always registered. */
  blockers: z.array(BlockerSchema).default([]), registered: z.boolean().default(true), path: z.string().nullish(),
});

const RawStatusSchema = z.object({
  enabled: z.boolean(),
  pausedGraphPolicy: PausedGraphPolicySchema.default('report'),
  leads: z.array(LeadSchema).optional(), workers: z.array(WorkerSchema).optional(),
  needsYou: z.array(NeedsSchema).default([]), review: z.array(ReviewSchema).optional(), projects: z.array(ProjectSchema).optional(),
});

export type Lead = z.infer<typeof LeadSchema> & { legacy?: boolean };
export type NeedsItem = z.infer<typeof NeedsSchema>;
export type ApprovalItem = NeedsItem & { workerSessionId: string; requestId: string; sessionId: string; tool: string; ceiling: z.infer<typeof AutonomySchema>; reason: string };
export type ReviewItem = z.infer<typeof ReviewSchema>;
/** `blockers` / `registered` are always set on parsed rows; optional here so a row built by hand (an older host's shape) still type-checks. */
export type ProjectRow = Omit<z.infer<typeof ProjectSchema>, 'blockers' | 'registered'> & { blockers?: Blocker[]; registered?: boolean };
export type SuperagentStatus = {
  enabled: boolean; pausedGraphPolicy: PausedGraphPolicy; leads: Lead[]; needsYou: NeedsItem[]; review: ReviewItem[];
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
    enabled: raw.enabled, pausedGraphPolicy: raw.pausedGraphPolicy, leads, legacy,
    needsYou: raw.needsYou.filter((n) => n.status !== 'in_review'),
    review: raw.review ?? oldReview,
    projects: raw.projects ?? null,
  };
}
export const StatusSchema = RawStatusSchema.transform(normalizeStatus);

export const ConfigSchema = z.object({ enabled: z.boolean(), pausedGraphPolicy: PausedGraphPolicySchema.optional() });
const ConfigReplySchema = z.object({ enabled: z.boolean(), pausedGraphPolicy: PausedGraphPolicySchema.optional() });
export const RelayTextSchema = z.string().trim().min(1, 'Enter a message to relay.');
export const SuperagentSessionSchema = z.object({ sessionId: z.string(), title: z.string(), createdAt: z.union([z.string(), z.number()]), updatedAt: z.union([z.string(), z.number()]), path: z.string(), project: z.string().nullish(), state: z.object({ kind: z.string() }).nullish(), pendingWork: z.string().nullish() });
export type SuperagentSession = z.infer<typeof SuperagentSessionSchema>;
const SessionsSchema = z.union([z.array(SuperagentSessionSchema), z.object({ sessions: z.array(SuperagentSessionSchema) }).transform((value) => value.sessions)]);

export const PresetSchema = z.object({
  id: z.string().min(1), label: z.string().min(1), description: z.string().default(''),
  model: z.string().nullable().default(null), thinking: z.string().nullable().default(null),
  autonomy: AutonomySchema, mode: z.enum(MODES), maxParallel: z.number().int().nullable().default(null),
  review: z.enum(REVIEWS).default('none'), profiles: z.array(z.string()).nullable().default(null), priority: z.boolean().default(false),
});
export type Preset = z.infer<typeof PresetSchema>;
const PresetListSchema = z.object({ defaultPresetId: z.string().nullish(), presets: z.array(PresetSchema) });

/** `start` refused because the project is not registered / is archived (the host's own code, or the one it carries in `detail`). */
export function startRefusal(error: { code?: unknown; detail?: unknown }): 'not_registered' | 'archived' | null {
  const detail = error.detail && typeof error.detail === 'object' ? (error.detail as { code?: unknown }).code : undefined;
  const codes = [String(error.code), String(detail)];
  if (codes.includes('E_PROJECT_NOT_REGISTERED')) return 'not_registered';
  if (codes.includes('E_PROJECT_ARCHIVED')) return 'archived';
  return null;
}

/** One agent on the fleet canvas (`superagent.fleet`): a lead, a graph/swarm worker or a subagent. */
export const FleetNodeSchema = z.object({
  id: z.string(), parentId: z.string().nullish(), project: z.string().nullable().default(null), projectId: z.string().nullish(),
  title: z.string().default(''),
  mode: z.enum(['plain', 'graph', 'swarm']).catch('plain'),
  role: z.enum(['lead', 'worker', 'subagent']).catch('subagent'),
  status: z.enum(['working', 'waiting', 'idle', 'ended']).catch('idle'),
  model: z.string().nullable().default(null), startedAt: z.number().nullable().default(null), endedAt: z.number().nullable().default(null),
});
export const FleetSchema = z.object({ nodes: z.array(FleetNodeSchema).default([]), edges: z.array(z.object({ from: z.string(), to: z.string() })).default([]) });
export type FleetNode = z.infer<typeof FleetNodeSchema>;
export type Fleet = z.infer<typeof FleetSchema>;

export type RelayDelivery = 'queue' | 'steer';
export const superagentApi = {
  fleet: (host: string) => hostCall(host, 'superagent.fleet', {}, FleetSchema),
  sessions: (host: string) => hostCall(host, 'superagent.sessions', {}, SessionsSchema),
  status: (host: string) => hostCall(host, 'superagent.status', {}, StatusSchema),
  configure: (host: string, config: z.infer<typeof ConfigSchema>) => hostCall(host, 'superagent.configure', ConfigSchema.parse(config), ConfigReplySchema),
  configureProject: (host: string, projectId: string, autoStart: boolean) => hostCall(host, 'superagent.configure_project', { projectId, autoStart }, z.object({ projectId: z.string(), autoStart: z.boolean() })),
  /** The project's lead model + thinking (both null clears; a running lead keeps its model until restarted). */
  configureProjectLead: (host: string, projectId: string, model: string | null, thinking: string | null) => hostCall(host, 'superagent.configure_project_lead', { projectId, model, thinking }, z.object({ projectId: z.string(), model: z.string().nullish(), thinking: z.string().nullish() })),
  setMode: (host: string, projectId: string, mode: LeadMode) => hostCall(host, 'superagent.set_mode', { projectId, mode }, z.object({ sessionId: z.string().nullish(), mode: z.string() })),
  start: (host: string, projectId: string, taskIds: string[], presetId?: string | null) => hostCall(host, 'superagent.start', { projectId, taskIds, ...(presetId ? { presetId } : {}) }, z.object({ sessionId: z.string() })),
  /** Clears one blocker: register a workspace, unarchive, or switch Auto on. `register` needs `path` and returns the new slug. */
  enable: (host: string, input: { action: EnableAction; projectId?: string; path?: string }) => hostCall(host, 'superagent.enable', {
    action: input.action, ...(input.projectId ? { projectId: input.projectId } : {}), ...(input.path ? { path: input.path } : {}),
  }, z.object({ ok: z.literal(true), projectId: z.string().nullish() })),
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

/** Inspect a history session without treating metadata-only entries as messages. */
export function isEmptySuperagentSession(hostId: string, id: string, client: HostClient = hostClient(hostId)): Promise<boolean> {
  return new Promise((resolve) => {
    let done = false;
    let stream: ReturnType<HostClient['openSession']> | undefined;
    const finish = (empty: boolean) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      stream?.close();
      resolve(empty);
    };
    const timer = setTimeout(() => finish(false), 3000);
    stream = client.openSession({ kind: 'open', session: id }, {
      snapshot(snapshot) {
        const state = snapshot.state as { isStreaming?: boolean } | undefined;
        const empty = snapshot.reset && !snapshot.page?.has_more && !snapshot.waking && !state?.isStreaming && !!snapshot.entries && !snapshot.entries.some((entry) => (entry as { type?: string }).type === 'message');
        finish(empty);
      },
      events() {},
      closed() { finish(false); },
    });
    if (done) stream.close();
  });
}

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
