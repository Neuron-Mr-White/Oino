import { z } from "zod";
import { SemverSchema } from "./version.js";
import { AppErrorSchema } from "./error.js";
import { FsPathSchema, SearchQuerySchema, SearchKindSchema } from "./fs.js";
import { GitRequestSchema } from "./git.js";
import { ForwardPortSchema } from "./forward.js";
import { BrowserCmdSchema, BrowserEventSchema, BrowserProfileSchema, BrowserUrlSchema, BrowserViewportSchema } from "./browser.js";
import { DesktopCmdSchema, DesktopEventSchema, DesktopQualitySchema, DesktopViewportSchema } from "./desktop.js";
import { FEATURE_NOTE_MAX, FeatureKindSchema, FeatureLeaveScopeSchema, FeatureScopeSchema } from "./sharing.js";
import { ChatCloseRequestSchema, ChatConfigRequestSchema, ChatConfigSetRequestSchema, ChatNewRequestSchema, ChatNewTargetSchema } from "./chat.js";
import { DeviceRevokeRequestSchema, DevicesListRequestSchema, HostInfoRequestSchema, PairBeginRequestSchema, PairStatusRequestSchema } from "./admin.js";
import { HostSettingsGetRequestSchema, HostSettingsSetRequestSchema } from "./settings.js";
import {
  CronAddRequestSchema,
  CronEditRequestSchema,
  CronJobIdSchema,
  CronRunIdSchema,
  CronRunsLimitSchema,
  CronSettingsSetRequestSchema,
} from "./cron.js";

/** Multiplexer kind: herdr (first-party) or tmux. */
export const MuxKindSchema = z.enum(["herdr", "tmux"]);
export type MuxKind = z.infer<typeof MuxKindSchema>;

/** How the terminal reacts to the phone's own viewport size. */
export const SizePolicySchema = z.enum(["fit_phone", "keep_pc"]);
export type SizePolicy = z.infer<typeof SizePolicySchema>;

const SessionIdSchema = z
  .string()
  .min(1, "session must be at least 1 character")
  .max(128, "session must be at most 128 characters");

const DimensionSchema = z
  .number()
  .int("must be an integer")
  .min(1, "must be at least 1")
  .max(1000, "must be at most 1000");

const PairCodeSchema = z
  .string()
  .min(6, "code must be at least 6 characters")
  .max(64, "code must be at most 64 characters");

const PairNameSchema = z
  .string()
  .min(1, "name must be at least 1 character")
  .max(64, "name must be at most 64 characters");

/** Z-base-32 (RFC 4648 variant used by iroh) or lowercase-hex endpoint id. */
const HOST_ID_HEX_RE = /^[0-9a-f]{64}$/;
const HOST_ID_Z32_RE = /^[ybndrfg8ejkmcpqxot1uwisza345h769]{52}$/;

const HostIdSchema = z
  .string()
  .refine(
    (value) => HOST_ID_HEX_RE.test(value) || HOST_ID_Z32_RE.test(value),
    "host_id must be a 64-char lowercase hex string or a 52-char z32 string",
  );

const HostNameSchema = z
  .string()
  .min(1, "host_name must be at least 1 character")
  .max(64, "host_name must be at most 64 characters");

const SessionNameSchema = z
  .string()
  .min(1, "name must be at least 1 character")
  .max(128, "name must be at most 128 characters");

/** One session entry returned by a `list` request. */
export const SessionSummarySchema = z.object({
  name: SessionNameSchema,
  running: z.boolean(),
  default: z.boolean(),
});

export type SessionSummary = z.infer<typeof SessionSummarySchema>;

/** `ok.data` payload for a successful `list` request. */
export const ListDataSchema = z.object({
  sessions: z.array(SessionSummarySchema),
});

export type ListData = z.infer<typeof ListDataSchema>;

/** `ok.data` payload for a successful `pair` request. */
export const PairDataSchema = z.object({
  host_id: HostIdSchema,
  host_name: HostNameSchema,
});

export type PairData = z.infer<typeof PairDataSchema>;

