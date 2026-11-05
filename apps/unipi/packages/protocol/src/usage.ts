/**
 * Settings › Usage (owner fix 2026-11-04): `usage_stats` aggregates the token /
 * cost history of every session file on a host for the last N local days.
 * Mirrors `unipi_proto::usage`.
 */
import { z } from "zod";

export const USAGE_DAYS_MAX = 366;

export const UsageStatsRequestSchema = z.object({
  op: z.literal("usage_stats"),
  days: z.number().int().min(1).max(USAGE_DAYS_MAX),
  tz_offset_min: z.number().int().optional(),
});

export const UsageTotalsSchema = z.object({
  input: z.number(),
  output: z.number(),
  cache_read: z.number(),
  cache_write: z.number(),
  tokens: z.number(),
  /** Priced spend: models without a price add nothing (see `unpriced_tokens`). */
  cost: z.number(),
  unpriced_tokens: z.number(),
  turns: z.number(),
  sessions: z.number(),
  active_days: z.number(),
  /** The longest session in the range, first to last reply (ms). */
  longest_session_ms: z.number().default(0),
});
export type UsageTotals = z.infer<typeof UsageTotalsSchema>;

export const UsageDaySchema = z.object({
  day: z.string(),
  tokens: z.number(),
  cost: z.number(),
  unpriced_tokens: z.number(),
  turns: z.number().default(0),
  /** Tokens per model that day (top models by name, the rest as `other`). */
  models: z.record(z.string(), z.number()).default({}),
});
export const USAGE_OTHER_MODEL = "other";
export type UsageDay = z.infer<typeof UsageDaySchema>;

export const UsageModelSchema = z.object({ name: z.string(), tokens: z.number(), cost: z.number(), unpriced_tokens: z.number(), turns: z.number() });
export type UsageModel = z.infer<typeof UsageModelSchema>;

export const UsageProjectSchema = z.object({ name: z.string(), tokens: z.number(), cost: z.number(), unpriced_tokens: z.number(), sessions: z.number() });
export type UsageProject = z.infer<typeof UsageProjectSchema>;

export const UsageDataSchema = z
  .object({
    days: z.number(),
    totals: UsageTotalsSchema,
    daily: z.array(UsageDaySchema),
    by_model: z.array(UsageModelSchema),
    by_project: z.array(UsageProjectSchema),
  })
  .passthrough();
export type UsageData = z.infer<typeof UsageDataSchema>;
