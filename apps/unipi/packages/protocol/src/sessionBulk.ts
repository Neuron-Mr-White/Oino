/**
 * Bulk session ops and the sidebar's session query (owner fix 2026-11-03,
 * `[ux-sessions]`; mirrors `unipi_proto::session_bulk`).
 *
 * - `sessions_archive{sessions}` / `sessions_restore{sessions}`: one host op for
 *   N chats ("Archive all" and its Undo). Removals / restores ride `events_open`
 *   as `sessions_removed` / `sessions_changed` chunks — the app's progress.
 * - `sessions_query{…}`: one page of the sidebar search over title + first user
 *   message, live and/or archived chats, with the list's filters host-side.
 */
import { z } from "zod";
import { SessionCardSchema, SessionTargetSchema } from "./session.js";

export const SESSIONS_BATCH_MAX = 5_000;
export const SESSIONS_EVENT_CHUNK = 200;
export const SESSIONS_QUERY_LIMIT_DEFAULT = 50;
export const SESSIONS_QUERY_LIMIT_MAX = 100;
export const SESSIONS_QUERY_TEXT_MAX = 256;

export const SessionsArchiveRequestSchema = z.object({
  op: z.literal("sessions_archive"),
  sessions: z.array(SessionTargetSchema).min(1).max(SESSIONS_BATCH_MAX),
});

export const SessionsRestoreRequestSchema = z.object({
  op: z.literal("sessions_restore"),
  sessions: z.array(SessionTargetSchema).min(1).max(SESSIONS_BATCH_MAX),
});

/** `sessions_archive` / `sessions_restore` → which ids went, which failed. */
export const SessionsBatchDataSchema = z
  .object({
    done: z.array(z.string()),
    failed: z.array(z.object({ session: z.string(), message: z.string() }).passthrough()).optional(),
  })
  .passthrough();
export type SessionsBatchData = z.infer<typeof SessionsBatchDataSchema>;

export const SessionsQueryScopeSchema = z.enum(["live", "archived", "all"]);
export type SessionsQueryScope = z.infer<typeof SessionsQueryScopeSchema>;
export const SessionsQuerySortSchema = z.enum(["updated", "created", "name"]);
export type SessionsQuerySort = z.infer<typeof SessionsQuerySortSchema>;

const RangeSchema = z.object({ from: z.number().int().optional(), to: z.number().int().optional() });

export const SessionsQueryFiltersSchema = z.object({
  workspaces: z.array(z.string()).max(256).optional(),
  automation: z.array(z.enum(["cron", "manual"])).max(2).optional(),
  status: z.array(z.enum(["running", "waiting", "ready", "failed", "timedout"])).max(5).optional(),
  models: z.array(z.string()).max(256).optional(),
  created: RangeSchema.optional(),
  updated: RangeSchema.optional(),
});
export type SessionsQueryFilters = z.infer<typeof SessionsQueryFiltersSchema>;

export const SessionsQueryRequestSchema = z.object({
  op: z.literal("sessions_query"),
  query: z.string().max(SESSIONS_QUERY_TEXT_MAX).optional(),
  scope: SessionsQueryScopeSchema.optional(),
  filters: SessionsQueryFiltersSchema.optional(),
  sort: SessionsQuerySortSchema.optional(),
  limit: z.number().int().min(1).max(SESSIONS_QUERY_LIMIT_MAX).optional(),
  cursor: z.string().optional(),
});

export const SessionsQueryHitSchema = z
  .object({ card: SessionCardSchema, archived: z.boolean().optional(), snippet: z.string().optional() })
  .passthrough();
export type SessionsQueryHit = z.infer<typeof SessionsQueryHitSchema>;

/** `sessions_query` → one page, the match count and the next page's cursor. */
export const SessionsQueryDataSchema = z
  .object({
    hits: z.array(SessionsQueryHitSchema),
    total: z.number().int().min(0),
    next_cursor: z.string().optional(),
  })
  .passthrough();
export type SessionsQueryData = z.infer<typeof SessionsQueryDataSchema>;