/** `ok.data` payload for a successful `attach` request. */
export const AttachDataSchema = z.object({
  effective_size_policy: SizePolicySchema,
});

export type AttachData = z.infer<typeof AttachDataSchema>;

/** A herdr identifier (`w4E`, `w4E:t16`, `w4E:p18`): length-bounded, no whitespace/control characters. */
const HerdrIdSchema = z
  .string()
  .min(1, "herdr id must not be empty")
  .max(128, "herdr id must be at most 128 characters")
  .refine((id) => !/[\s\u0000-\u001f\u007f]/.test(id), "herdr id must not contain whitespace or control characters");

/** A tmux session id: `$3`. */
const TmuxSessionIdSchema = z.string().regex(/^\$\d+$/, "tmux session id must look like $3");
/** A tmux window id: `@5`. */
const TmuxWindowIdSchema = z.string().regex(/^@\d+$/, "tmux window id must look like @5");
/** A tmux pane id: `%7`. */
const TmuxPaneIdSchema = z.string().regex(/^%\d+$/, "tmux pane id must look like %7");

/** Where in a herdr session to attach (DESIGN.md §5.6). */
export const HerdrTargetSchema = z.object({
  mux: z.literal("herdr"),
  session: SessionIdSchema,
  workspace_id: HerdrIdSchema.optional(),
  tab_id: HerdrIdSchema.optional(),
  pane_id: HerdrIdSchema.optional(),
});

/** Where in a tmux server to attach (DESIGN.md §5.6). */
export const TmuxTargetSchema = z.object({
  mux: z.literal("tmux"),
  session: z
    .string()
    .min(1, "session must be at least 1 character")
    .max(128, "session must be at most 128 characters"),
  window_id: TmuxWindowIdSchema.optional(),
  pane_id: TmuxPaneIdSchema.optional(),
});

export const AttachTargetSchema = z.discriminatedUnion("mux", [HerdrTargetSchema, TmuxTargetSchema]);
export type AttachTarget = z.infer<typeof AttachTargetSchema>;

/** `ok.data` payload for a `preview` request: pane text with ANSI kept. */
export const PreviewDataSchema = z.object({
  text: z.string().max(64 * 1024, "preview text must be at most 64 KiB"),
});
export type PreviewData = z.infer<typeof PreviewDataSchema>;

export const AgentStatusValueSchema = z.enum(["idle", "working", "blocked", "done", "unknown"]);
export type AgentStatusValue = z.infer<typeof AgentStatusValueSchema>;

/** The pi (or other agent) binding for one pane (DESIGN.md §5.6). */
export const PaneAgentSchema = z.object({
  name: z.string().min(1),
  status: AgentStatusValueSchema,
  session: z.object({ kind: z.string().min(1), value: z.string().min(1) }).optional(),
});
export type PaneAgent = z.infer<typeof PaneAgentSchema>;

export type HerdrPane = z.infer<typeof HerdrPaneSchema>;
export const HerdrPaneSchema = z.object({
  id: HerdrIdSchema,
  focused: z.boolean(),
  title: z.string().max(512).optional(),
  terminal_title: z.string().max(512).optional(),
  cwd: z.string().min(1).optional(),
  agent: PaneAgentSchema.optional(),
  /** The pi chat bridge in this pane (chat_list only). */
  chat: z
    .object({
      pid: z.number().int().min(1),
      session_name: z.string().optional(),
      cwd: z.string().optional(),
      /** Set while this pi is waiting on the user (UNI-161 "needs you"): an
       * open dialog, or idle right after an `agent_end` the phone hasn't
       * seen yet. Cleared once answered/opened. */
      waiting: z
        .object({
          kind: z.string().min(1),
          title: z.string().optional(),
          since: z.number().int().min(0),
        })
        .optional(),
      /** pi's own run state from the bridge record (UNI-212): herdr's
       * pane `agent_status` can miss a run. Absent from older bridges. */
      running: z.boolean().optional(),
      /** UNI-221: wake-capable work still pending while pi is idle (bg
       * task with a wake, background subagent, fusion handoff) — the app
       * shows "Working…". Absent when nothing is pending / older bridges. */
      pending_work: z.string().optional(),
      /** UNI-269: the long-horizon working mode (goal/ralph/swarm/graph) —
       * the list's mode badge. Absent = regular / older bridges. */
      lh_mode: z.string().max(32).optional(),
      /** UNI-269: a parked long-horizon owner's mode. */
      lh_paused: z.string().max(32).optional(),
    })
    .optional(),
});

