import { z } from "zod";
import { AppErrorSchema } from "./error.js";
import { FsPathSchema } from "./fs.js";

/**
 * Session streams and the hub event channel (host.md §7.3–7.5, §4.2; WP-H4).
 *
 * One persistent bi-stream per open chat (`session_open` / `session_new` /
 * `session_fork`):
 *
 * ```text
 * client → host : hello{req: session_open{session, since_seq?}}
 * host → client : ok{data: SessionSnapshot}            (first frame)
 * client → host : Control::SessionCmd{id?, cmd}        (repeatable)
 * host → client : SessionReply (ok|err, id echoed)     (correlated)
 * host → client : events{items: [SessionEventDto…]}    (live, ≤1 frame/16 ms)
 * ```
 *
 * A `SessionSnapshot` with `reset: true` is a full snapshot (`state`,
 * `entries` of the current branch, `leaf_id`, `commands`, `ui` — the
 * ui-protocol §9.2 registry snapshot — and `topics`, the sticky values).
 * `reset: false` resumes: `replay` carries the buffered events after the
 * client's `sinceSeq`; the app applies them in order.
 *
 * `seq` numbers the events of one session from 1; the host keeps the last
 * 4096. A client that reconnects with its last seen `sinceSeq` gets the
 * replay when the host can prove continuity and a snapshot otherwise — a
 * reset is always allowed, a gap never is.
 *
 * Per ui-protocol D-U14 only envelopes are parsed with zod: `event.event`
 * (oino's `ClientEvent` JSON) and `ui` (registry nodes) are narrowed by the
 * session reducer by `type` / applied verbatim.
 */

/** Longest `session` target: an id, a unique prefix (≥4 chars) or a file path. */
export const SESSION_TARGET_MAX = 4096;
/** Longest session name (written into a `session_info` entry). */
export const SESSION_NAME_MAX = 256;
/** Event kinds the host adds to core's `ClientEvent` set (host.md §7.4). */
export const HOST_EVENT_KINDS = ["crashed", "taken_over", "reloaded", "woken", "wake_failed"] as const;

const trimmedString = (max: number, what: string) =>
  z
    .string()
    .transform((value) => value.trim())
    .refine((value) => value.length >= 1 && value.length <= max, {
      message: `${what} must be 1..=${max} characters`,
    });

/** A `session` target: exact id, unique prefix or session file path. */
export const SessionTargetSchema = trimmedString(SESSION_TARGET_MAX, "session");
/** A session name (single line, 1..=256 characters). */
export const SessionNameSchema = trimmedString(SESSION_NAME_MAX, "name").refine(
  (value) => !/[\u0000\n\r]/.test(value),
  { message: "name must be a single line" },
);

/** The run state of a card (host.md §4.2 `HubState`). */
export const RunStateSchema = z.enum(["idle", "working", "crashed", "taken_over"]);
export type RunState = z.infer<typeof RunStateSchema>;

/** Line changes of a session (app spec §3.14): the sidebar row's `+N −M`. */
export const CardChangesSchema = z.object({ add: z.number().int().min(0), del: z.number().int().min(0) });
export type CardChanges = z.infer<typeof CardChangesSchema>;

/** One row of `sessions_list` and of a `session_changed` hub event (host.md §4.2). */
export const SessionCardSchema = z
  .object({
    session_id: z.string().min(1),
    /** Session file (`""` before the first message is written). */
    file: z.string().optional(),
    name: z.string().nullable().optional(),
    cwd: z.string().optional(),
    /** Last activity, epoch milliseconds. */
    updated_at: z.number().optional(),
    message_count: z.number().int().min(0).optional(),
    run_state: RunStateSchema.default("idle"),
    /** A dialog waits for the user. */
    needs_you: z.boolean().optional(),
    model: z.string().nullable().optional(),
    /** Panic text when `run_state` is `crashed`. */
    error: z.string().optional(),
    /** Who took over when `run_state` is `taken_over` (`tui`, `host:<name>`, `cli`). */
    from: z.string().optional(),
    /** Pinned to the sidebar's Pinned group (Z-14, app spec §3.14). */
    pinned: z.boolean().optional(),
    /** Ended a run / needed the user while unfocused (Z-86). */
    unread: z.boolean().optional(),
    /** A manual name beats auto-rename (Z-15). */
    title_locked: z.boolean().optional(),
    /** Who runs the session when it is not the user (`"cron"`; A22 fills it). */
    source: z.string().optional(),
    /** The cron job whose run owns the session (`source: "cron"`): its id … */
    cron_job_id: z.string().optional(),
    /** … and its name, when the job still exists (the chat header's cron chip). */
    cron_job_name: z.string().optional(),
    /** Line changes (`+N −M`) when a summary exists. */
    changes: CardChangesSchema.optional(),
    /** Last activity the host knows (epoch ms); the row's relative time. */
    last_activity: z.number().optional(),
    /** Work the session waits on between runs that will wake it (bg task with a wake, background
     *  subagents, graph nodes); `run_state` stays `working` meanwhile. */
    pending_work: z.string().optional(),
    /** Quiet background work (a bg task without wake) on an idle session: a quiet marker, not "Working". */
    background_work: z.string().optional(),
    /** When the composer draft was last saved (epoch ms; absent = no draft): the sidebar's "Draft". */
    draft_at: z.number().optional(),
  })
  .passthrough();
