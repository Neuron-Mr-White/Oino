import { z } from "zod";
import { FsPathSchema } from "./fs.js";

/**
 * Start and close pi chats from the app (UNI-260, proto 1.12.0). Mirrors
 * `crates/proto/src/chat.rs`; docs/m5/PROTOCOL.md §10.
 *
 * - `chat_new{target, cwd?, outside?}` → `ok{data: ChatNewData}`: the host
 *   opens a herdr tab (existing or new workspace) or a tmux window/session in
 *   `cwd`, runs the PC's pi launcher there and waits for its bridge.
 * - `chat_close{pid, close_tab}` → `ok{data: ChatCloseData}`: stops that pi
 *   gracefully, then (close_tab) closes its herdr tab / tmux window.
 * - `chat_config` / `chat_config_set{launcher}` → `ok{data: ChatConfigData}`.
 *
 * Older hosts answer `E_SCHEMA` (unknown variant).
 */

export const CHAT_LAUNCHER_MAX = 256;
export const CHAT_LAUNCHER_DEFAULT = "pi";

const Name = z.string().min(1).max(128);
const HerdrId = z
  .string()
  .min(1)
  .max(128)
  .refine((id) => !/[\s\u0000-\u001f\u007f]/.test(id), "herdr id must not contain whitespace or control characters");

export const ChatLauncherSchema = z
  .string()
  .max(CHAT_LAUNCHER_MAX)
  .refine((v) => !/[\u0000-\u001f\u007f]/.test(v), "launcher must be one line without control characters");

export const ChatNewTargetSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("herdr_workspace"), session: Name, workspace_id: HerdrId }),
  z.object({ kind: z.literal("herdr_new_workspace"), session: Name, label: Name.optional() }),
  z.object({ kind: z.literal("tmux_window"), session: Name }),
  z.object({ kind: z.literal("tmux_new_session"), name: Name }),
]);
export type ChatNewTarget = z.infer<typeof ChatNewTargetSchema>;

export const ChatNewRequestSchema = z.object({
  op: z.literal("chat_new"),
  target: ChatNewTargetSchema,
  cwd: FsPathSchema.optional(),
  outside: z.boolean().optional(),
});
export const ChatCloseRequestSchema = z.object({
  op: z.literal("chat_close"),
  pid: z.number().int().min(1).max(4294967295),
  close_tab: z.boolean().optional(),
});
export const ChatConfigRequestSchema = z.object({ op: z.literal("chat_config") });
export const ChatConfigSetRequestSchema = z.object({ op: z.literal("chat_config_set"), launcher: ChatLauncherSchema });

export const ChatNewDataSchema = z.object({
  pid: z.number().int().min(1),
  mux: z.enum(["herdr", "tmux"]),
  session: z.string(),
  workspace_id: z.string().optional(),
  tab_id: z.string().optional(),
  pane_id: z.string(),
  cwd: z.string().optional(),
});
export type ChatNewData = z.infer<typeof ChatNewDataSchema>;

export const ChatCloseDataSchema = z.object({
  stopped: z.boolean(),
  closed: z.enum(["none", "pane", "tab", "window"]),
});
export type ChatCloseData = z.infer<typeof ChatCloseDataSchema>;

export const ChatConfigDataSchema = z.object({
  launcher: z.string().min(1),
  default: z.boolean().default(false),
  herdr: z.boolean().default(false),
  tmux: z.boolean().default(false),
});
export type ChatConfigData = z.infer<typeof ChatConfigDataSchema>;