export type HerdrTab = z.infer<typeof HerdrTabSchema>;
export const HerdrTabSchema = z.object({
  id: HerdrIdSchema,
  label: z.string().min(1).max(128),
  number: z.number().int().min(0),
  focused: z.boolean(),
  agent_status: AgentStatusValueSchema,
  panes: z.array(HerdrPaneSchema),
});

export type HerdrWorkspace = z.infer<typeof HerdrWorkspaceSchema>;
export const HerdrWorkspaceSchema = z.object({
  id: HerdrIdSchema,
  label: z.string().min(1).max(128),
  number: z.number().int().min(0),
  focused: z.boolean(),
  agent_status: AgentStatusValueSchema,
  tab_count: z.number().int().min(0),
  pane_count: z.number().int().min(0),
  tabs: z.array(HerdrTabSchema),
});

export type HerdrSession = z.infer<typeof HerdrSessionSchema>;
export const HerdrSessionSchema = z.object({
  name: SessionNameSchema,
  running: z.boolean(),
  default: z.boolean(),
  workspaces: z.array(HerdrWorkspaceSchema),
  /** Set when this session's snapshot could not be read. */
  error: z.string().min(1).optional(),
});

export const MuxUnavailableSchema = z.object({ available: z.literal(false), reason: z.string().min(1) });
export const MuxAvailable = <T extends z.ZodTypeAny>(sessions: T) =>
  z.object({ available: z.literal(true), sessions });

export const HerdrStateSchema = z.discriminatedUnion("available", [
  MuxUnavailableSchema,
  MuxAvailable(z.array(HerdrSessionSchema)),
]);

export type TmuxPane = z.infer<typeof TmuxPaneSchema>;
export const TmuxPaneSchema = z.object({
  id: TmuxPaneIdSchema,
  index: z.number().int().min(0),
  active: z.boolean(),
  current_command: z.string(),
  cwd: z.string(),
  pid: z.number().int().min(0),
  title: z.string(),
  agent: PaneAgentSchema.optional(),
  /** UNI-260: the pi chat bridge in this pane (chat_list only). */
  chat: z.object({ pid: z.number().int().min(1) }).passthrough().optional(),
});

export type TmuxWindow = z.infer<typeof TmuxWindowSchema>;
export const TmuxWindowSchema = z.object({
  id: TmuxWindowIdSchema,
  index: z.number().int().min(0),
  name: z.string(),
  active: z.boolean(),
  panes: z.array(TmuxPaneSchema),
});

export type TmuxSession = z.infer<typeof TmuxSessionSchema>;
export const TmuxSessionSchema = z.object({
  id: TmuxSessionIdSchema,
  name: z.string().min(1),
  attached: z.number().int().min(0),
  window_count: z.number().int().min(0),
  created: z.number().int().min(0),
  activity: z.number().int().min(0),
  windows: z.array(TmuxWindowSchema),
});

export const TmuxStateSchema = z.discriminatedUnion("available", [
  MuxUnavailableSchema,
  MuxAvailable(z.array(TmuxSessionSchema)),
]);

export type HerdrState = z.infer<typeof HerdrStateSchema>;
export type TmuxState = z.infer<typeof TmuxStateSchema>;

/** `ok.data` payload for a `tree` request (DESIGN.md §5.6). */
export const TreeDataSchema = z.object({
  protected: z.array(z.string()).optional(),
  herdr: HerdrStateSchema,
  tmux: TmuxStateSchema,
});
export type TreeData = z.infer<typeof TreeDataSchema>;

/**
 * The request carried in a `hello` control message. Internally
 * tagged on `op`, mirroring the Rust enum's `#[serde(tag = "op")]`.
 */