export type SessionCard = z.infer<typeof SessionCardSchema>;

/** The patch of `session_meta_set` (app spec §3.14): only the fields present change. */
export const SessionMetaPatchSchema = z
  .object({
    pinned: z.boolean().optional(),
    unread: z.boolean().optional(),
    title_locked: z.boolean().optional(),
  })
  .strict();
export type SessionMetaPatch = z.infer<typeof SessionMetaPatchSchema>;

/** `sessions_list` → the cards, newest activity first (a bare array, like `term_list`). */
export const SessionsListDataSchema = z.array(SessionCardSchema);
export type SessionsListData = z.infer<typeof SessionsListDataSchema>;

/**
 * One numbered event of a session stream (`{session, seq, event}`): `event` is
 * oino's `ClientEvent` JSON verbatim, or a host-only `crashed` / `taken_over` /
 * `reloaded`.
 */
export const SessionEventDtoSchema = z
  .object({
    session: z.string(),
    seq: z.number().int().min(1),
    event: z
      .object({ type: z.string().optional(), kind: z.string().optional() })
      .passthrough(),
  })
  .passthrough();
export type SessionEventDto = z.infer<typeof SessionEventDtoSchema>;

/** The data of `Control::Events`: one flush tick's batch. */
export const SessionEventsSchema = z.object({ items: z.array(SessionEventDtoSchema) });
export type SessionEvents = z.infer<typeof SessionEventsSchema>;

/** A correlated command reply on a session stream: `{type:"ok"|"err", id?, …}`. */
export const SessionReplySchema = z.discriminatedUnion("type", [
  z
    .object({
      type: z.literal("ok"),
      id: z.union([z.string(), z.number()]).nullable().optional(),
      data: z.unknown().optional(),
    })
    .passthrough(),
  z
    .object({
      type: z.literal("err"),
      id: z.union([z.string(), z.number()]).nullable().optional(),
      error: AppErrorSchema,
    })
    .passthrough(),
]);
export type SessionReply = z.infer<typeof SessionReplySchema>;

/** The registry envelope of ui-protocol §9.2 (`UiSnapshot`); nodes stay unparsed. */
export const UiSnapshotEnvelopeSchema = z
  .object({
    scope: z.enum(["session", "host"]).default("session"),
    sessionId: z.string().nullable().optional(),
    rev: z.number().int().min(0),
    entries: z.array(z.unknown()).default([]),
    dialogs: z.array(z.unknown()).default([]),
    meta: z.record(z.string(), z.unknown()).optional(),
  })
  .passthrough();
export type UiSnapshotEnvelope = z.infer<typeof UiSnapshotEnvelopeSchema>;

/** The registry patch envelope of ui-protocol §9.2 (`UiPatch`); ops stay unparsed. */
export const UiPatchEnvelopeSchema = z
  .object({
    scope: z.enum(["session", "host"]).default("session"),
    sessionId: z.string().nullable().optional(),
    rev: z.number().int().min(0),
    ops: z.array(z.unknown()).default([]),
  })
  .passthrough();
export type UiPatchEnvelope = z.infer<typeof UiPatchEnvelopeSchema>;

/** Where a paged `entries` list sits in the branch (snapshot `page`, `get_entries{before}` reply). */
export const HistoryPageSchema = z
  .object({
    total: z.number().int().min(0),
    has_more: z.boolean(),
    cursor: z.string().optional(),
  })
  .passthrough();
export type HistoryPage = z.infer<typeof HistoryPageSchema>;

/**
 * The first frame of a session stream (host.md §7.3, §7.4): a full snapshot
 * when `reset`, else a resume patch with `replay` (the snapshot fields absent).
 */
