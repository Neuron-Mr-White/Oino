import { z } from "zod";

/**
 * "Ask the PC to turn this on" (UNI-248, proto 1.11.0) and "turn it off
 * again" (UNI-253, proto 1.15.0). Mirrors
 * `crates/proto/src/sharing.rs`:
 * - `feature_request{feature, scope?, note?}` → `ok{data: FeatureRequestItem}`
 *   (stored as pending on the PC; the owner approves or denies it — nothing
 *   is ever switched on by itself).
 * - `feature_requests` → `ok{data: FeatureRequestsData}` (this device's
 *   latest request per feature).
 * - `feature_leave{feature, scope?}` → `ok{data: FeatureLeaveData}`: a device
 *   that has a feature drops it again, for itself (`scope: "self"`, the
 *   default) or for everyone (`scope: "all"`, only when it has it). Nothing
 *   here can switch a feature on.
 * Hosts before 1.11 / 1.15 answer with `E_SCHEMA` (unknown op).
 */

export const FEATURE_NOTE_MAX = 200;
export const FEATURE_PORTS_MAX = 16;

export const FeatureKindSchema = z.enum(["forward", "browser", "desktop", "push", "cron"]);
export type FeatureKind = z.infer<typeof FeatureKindSchema>;

export const FeatureScopeSchema = z
  .object({
    any_port: z.boolean().optional(),
    ports: z.array(z.number().int().min(1).max(65535)).max(FEATURE_PORTS_MAX).optional(),
    view_only: z.boolean().optional(),
    headed: z.boolean().optional(),
  })
  .strict();
export type FeatureScope = z.infer<typeof FeatureScopeSchema>;

export const FeatureRequestStatusSchema = z.enum(["pending", "approved", "denied", "expired"]);
export type FeatureRequestStatus = z.infer<typeof FeatureRequestStatusSchema>;

export const FeatureRequestItemSchema = z.object({
  id: z.string().min(1).max(64),
  feature: FeatureKindSchema,
  scope: z
    .object({
      any_port: z.boolean().optional(),
      ports: z.array(z.number().int()).optional(),
      view_only: z.boolean().optional(),
      headed: z.boolean().optional(),
    })
    .default({}),
  device_name: z.string(),
  note: z.string().optional(),
  status: FeatureRequestStatusSchema,
  created_at: z.number().int().min(0),
  decided_at: z.number().int().min(0).optional(),
  until: z.number().int().min(0).optional(),
});
export type FeatureRequestItem = z.infer<typeof FeatureRequestItemSchema>;

export const FeatureRequestsDataSchema = z.object({
  requests: z.array(FeatureRequestItemSchema).default([]),
});
export type FeatureRequestsData = z.infer<typeof FeatureRequestsDataSchema>;

/** How much of a feature a device turns off (UNI-253). */
export const FeatureLeaveScopeSchema = z.enum(["self", "all"]);
export type FeatureLeaveScope = z.infer<typeof FeatureLeaveScopeSchema>;

export const FeatureLeaveDataSchema = z.object({
  feature: FeatureKindSchema,
  /** This op actually removed something (false when the device wasn't allowed). */
  left: z.boolean(),
  /** Is the feature on for anyone after this? */
  now_enabled: z.boolean(),
  /** The remaining allow-list entries; `[]` means every paired device. */
  devices: z.array(z.string()).default([]),
});
export type FeatureLeaveData = z.infer<typeof FeatureLeaveDataSchema>;