/** One multiplexer mutation/navigation (DESIGN §3.3, §3.1.1). Tagged
 * on `action`; pane cycling is mux-prefixed because a session name can
 * exist in both multiplexers. */
export const MuxActionSchema = z.discriminatedUnion("action", [
  z.object({
    action: z.literal("workspace_create"),
    session: SessionNameSchema,
    cwd: z.string().min(1).optional(),
    label: SessionNameSchema.optional(),
  }),
  z.object({
    action: z.literal("workspace_rename"),
    session: SessionNameSchema,
    workspace_id: HerdrIdSchema,
    label: SessionNameSchema,
  }),
  z.object({
    action: z.literal("workspace_close"),
    session: SessionNameSchema,
    workspace_id: HerdrIdSchema,
  }),
  z.object({ action: z.literal("tab_next"), session: SessionNameSchema }),
  z.object({ action: z.literal("tab_prev"), session: SessionNameSchema }),
  z.object({ action: z.literal("herdr_pane_next"), session: SessionNameSchema }),
  z.object({ action: z.literal("herdr_pane_prev"), session: SessionNameSchema }),
  z.object({
    action: z.literal("tmux_session_create"),
    name: SessionNameSchema,
    cwd: z.string().min(1).optional(),
  }),
  z.object({
    action: z.literal("tmux_session_rename"),
    session: SessionNameSchema,
    name: SessionNameSchema,
  }),
  z.object({ action: z.literal("tmux_session_kill"), session: SessionNameSchema }),
  z.object({ action: z.literal("window_next"), session: SessionNameSchema }),
  z.object({ action: z.literal("window_prev"), session: SessionNameSchema }),
  z.object({ action: z.literal("tmux_pane_next"), session: SessionNameSchema }),
  z.object({ action: z.literal("tmux_pane_prev"), session: SessionNameSchema }),
]);
export type MuxAction = z.infer<typeof MuxActionSchema>;

/** One shortcuts-panel entry: a named action and its key (the
 * prefix is shown separately). */
export const ShortcutKeySchema = z.object({
  name: z.string().min(1),
  key: z.string().min(1),
});
export const ShortcutMuxSchema = z.object({
  prefix: z.string().min(1),
  keys: z.array(ShortcutKeySchema),
});
export type ShortcutKey = z.infer<typeof ShortcutKeySchema>;
export type ShortcutMux = z.infer<typeof ShortcutMuxSchema>;
export const ShortcutsDataSchema = z.object({
  herdr: ShortcutMuxSchema,
  tmux: ShortcutMuxSchema,
});
export type ShortcutsData = z.infer<typeof ShortcutsDataSchema>;

/** Board relay (DESIGN.md §3.9 "Data path"); mirrors `crates/proto/src/board.rs`. */
export const BOARD_MAX_BODY = 26 * 1024 * 1024;
export const BOARD_MAX_PATH = 4096;

/**
 * The board path allow-list: `/api/…` only, ≤ 4096 chars, no control
 * characters, `#` or `\`, and no empty or dot (`.`, `..`, `%2e`) segments
 * before the query. Returns the reason, or null when allowed.
 */
export function boardPathError(path: string): string | null {
  if (!path.startsWith("/api/")) return "path must start with /api/";
  const length = [...path].length;
  if (length > BOARD_MAX_PATH) return `path must be at most ${BOARD_MAX_PATH} characters, got ${length}`;
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001f\u007f-\u009f#\\]/.test(path)) return "path must not contain control characters, '#' or '\\'";
  const route = path.split("?", 1)[0] ?? "";
  for (const segment of route.split("/").slice(1)) {
    if (segment === "") return "path must not contain empty segments";
    const decoded = segment.toLowerCase().replaceAll("%2e", ".");
    if (decoded === "." || decoded === "..") return "path must not contain dot segments";
  }
  return null;
}