export const SessionSnapshotSchema = z
  .object({
    session_id: z.string().min(1),
    reset: z.boolean(),
    seq: z.number().int().min(0),
    /** `RpcSessionState` (`get_state`). */
    state: z.unknown().optional(),
    /** The current branch's entries. */
    entries: z.array(z.unknown()).optional(),
    leaf_id: z.union([z.string(), z.null()]).optional(),
    commands: z.array(z.unknown()).optional(),
    /** ui-protocol registry snapshot (§9.2), applied to the U9 registry. */
    ui: UiSnapshotEnvelopeSchema.optional(),
    /** Sticky topic values of the session's subscriptions. */
    topics: z.array(z.unknown()).optional(),
    /** Resume events after `sinceSeq`. */
    replay: z.array(SessionEventDtoSchema).optional(),
    /** `entries` is the newest page of the branch; older pages: `get_entries{before: cursor, limit}`. */
    page: HistoryPageSchema.optional(),
    /** Read from the session file while the runtime wakes: `commands`/`ui`/`topics` come with `{kind:"woken"}`. */
    waking: z.boolean().optional(),
  })
  .passthrough()
  .superRefine((value, ctx) => {
    if (value.reset && value.replay !== undefined) {
      ctx.addIssue({ code: "custom", message: "a full snapshot has no replay" });
    }
    if (!value.reset && value.replay === undefined) {
      ctx.addIssue({ code: "custom", message: "a resume patch carries replay" });
    }
  });
export type SessionSnapshot = z.infer<typeof SessionSnapshotSchema>;

/** `Control::SessionCmd` (host.md §7.3): any RPC command verbatim, `id` optional. */
export const SessionCmdControlSchema = z
  .object({
    type: z.literal("session_cmd"),
    id: z.union([z.string(), z.number()]).optional(),
    cmd: z.object({ type: z.string() }).passthrough(),
  })
  .passthrough();
export type SessionCmdControl = z.infer<typeof SessionCmdControlSchema>;

/** `Control::Events` (host.md §7.3). */
export const EventsControlSchema = z
  .object({ type: z.literal("events"), items: z.array(SessionEventDtoSchema) })
  .passthrough();
export type EventsControl = z.infer<typeof EventsControlSchema>;

/** Severity of a `host_notice` hub event. */
export const NoticeLevelSchema = z.enum(["info", "warning"]);
export type NoticeLevel = z.infer<typeof NoticeLevelSchema>;

/**
 * What `events_open` streams, one per connection (host.md §7.4; ui-protocol
 * §9.2 for `ui`): card changes, removals, host notices (the safe-mode banner
 * and updater prompts), and the host-scope registry snapshot/patches.
 */
export const HubEventSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("session_changed"), card: SessionCardSchema }).passthrough(),
  z.object({ kind: z.literal("session_removed"), session: z.string() }).passthrough(),
  // [ux-sessions] (owner fix 2026-11-03): one chunk of a batch archive / restore, and
  // "this connection fell behind — re-list" after the host dropped events.
  z.object({ kind: z.literal("sessions_removed"), sessions: z.array(z.string()), archived: z.boolean().optional() }).passthrough(),
  z.object({ kind: z.literal("sessions_changed"), cards: z.array(SessionCardSchema) }).passthrough(),
  z.object({ kind: z.literal("resync") }).passthrough(),
  z
    .object({
      kind: z.literal("host_notice"),
      level: NoticeLevelSchema,
      text: z.string(),
    })
    .passthrough(),
  z
    .object({
      kind: z.literal("ui"),
      snapshot: UiSnapshotEnvelopeSchema.optional(),
      patch: UiPatchEnvelopeSchema.optional(),
    })
    .passthrough(),
  z.object({ kind: z.literal("git_changed"), root: z.string() }).passthrough(),
  // WP-A22 (app.md §3.14): a board project's files changed (the watcher is on
  // the files, not the writer — CLI and agent writes land here too).
  z.object({ kind: z.literal("board_changed"), project: z.string() }).passthrough(),
]);
export type HubEvent = z.infer<typeof HubEventSchema>;

export const SessionsListRequestSchema = z.object({
  op: z.literal("sessions_list"),
  /** App spec §3.14 (WP-A5, Z-13): the archived (trashed) sessions instead of the live list. */
  archived: z.boolean().optional(),
});

/** `session_search` page size defaults / caps (mirror `unipi_proto::session`). */
export const SESSION_SEARCH_LIMIT_DEFAULT = 50;
export const SESSION_SEARCH_LIMIT_MAX = 200;

/**
 * Owner fix 2026-11-03: one page of the host's sessions whose name / folder / id
 * match every word of `query` (empty = most recent), newest first.
 */
export const SessionSearchRequestSchema = z.object({
  op: z.literal("session_search"),
  query: z.string().max(256).optional(),
  limit: z.number().int().min(1).max(SESSION_SEARCH_LIMIT_MAX).optional(),
  cursor: z.string().optional(),
  archived: z.boolean().optional(),
});

