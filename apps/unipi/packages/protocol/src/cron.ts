import { z } from "zod";
import { ChatNewDataSchema } from "./chat.js";

/**
 * Scheduled pi runs from the app (UNI-317) and their settings (UNI-318).
 * Mirrors `crates/proto/src/cron.rs`. One request/reply op per action
 * (`hello{req:{op:"cron_*", …}}` → `ok{data}` or `err`).
 *
 * Who may use these: this PC's own app (local socket) always; a phone
 * needs the `cron` sharing feature in its per-device allow list first
 * (UNI-248 pattern), for every op below including the read-only ones.
 */

export const CRON_NAME_MAX = 200;
export const CRON_PROMPT_MAX = 20_000;
export const CRON_RUNS_LIMIT_MAX = 500;
export const CRON_PARSE_PREVIEW = 3;

export const CronJobIdSchema = z
  .string()
  .min(1)
  .max(64)
  .regex(/^[A-Za-z0-9_-]+$/, "id must be [A-Za-z0-9_-]");

export const CronRunIdSchema = z
  .string()
  .min(1)
  .max(64)
  .regex(/^[A-Za-z0-9_-]+$/, "run id must be [A-Za-z0-9_-]");

export const CronNameSchema = z.string().min(1).max(CRON_NAME_MAX);
export const CronPromptSchema = z.string().min(1).max(CRON_PROMPT_MAX);
export const CronRunsLimitSchema = z.number().int().min(1).max(CRON_RUNS_LIMIT_MAX);

export const CronRunStatusSchema = z.enum(["running", "ok", "failed", "timeout", "skipped", "queued"]);
export type CronRunStatus = z.infer<typeof CronRunStatusSchema>;

/** UNI-319: a job's notify choice; each half absent = the PC's default. */
export const CronNotifySchema = z.object({
  channels: z.array(z.string()).optional(),
  events: z.array(z.string()).optional(),
});
export type CronNotify = z.infer<typeof CronNotifySchema>;

/** UNI-319: the chat a job wakes; `pid` = the live pi on it right now. */
export const CronWakeSchema = z.object({
  session_file: z.string().optional(),
  session_id: z.string().optional(),
  pid: z.number().int().min(1).optional(),
  session_name: z.string().optional(),
});
export type CronWake = z.infer<typeof CronWakeSchema>;

export const CronJobSchema = z.object({
  id: z.string(),
  name: z.string(),
  prompt: z.string(),
  schedule: z.string(),
  tz: z.string(),
  cwd: z.string(),
  model: z.string().optional(),
  thinking: z.string().optional(),
  enabled: z.boolean(),
  time_limit_s: z.number().int().min(0),
  cost_cap_usd: z.number().optional(),
  created: z.number().int().min(0),
  updated: z.number().int().min(0),
  next_fire: z.string().optional(),
  running: z.boolean().default(false),
  last_run_status: CronRunStatusSchema.optional(),
  notify: CronNotifySchema.optional(),
  wake: CronWakeSchema.optional(),
});
export type CronJob = z.infer<typeof CronJobSchema>;

export const CronRunSchema = z.object({
  id: z.string(),
  job_id: z.string(),
  status: CronRunStatusSchema,
  fired_at: z.number().int().min(0),
  started_at: z.number().int().min(0).optional(),
  ended_at: z.number().int().min(0).optional(),
  exit_code: z.number().int().optional(),
  summary: z.string().optional(),
  reason: z.string().optional(),
  session_path: z.string().optional(),
  cost_usd: z.number().optional(),
  missed: z.boolean().default(false),
});
export type CronRun = z.infer<typeof CronRunSchema>;

export const CronJobsDataSchema = z.object({
  jobs: z.array(CronJobSchema),
  paused: z.boolean(),
});
export type CronJobsData = z.infer<typeof CronJobsDataSchema>;

export const CronShowDataSchema = z.object({
  job: CronJobSchema,
  runs: z.array(CronRunSchema),
});
export type CronShowData = z.infer<typeof CronShowDataSchema>;

export const CronRunsDataSchema = z.object({
  runs: z.array(CronRunSchema),
});
export type CronRunsData = z.infer<typeof CronRunsDataSchema>;

export const CronLogDataSchema = z.object({
  job_id: z.string(),
  run: CronRunSchema,
  log: z.string(),
});
export type CronLogData = z.infer<typeof CronLogDataSchema>;