export const BoardMethodSchema = z.enum(["GET", "POST", "PUT"]);
export const BoardPathSchema = z.string().superRefine((path, ctx) => {
  const reason = boardPathError(path);
  if (reason) ctx.addIssue({ code: "custom", message: reason });
});
export const BoardProjectSchema = z
  .string()
  .min(1, "project must be at least 1 character")
  .max(128, "project must be at most 128 characters")
  .regex(/^[A-Za-z0-9][A-Za-z0-9._-]*$/, "project must be a slug ([A-Za-z0-9._-], starting alphanumeric)");
const ContentTypeSchema = z
  .string()
  .min(1)
  .max(256)
  .regex(/^[\x20-\x7e]+$/, "content_type must be printable ASCII");

/** Blob channel (UNI-157 "media + attachments"); mirrors `crates/proto/src/blob.rs`. */
/** UNI-278: 4 GiB (was 50 MiB); uploads stream to disk. */
export const BLOB_MAX_SIZE = 4 * 1024 * 1024 * 1024;

export const Sha256HexSchema = z
  .string()
  .regex(/^[0-9a-f]{64}$/, "sha256 must be 64 lowercase hex characters");

export const BlobSizeSchema = z
  .number()
  .int()
  .min(1, "size must be at least 1")
  .max(BLOB_MAX_SIZE, `size must be at most ${BLOB_MAX_SIZE} bytes`);

export const BlobMimeSchema = z
  .string()
  .min(1)
  .max(256)
  .regex(/^[\x20-\x7e]+$/, "mime must be printable ASCII");

export const BlobNameSchema = z
  .string()
  .min(1)
  .max(256)
  .refine((v) => !/[\u0000-\u001f\u007f/\\]/.test(v), "name must not contain control characters or path separators");

export const BlobPathSchema = z
  .string()
  .min(1)
  .max(4096)
  .refine((v) => !v.includes("\u0000"), "path must not contain NUL bytes");

export const BlobRangeSchema = z.object({
  start: z.number().int().min(0),
  end: z.number().int().min(0),
});

export const BlobPutDataSchema = z.object({ have: z.boolean(), path: z.string() });
export type BlobPutData = z.infer<typeof BlobPutDataSchema>;

export const BlobGetDataSchema = z.object({ size: z.number().int().min(0), mime: z.string() });
export type BlobGetData = z.infer<typeof BlobGetDataSchema>;

/** `ok.data` for `board_http`; the body follows in kind-1 frames, ended by `exit`. */
export const BoardHttpDataSchema = z.object({
  status: z.number().int().min(100).max(599),
  content_type: z.string(),
});
export type BoardHttpData = z.infer<typeof BoardHttpDataSchema>;

/** A UnifiedPush/ntfy endpoint URL (UNI-161 §2): 1..=2048 chars, https://. */
const NotifyEndpointSchema = z
  .string()
  .min(1, "endpoint must be at least 1 character")
  .max(2048, "endpoint must be at most 2048 characters")
  .refine((value) => value.startsWith("https://"), "endpoint must be an https:// URL");

/** `ok.data` for `notify_config` (UNI-161 §1): the host's own ntfy
 * server/topic/token (from `~/.unipi/config/notify/config.json`'s `ntfy`
 * block) plus the per-pairing E2E key (base64, 32 raw bytes). `app_detail`
 * mirrors the pi-side `notify.ntfy.appDetail` setting. */
export const NotifyConfigDataSchema = z.object({
  server: z.string().optional(),
  topic: z.string().optional(),
  token: z.string().optional(),
  key: z.string(),
  app_detail: z.enum(["minimal", "full"]).default("minimal"),
});
export type NotifyConfigData = z.infer<typeof NotifyConfigDataSchema>;