/** `session_search` → a page of cards, the match count and the next page's cursor. */
export const SessionSearchDataSchema = z
  .object({
    cards: z.array(SessionCardSchema),
    total: z.number().int().min(0),
    next_cursor: z.string().optional(),
  })
  .passthrough();
export type SessionSearchData = z.infer<typeof SessionSearchDataSchema>;

export const EventsOpenRequestSchema = z.object({ op: z.literal("events_open") });

export const SessionNewRequestSchema = z.object({
  op: z.literal("session_new"),
  cwd: FsPathSchema.optional(),
  name: SessionNameSchema.optional(),
});

export const SessionOpenRequestSchema = z.object({
  op: z.literal("session_open"),
  session: SessionTargetSchema,
  since_seq: z.number().int().min(0).optional(),
});

export const SessionForkRequestSchema = z.object({
  op: z.literal("session_fork"),
  session: SessionTargetSchema,
  entry_id: z.string().min(1).optional(),
  name: SessionNameSchema.optional(),
});

export const SessionArchiveRequestSchema = z.object({
  op: z.literal("session_archive"),
  session: SessionTargetSchema,
});

/** App spec §3.14 (WP-A5, Z-13): move an archived session back into its project. */
export const SessionRestoreRequestSchema = z.object({
  op: z.literal("session_restore"),
  session: SessionTargetSchema,
});

/** App spec §3.14 (WP-A5, Z-13/Z-20): delete one archived session file. */
export const SessionDeleteRequestSchema = z.object({
  op: z.literal("session_delete"),
  session: SessionTargetSchema,
});

/** App spec §3.14 (WP-A5, Z-13): delete every archived session file. */
export const TrashEmptyRequestSchema = z.object({ op: z.literal("trash_empty") });

/** `trash_empty` → how many archived sessions went. */
export const TrashEmptyDataSchema = z.object({ deleted: z.number().int().min(0) }).passthrough();
export type TrashEmptyData = z.infer<typeof TrashEmptyDataSchema>;

export const SessionRenameRequestSchema = z.object({
  op: z.literal("session_rename"),
  session: SessionTargetSchema,
  name: SessionNameSchema,
});

/**
 * UniPi Oino "Move to a workspace…": a "No workspace" chat (its private scratch folder)
 * moves to a real folder from its next turn on — one way only. `copy_files` copies the
 * scratch files over first, never overwriting.
 */
export const SessionMoveRequestSchema = z.object({
  op: z.literal("session_move"),
  session: SessionTargetSchema,
  cwd: FsPathSchema,
  copy_files: z.boolean().optional(),
});

/** `session_move` → the moved card (its `cwd` is the workspace) and the copy outcome. */
export const SessionMoveDataSchema = z
  .object({
    card: SessionCardSchema,
    copied: z.number().int().min(0).default(0),
    skipped: z.number().int().min(0).default(0),
  })
  .passthrough();
export type SessionMoveData = z.infer<typeof SessionMoveDataSchema>;

export const SessionCloseRequestSchema = z.object({
  op: z.literal("session_close"),
  session: SessionTargetSchema,
});

/** Owner 2026-11-06: a session's composer draft, kept host-side (`{draft}`, `null` = none). */
export const DraftGetRequestSchema = z.object({ op: z.literal("draft_get"), session: SessionTargetSchema });
/** Sets (or clears with `null`) a session's draft → the updated card. */
export const DraftSetRequestSchema = z.object({ op: z.literal("draft_set"), session: SessionTargetSchema, draft: z.record(z.string(), z.unknown()).nullable() });

/** App spec §3.14 (WP-A4): pin / unread / title-lock flags, persisted host-side. */
export const SessionMetaSetRequestSchema = SessionMetaPatchSchema.extend({
  op: z.literal("session_meta_set"),
  session: SessionTargetSchema,
});

/**
 * Maps core's host-only event kinds (§7.4) to the banner text the session
 * store shows; `null` for a core event (the reducer handles it by `type`).
 */
export function hostEventBanner(event: { type?: string; kind?: string; error?: unknown; from?: unknown }): string | null {
  const kind = event.type ?? event.kind;
  switch (kind) {
    case "crashed":
      return typeof event.error === "string" && event.error
        ? `The session crashed: ${event.error}`
        : "The session crashed.";
    case "taken_over":
      return typeof event.from === "string" && event.from
        ? `Taken over from the ${event.from}.`
        : "Another window took this session over.";
    case "reloaded":
      return "The session was reloaded.";
    case "wake_failed":
      return typeof event.error === "string" && event.error
        ? `This chat could not start: ${event.error}`
        : "This chat could not start.";
    default:
      return null;
  }
}
