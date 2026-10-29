import { z } from "zod";
import { Sha256HexSchema } from "./control.js";

/**
 * Push a file from the PC to the phone (UNI-214, forwarding phase 2).
 * Mirrors `crates/proto/src/push.rs`:
 * - `push_list{wait?}` → `ok{data: PushListData}` (long-poll ≤ 30 s;
 *   `enabled:false` when push is off on the host, never an error).
 * - bytes via `blob_get{path}` (the push's path is readable by its device).
 * - `push_done{id, outcome, detail?}` → `ok{}`.
 */

export const PUSH_WAIT_MAX = 30;

export const PushIdSchema = z
  .string()
  .min(1)
  .max(64)
  .regex(/^[A-Za-z0-9_-]+$/, "id must be [A-Za-z0-9_-]");

export const PushWaitSchema = z.number().int().min(0).max(PUSH_WAIT_MAX);

export const PushOutcomeSchema = z.enum(["installed", "saved", "opened", "dismissed", "failed"]);
export type PushOutcome = z.infer<typeof PushOutcomeSchema>;

export const PushItemSchema = z.object({
  id: PushIdSchema,
  name: z.string(),
  size: z.number().int().min(0),
  sha256: Sha256HexSchema,
  mime: z.string(),
  path: z.string(),
  note: z.string().optional(),
  created_at: z.number().int().min(0),
});
export type PushItem = z.infer<typeof PushItemSchema>;

export const PushListDataSchema = z.object({
  enabled: z.boolean(),
  host_name: z.string(),
  pushes: z.array(PushItemSchema).default([]),
});
export type PushListData = z.infer<typeof PushListDataSchema>;