const BaseRequestSchema = z.discriminatedUnion("op", [
  z.object({
    op: z.literal("list"),
    mux: MuxKindSchema,
  }),
  z.object({
    op: z.literal("attach"),
    target: AttachTargetSchema,
    cols: DimensionSchema,
    rows: DimensionSchema,
    size_policy: SizePolicySchema,
  }),
  z.object({ op: z.literal("tree") }),
  z.object({ op: z.literal("shell"), cols: DimensionSchema, rows: DimensionSchema }),
  z.object({
    op: z.literal("preview"),
    target: AttachTargetSchema,
    lines: z.number().int().min(1, "lines must be at least 1").max(200, "lines must be at most 200"),
  }),
  z.object({
    op: z.literal("pair"),
    code: PairCodeSchema,
    name: PairNameSchema.optional(),
  }),
  z.object({
    op: z.literal("board_http"),
    method: BoardMethodSchema,
    path: BoardPathSchema,
    content_type: ContentTypeSchema.optional(),
    body_len: z.number().int().min(0).max(BOARD_MAX_BODY).optional(),
  }),
  z.object({ op: z.literal("board_sse"), project: BoardProjectSchema }),
  z.object({ op: z.literal("chat_list") }),
  z.object({ op: z.literal("chat_open"), pid: z.number().int().min(1).max(4294967295) }),
  z.object({
    op: z.literal("blob_put"),
    sha256: Sha256HexSchema,
    size: BlobSizeSchema,
    mime: BlobMimeSchema,
    name: BlobNameSchema.optional(),
    offset: z.number().int().min(0).optional(),
  }),
  z.object({
    op: z.literal("blob_get"),
    sha256: Sha256HexSchema.optional(),
    path: BlobPathSchema.optional(),
    range: BlobRangeSchema.optional(),
  }),
  z.object({ op: z.literal("fs_roots") }),
  z.object({
    op: z.literal("fs_list"),
    path: FsPathSchema,
    hidden: z.boolean().optional(),
    outside: z.boolean().optional(),
  }),
  z.object({
    op: z.literal("fs_read"),
    path: FsPathSchema,
    range: BlobRangeSchema.optional(),
    outside: z.boolean().optional(),
    // UNI-191: "Edit as text" for any file type — lossily decode a
    // binary file as UTF-8 instead of refusing with E_BINARY.
    as_text: z.boolean().optional(),
  }),
  z.object({
    op: z.literal("fs_write"),
    path: FsPathSchema,
    text: z.string(),
    base_hash: z.string().optional(),
    create: z.boolean().optional(),
    outside: z.boolean().optional(),
  }),
  z.object({
    op: z.literal("fs_stat"),
    path: FsPathSchema,
    outside: z.boolean().optional(),
  }),
  z.object({
    op: z.literal("fs_search"),
    root: FsPathSchema,
    query: SearchQuerySchema,
    kind: SearchKindSchema,
    outside: z.boolean().optional(),
  }),
  z.object({
    op: z.literal("fs_mkdir"),
    path: FsPathSchema,
    outside: z.boolean().optional(),
  }),
  z.object({
    op: z.literal("fs_rename"),
    from: FsPathSchema,
    to: FsPathSchema,
    outside: z.boolean().optional(),
  }),
  z.object({
    op: z.literal("fs_import"),
    sha256: Sha256HexSchema,
    to: FsPathSchema,
    overwrite: z.boolean().optional(),
    outside: z.boolean().optional(),
  }),
  z.object({
    op: z.literal("fs_delete"),
    path: FsPathSchema,
    confirm: z.boolean().default(false),
    outside: z.boolean().optional(),
  }),
  // UNI-217: basic git on the PC (see git.ts).
  GitRequestSchema,
  z.object({ op: z.literal("notify_config") }),
  z.object({ op: z.literal("notify_register"), endpoint: NotifyEndpointSchema }),
  z.object({ op: z.literal("notify_test") }),
  // UNI-213: port forwarding (off by default on the host).
  z.object({ op: z.literal("ports_list") }),
  z.object({ op: z.literal("tcp_forward"), port: ForwardPortSchema }),
  // UNI-214: files pushed from the PC (`unipi-host push`).
  z.object({ op: z.literal("push_list"), wait: z.number().int().min(0).max(30).optional() }),
  z.object({
    op: z.literal("push_done"),
    id: z.string().min(1).max(64).regex(/^[A-Za-z0-9_-]+$/),
    outcome: z.enum(["installed", "saved", "opened", "dismissed", "failed"]),
    detail: z.string().optional(),
  }),  // UNI-215: the host's dedicated browser over CDP (off by default on the host).
  z.object({ op: z.literal("browser_status") }),
  z.object({
    op: z.literal("browser_open"),
    viewport: BrowserViewportSchema,
    url: BrowserUrlSchema.optional(),
    target: z.string().optional(),
    // UNI-292 (1.16.0): the browser profile (default "default").
    profile: BrowserProfileSchema.optional(),
  }),
  z.object({ op: z.literal("browser_close") }),
  // UNI-268 (proto 1.13.0): show/hide the dedicated browser's window on the PC.
  z.object({ op: z.literal("browser_window"), show: z.boolean() }),
  // UNI-296 (proto 1.17.0): list profiles; take over / give back a browser.
  z.object({ op: z.literal("browser_profiles") }),
  z.object({ op: z.literal("browser_takeover"), on: z.boolean(), profile: BrowserProfileSchema.optional() }),
  // UNI-216: stream the PC's desktop with control (off by default on the host).
  z.object({ op: z.literal("desktop_status") }),
  z.object({
    op: z.literal("desktop_open"),
    viewport: DesktopViewportSchema,
    quality: DesktopQualitySchema.optional(),
    view_only: z.boolean().optional(),
    /** UNI-271: monitor id from `desktop_status.displays`. */
    display: z.string().optional(),
  }),
  // UNI-248: ask the PC's owner to switch a feature on (see sharing.ts).
  z.object({
    op: z.literal("feature_request"),
    feature: FeatureKindSchema,
    scope: FeatureScopeSchema.optional(),
    note: z.string().max(FEATURE_NOTE_MAX).optional(),
  }),
  z.object({ op: z.literal("feature_requests") }),
  // UNI-253: turn a feature off again, never on (see sharing.ts).
  z.object({
    op: z.literal("feature_leave"),
    feature: FeatureKindSchema,
    scope: FeatureLeaveScopeSchema.optional(),
  }),
  // UNI-260: start / close pi chats (see chat.ts).
  ChatNewRequestSchema,
  ChatCloseRequestSchema,
  ChatConfigRequestSchema,
  ChatConfigSetRequestSchema,
  // UNI-232: local-only admin ops (see admin.ts).
  HostSettingsGetRequestSchema,
  HostSettingsSetRequestSchema,
  HostInfoRequestSchema,
  PairBeginRequestSchema,
  PairStatusRequestSchema,
  DevicesListRequestSchema,
  DeviceRevokeRequestSchema,
  // UNI-317/UNI-318: scheduled cron jobs + their settings (see cron.ts).
  // A phone needs the `cron` sharing feature for every op below.
  z.object({ op: z.literal("cron_list") }),
  z.object({ op: z.literal("cron_show"), id: CronJobIdSchema }),
  z.object({ op: z.literal("cron_runs"), id: CronJobIdSchema.optional(), limit: CronRunsLimitSchema.optional() }),
  z.object({ op: z.literal("cron_log"), run_id: CronRunIdSchema }),
  CronAddRequestSchema.extend({ op: z.literal("cron_add") }),
  z.object({ op: z.literal("cron_edit"), id: CronJobIdSchema, patch: CronEditRequestSchema }),
  z.object({ op: z.literal("cron_rm"), id: CronJobIdSchema }),
  z.object({ op: z.literal("cron_enable"), id: CronJobIdSchema }),
  z.object({ op: z.literal("cron_disable"), id: CronJobIdSchema }),
  z.object({ op: z.literal("cron_run"), id: CronJobIdSchema }),
  z.object({ op: z.literal("cron_stop"), id: CronJobIdSchema }),
  z.object({ op: z.literal("cron_pause") }),
  z.object({ op: z.literal("cron_resume") }),
  z.object({ op: z.literal("cron_status") }),
  z.object({ op: z.literal("cron_parse"), schedule: z.string().optional(), at: z.string().optional(), tz: z.string().optional() }),
  z.object({ op: z.literal("cron_open_session"), run_id: CronRunIdSchema, target: ChatNewTargetSchema }),
  z.object({ op: z.literal("cron_settings_get") }),
  CronSettingsSetRequestSchema.extend({ op: z.literal("cron_settings_set") }),
]);

