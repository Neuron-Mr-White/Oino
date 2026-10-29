import { z } from "zod";

/**
 * UNI-232 (proto 1.14.0): local-only admin ops. Mirrors
 * `crates/proto/src/admin.rs`. Only this PC's own UniPi app (the host's
 * local socket `app.sock`) may call them; over iroh they answer
 * `E_UNAUTHORIZED`. Older hosts answer `E_SCHEMA` (unknown op).
 * - `host_info` → `HostInfoData`
 * - `pair_begin{ttl_secs?}` → `PairBeginData`
 * - `pair_status{code, cancel?}` → `PairStatusData`
 * - `devices_list` → `DevicesListData`
 * - `device_revoke{id}` → `ok{}`
 */

export const PAIR_TTL_MAX = 3600;
export const PAIR_TTL_DEFAULT = 600;

export const HostInfoRequestSchema = z.object({ op: z.literal("host_info") });
export const PairBeginRequestSchema = z.object({ op: z.literal("pair_begin"), ttl_secs: z.number().int().min(30).max(PAIR_TTL_MAX).optional() });
export const PairStatusRequestSchema = z.object({
  op: z.literal("pair_status"),
  code: z.string().min(4).max(64).regex(/^[A-Za-z0-9_-]+$/),
  cancel: z.boolean().optional(),
});
export const DevicesListRequestSchema = z.object({ op: z.literal("devices_list") });
export const DeviceRevokeRequestSchema = z.object({ op: z.literal("device_revoke"), id: z.string().min(8).max(64).regex(/^[0-9a-fA-F]+$/) });

export const HostToolsSchema = z.object({ herdr: z.boolean(), tmux: z.boolean(), pi: z.boolean(), kanboard: z.boolean() });

export const HostInfoDataSchema = z.object({
  version: z.string(),
  proto: z.string(),
  mode: z.enum(["service", "app", "manual"]),
  pid: z.number().int(),
  started_at: z.number().int(),
  host_id: z.string(),
  host_name: z.string(),
  data_dir: z.string(),
  tools: HostToolsSchema,
  devices: z.number().int().min(0),
});
export type HostInfoData = z.infer<typeof HostInfoDataSchema>;

export const PairBeginDataSchema = z.object({ link: z.string().min(1), code: z.string().min(1), expires_at: z.number().int(), host_name: z.string() });
export type PairBeginData = z.infer<typeof PairBeginDataSchema>;

export const PairStatusDataSchema = z.discriminatedUnion("state", [
  z.object({ state: z.literal("waiting"), expires_at: z.number().int() }),
  z.object({ state: z.literal("paired"), device: z.string() }),
  z.object({ state: z.literal("expired"), reason: z.enum(["timeout", "replaced", "locked", "cancelled"]) }),
]);
export type PairStatusData = z.infer<typeof PairStatusDataSchema>;

export const DeviceItemSchema = z.object({ id: z.string(), name: z.string(), paired_at: z.number().int(), last_seen: z.number().int().optional() });
export type DeviceItem = z.infer<typeof DeviceItemSchema>;
export const DevicesListDataSchema = z.object({ devices: z.array(DeviceItemSchema) });
export type DevicesListData = z.infer<typeof DevicesListDataSchema>;