export const CronRunStartedDataSchema = z.object({
  id: z.string(),
  run_id: z.string(),
});
export type CronRunStartedData = z.infer<typeof CronRunStartedDataSchema>;

export const CronStoppedDataSchema = z.object({
  id: z.string(),
  stopped: z.boolean(),
});
export type CronStoppedData = z.infer<typeof CronStoppedDataSchema>;

export const CronJobStatusSchema = z.object({
  id: z.string(),
  enabled: z.boolean(),
  next_fire: z.string().optional(),
  running: z.boolean(),
});
export type CronJobStatus = z.infer<typeof CronJobStatusSchema>;

export const CronStatusDataSchema = z.object({
  paused: z.boolean(),
  concurrency: z.number().int().min(0),
  jobs: z.array(CronJobStatusSchema),
});
export type CronStatusData = z.infer<typeof CronStatusDataSchema>;

export const CronParseDataSchema = z.object({
  expression: z.string(),
  one_time: z.boolean(),
  tz: z.string(),
  next_fires: z.array(z.string()),
});
export type CronParseData = z.infer<typeof CronParseDataSchema>;

export const CronSettingsDataSchema = z.object({
  concurrency: z.number().int().min(0),
  missed_window_s: z.number().int().min(0),
  retention_runs: z.number().int().min(0),
  retention_days: z.number().int().min(0),
  default_time_limit_s: z.number().int().min(0),
  paused: z.boolean(),
  enabled: z.boolean(),
  devices: z.array(z.string()).default([]),
  ntfy_topic: z.string().optional(),
  notify_default_events: z.array(z.string()).default([]),
  /** UNI-319: the notify channels set up on this PC. */
  notify_channels: z.array(z.string()).default([]),
});
export type CronSettingsData = z.infer<typeof CronSettingsDataSchema>;

export const CronAddRequestSchema = z.object({
  id: CronJobIdSchema.optional(),
  name: CronNameSchema,
  prompt: CronPromptSchema,
  schedule: z.string(),
  tz: z.string().optional(),
  cwd: z.string(),
  model: z.string().optional(),
  thinking: z.string().optional(),
  enabled: z.boolean().optional(),
  time_limit_s: z.number().int().min(0).optional(),
  cost_cap_usd: z.number().optional(),
  notify: CronNotifySchema.optional(),
  /** UNI-319: wake this live chat (its pi's pid) when a run ends. */
  wake_pid: z.number().int().min(1).optional(),
});
export type CronAddRequest = z.infer<typeof CronAddRequestSchema>;

export const CronEditRequestSchema = z.object({
  name: CronNameSchema.optional(),
  prompt: CronPromptSchema.optional(),
  schedule: z.string().optional(),
  tz: z.string().optional(),
  cwd: z.string().optional(),
  model: z.string().optional(),
  thinking: z.string().optional(),
  enabled: z.boolean().optional(),
  time_limit_s: z.number().int().min(0).optional(),
  cost_cap_usd: z.number().optional(),
  /** UNI-319: replaces the notify choice; `{}` = back to the PC's defaults. */
  notify: CronNotifySchema.optional(),
  wake_pid: z.number().int().min(1).optional(),
  /** UNI-319: stop waking any chat. */
  wake_clear: z.boolean().optional(),
});
export type CronEditRequest = z.infer<typeof CronEditRequestSchema>;

export const CronSettingsSetRequestSchema = z.object({
  concurrency: z.number().int().min(0).optional(),
  missed_window_s: z.number().int().min(0).optional(),
  retention_runs: z.number().int().min(0).optional(),
  retention_days: z.number().int().min(0).optional(),
  default_time_limit_s: z.number().int().min(0).optional(),
  paused: z.boolean().optional(),
  ntfy_topic: z.string().optional(),
  notify_default_events: z.array(z.string()).optional(),
});
export type CronSettingsSetRequest = z.infer<typeof CronSettingsSetRequestSchema>;

/** UNI-319: `cron_open_session` → the chat now on the run's session. */
export const CronOpenSessionDataSchema = z.object({
  pid: z.number().int().min(1),
  already_open: z.boolean().default(false),
  started: ChatNewDataSchema.optional(),
});
export type CronOpenSessionData = z.infer<typeof CronOpenSessionDataSchema>;