/// The flattened mux_action requests (serde `flatten` on the Rust
/// side): the action tag plus its payload sit next to `op`. Kept in
/// sync with MuxActionSchema above — zod v4 cannot map `extend` over
/// discriminated-union options without losing the literal markers.
export const MuxActionRequestSchema = z.union([
  z.object({
    op: z.literal("mux_action"),
    action: z.literal("workspace_create"),
    session: SessionNameSchema,
    cwd: z.string().min(1).optional(),
    label: SessionNameSchema.optional(),
  }),
  z.object({
    op: z.literal("mux_action"),
    action: z.literal("workspace_rename"),
    session: SessionNameSchema,
    workspace_id: HerdrIdSchema,
    label: SessionNameSchema,
  }),
  z.object({
    op: z.literal("mux_action"),
    action: z.literal("workspace_close"),
    session: SessionNameSchema,
    workspace_id: HerdrIdSchema,
  }),
  z.object({ op: z.literal("mux_action"), action: z.literal("tab_next"), session: SessionNameSchema }),
  z.object({ op: z.literal("mux_action"), action: z.literal("tab_prev"), session: SessionNameSchema }),
  z.object({ op: z.literal("mux_action"), action: z.literal("herdr_pane_next"), session: SessionNameSchema }),
  z.object({ op: z.literal("mux_action"), action: z.literal("herdr_pane_prev"), session: SessionNameSchema }),
  z.object({
    op: z.literal("mux_action"),
    action: z.literal("tmux_session_create"),
    name: SessionNameSchema,
    cwd: z.string().min(1).optional(),
  }),
  z.object({
    op: z.literal("mux_action"),
    action: z.literal("tmux_session_rename"),
    session: SessionNameSchema,
    name: SessionNameSchema,
  }),
  z.object({ op: z.literal("mux_action"), action: z.literal("tmux_session_kill"), session: SessionNameSchema }),
  z.object({ op: z.literal("mux_action"), action: z.literal("window_next"), session: SessionNameSchema }),
  z.object({ op: z.literal("mux_action"), action: z.literal("window_prev"), session: SessionNameSchema }),
  z.object({ op: z.literal("mux_action"), action: z.literal("tmux_pane_next"), session: SessionNameSchema }),
  z.object({ op: z.literal("mux_action"), action: z.literal("tmux_pane_prev"), session: SessionNameSchema }),
]);

