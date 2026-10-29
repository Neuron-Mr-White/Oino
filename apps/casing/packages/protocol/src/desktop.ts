import { z } from "zod";

/**
 * Desktop streaming with control (UNI-216, forwarding phase 4). Mirrors
 * `crates/proto/src/desktop.rs`.
 *
 * - `desktop_status` → `ok{data: DesktopStatusData}`.
 * - `desktop_open{viewport, quality?, view_only?}` → `ok{}`, then host → phone
 *   `desktop_event{event}` controls (a `frame` event is followed by kind-1
 *   frames carrying `bytes` of H.264 Annex-B) and phone → host
 *   `desktop_cmd{cmd}` controls. A failure after `ok` is one `err`.
 */

export const DESKTOP_MAX_TEXT = 4096;
export const DESKTOP_MAX_SIDE = 8192;

/**
 * `best` / `balanced` / `speed` (UNI-271, proto 1.13): RustDesk-style presets,
 * only sent to hosts whose `desktop_status.features` has `presets`.
 */
export const DesktopQualitySchema = z.enum(["auto", "low", "medium", "high", "best", "balanced", "speed"]);
export type DesktopQuality = z.infer<typeof DesktopQualitySchema>;

export const DesktopViewportSchema = z.object({
  width: z.number().int().min(16).max(DESKTOP_MAX_SIDE),
  height: z.number().int().min(16).max(DESKTOP_MAX_SIDE),
});
export type DesktopViewport = z.infer<typeof DesktopViewportSchema>;

/** `macos` / `windows` since proto 1.12 (non-Linux hosts). */
export const DisplayKindSchema = z.enum(["wayland", "x11", "macos", "windows", "none"]);
export const DesktopConsentSchema = z.enum(["granted", "needed", "not_needed"]);

export const DESKTOP_FEATURE_PRESETS = "presets";
export const DESKTOP_FEATURE_DISPLAYS = "displays";

/** One monitor the host can stream (UNI-271). */
export const DesktopDisplaySchema = z.object({
  id: z.string(),
  name: z.string(),
  width: z.number().int().min(1),
  height: z.number().int().min(1),
  primary: z.boolean().default(false),
});
export type DesktopDisplay = z.infer<typeof DesktopDisplaySchema>;

export const DesktopStatusDataSchema = z.object({
  enabled: z.boolean(),
  view_only: z.boolean(),
  compiled: z.array(z.string()),
  display: DisplayKindSchema,
  backend: z.string().optional(),
  consent: DesktopConsentSchema,
  reason: z.string().optional(),
  hint: z.string().optional(),
  /** UNI-271: optional abilities (`presets`, `displays`); absent on older hosts. */
  features: z.array(z.string()).optional(),
  displays: z.array(DesktopDisplaySchema).optional(),
});
export type DesktopStatusData = z.infer<typeof DesktopStatusDataSchema>;

const unit = z.number().finite().min(0).max(1);
const steps = z.number().finite().min(-100).max(100);

export const DesktopCmdSchema = z.discriminatedUnion("op", [
  z.object({ op: z.literal("pointer"), x: unit, y: unit }),
  z.object({ op: z.literal("button"), button: z.enum(["left", "middle", "right"]), down: z.boolean() }),
  z.object({ op: z.literal("scroll"), dx: steps.default(0), dy: steps.default(0) }),
  z.object({ op: z.literal("key"), keysym: z.number().int().min(1).max(0x1ffffff), down: z.boolean() }),
  z.object({ op: z.literal("text"), text: z.string().max(DESKTOP_MAX_TEXT) }),
  z.object({ op: z.literal("keyframe") }),
  z.object({ op: z.literal("quality"), quality: DesktopQualitySchema }),
  z.object({ op: z.literal("viewport"), viewport: DesktopViewportSchema }),
  z.object({ op: z.literal("ack"), seq: z.number().int().min(0) }),
]);
export type DesktopCmd = z.input<typeof DesktopCmdSchema>;

export const DesktopFrameMetaSchema = z.object({
  seq: z.number().int().min(0),
  key: z.boolean(),
  ts_ms: z.number().int().min(0),
  bytes: z.number().int().min(0),
});

export const DesktopEventSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("consent"), state: z.string() }),
  z.object({
    kind: z.literal("started"),
    backend: z.string(),
    width: z.number().int().min(1),
    height: z.number().int().min(1),
    view_only: z.boolean(),
    display: z.string().optional(),
  }),
  z.object({ kind: z.literal("config"), codec: z.string(), width: z.number().int().min(1), height: z.number().int().min(1) }),
  DesktopFrameMetaSchema.extend({ kind: z.literal("frame") }),
  z.object({ kind: z.literal("stats"), fps: z.number(), kbps: z.number().int().min(0), quality: DesktopQualitySchema, rtt_ms: z.number().int().min(0) }),
  z.object({ kind: z.literal("closed"), reason: z.string() }),
]);
export type DesktopEvent = z.infer<typeof DesktopEventSchema>;