export const RequestSchema = z.union([BaseRequestSchema, MuxActionRequestSchema]);

export type Request = z.infer<typeof RequestSchema>;

const TitleSchema = z
  .string()
  .max(512, "title must be at most 512 characters");

/**
 * The control (JSON, kind 0) message envelope for `unipi/app/1`.
 * Internally tagged on `type`, matching the Rust
 * `#[serde(tag = "type")]` enum.
 */
export const ControlSchema = z.discriminatedUnion("type", [
  z.object({
    type: z.literal("hello"),
    proto: SemverSchema,
    req: RequestSchema,
  }),
  z.object({
    type: z.literal("ok"),
    data: z.unknown().optional(),
  }),
  z.object({
    type: z.literal("err"),
    error: AppErrorSchema,
  }),
  z.object({
    type: z.literal("resize"),
    cols: DimensionSchema,
    rows: DimensionSchema,
  }),
  z.object({
    type: z.literal("exit"),
    code: z.number().int().nullable(),
  }),
  z.object({
    type: z.literal("title"),
    title: TitleSchema,
  }),
  z.object({
    type: z.literal("board_event"),
    event: z.string().optional(),
    data: z.string(),
  }),
  z.object({
    type: z.literal("chat"),
    line: z.string(),
  }),
  z.object({ type: z.literal("browser_cmd"), cmd: BrowserCmdSchema }),
  z.object({ type: z.literal("browser_event"), event: BrowserEventSchema }),
  z.object({ type: z.literal("desktop_cmd"), cmd: DesktopCmdSchema }),
  z.object({ type: z.literal("desktop_event"), event: DesktopEventSchema }),
]);

export type Control = z.infer<typeof ControlSchema>;
