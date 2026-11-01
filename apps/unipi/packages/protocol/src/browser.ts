import { z } from "zod";

/**
 * Remote desktop browser wire types (UNI-215, forwarding phase 3). Mirrors
 * `crates/proto/src/browser.rs`.
 *
 * - `browser_status` → `ok{data: BrowserStatusData}`.
 * - `browser_open{viewport, url?, target?}` → `ok{data: BrowserOpenData}`,
 *   then host → phone `browser_event{event}` controls (a `frame` event is
 *   followed by one kind-1 frame with the JPEG) and phone → host
 *   `browser_cmd{cmd}` controls.
 * - `browser_close` → `ok{}`.
 * - `browser_window{show}` (UNI-268, 1.13.0) → `ok{data: BrowserStatusData}`:
 *   show/hide the browser's window on the PC; a running browser relaunches
 *   with the same tabs and open viewers get `closed{reason:"relaunch"}`.
 * - UNI-292 (1.16.0), additive: `engine` ("cloak" | "chrome"), `stealth`,
 *   `stealth_reason` and `profile` on status/open data (absent from older
 *   hosts); `browser_open{profile?}` picks a browser profile (default
 *   "default"). `stealth: false` = plain Chrome: show "not stealthy".
 * - UNI-296 (1.17.0), additive: `agent: BrowserAgent` on status/open data and
 *   a `browser_event{kind:"agent"}` when it changes; `browser_takeover{on,
 *   profile?}` → `ok{data: BrowserAgent}` (while on, agents' input is
 *   refused); `browser_profiles` → `ok{data: BrowserProfilesData}`.
 * - UNI-299 (1.18.0), additive: CloakBrowser runs headed on a hidden,
 *   private display by default instead of the PC's real one. Status/open
 *   data carry `display: "hidden" | "window" | "headless"` (absent =
 *   older host, i.e. "window") and, only for "headless", `stealth_note`
 *   (no hidden display was possible: a milder red flag, `stealth` itself
 *   stays true). The existing `headless`/`browser_window` toggle now
 *   switches hidden ↔ window for CloakBrowser too.
 */

export const BROWSER_MAX_SIDE = 4096;
export const BROWSER_MAX_URL = 8192;
export const BROWSER_MAX_TEXT = 16 * 1024;

export const BrowserViewportSchema = z.object({
  width: z.number().int().min(50).max(BROWSER_MAX_SIDE),
  height: z.number().int().min(50).max(BROWSER_MAX_SIDE),
  scale: z.number().min(0.5).max(4),
  mobile: z.boolean().default(false),
});
export type BrowserViewport = z.infer<typeof BrowserViewportSchema>;

export const BrowserUrlSchema = z
  .string()
  .min(1)
  .max(BROWSER_MAX_URL)
  // eslint-disable-next-line no-control-regex
  .regex(/^[^\u0000-\u001f\u007f-\u009f]*$/, "url must not contain control characters");

/** UNI-292: a browser profile name, 1..=40 of a-z 0-9 - _. */
export const BROWSER_PROFILE_MAX = 40;
export const BrowserProfileSchema = z.string().min(1).max(BROWSER_PROFILE_MAX).regex(/^[a-z0-9_-]+$/);

/** UNI-292: which browser runs (absent = an older host, i.e. Chrome). */
export const BrowserEngineSchema = z.string();

export const BrowserTabSchema = z.object({ id: z.string(), url: z.string(), title: z.string() });
export type BrowserTab = z.infer<typeof BrowserTabSchema>;

export const BrowserInfoSchema = z.object({ name: z.string(), path: z.string(), version: z.string().optional() });
export type BrowserInfo = z.infer<typeof BrowserInfoSchema>;

/** UNI-296: what agents (CDP clients other than the app) do with one browser. */
export const BrowserAgentSchema = z.object({
  connected: z.boolean(),
  /** Input or navigation within the last few seconds. */
  driving: z.boolean(),
  /** The user took over: agents' input is refused. */
  taken_over: z.boolean().default(false),
});
export type BrowserAgent = z.infer<typeof BrowserAgentSchema>;

/** UNI-296: one browser profile (`browser_profiles`). */
export const BrowserProfileInfoSchema = z.object({
  name: z.string(),
  created: z.number().int().min(0),
  last_used: z.number().int().min(0).optional(),
  running: z.boolean(),
  engine: z.string().optional(),
  stealth: z.boolean().optional(),
  cookie_domains: z.array(z.string()).default([]),
});
export type BrowserProfileInfo = z.infer<typeof BrowserProfileInfoSchema>;

export const BrowserProfilesDataSchema = z.object({
  profiles: z.array(BrowserProfileInfoSchema),
  default: z.string(),
});
export type BrowserProfilesData = z.infer<typeof BrowserProfilesDataSchema>;

export const BrowserStatusDataSchema = z.object({
  enabled: z.boolean(),
  browser: BrowserInfoSchema.optional(),
  running: z.boolean(),
  headless: z.boolean(),
  idle_minutes: z.number().int().min(0),
  tabs: z.array(BrowserTabSchema).default([]),
  install_hint: z.string().optional(),
  /** UNI-268 (1.13.0): the PC can show the window (`browser_window`). Absent = older host. */
  can_show_window: z.boolean().optional(),
  /** UNI-292 (1.16.0): "cloak" | "chrome". */
  engine: BrowserEngineSchema.optional(),
  /** UNI-292: false = plain Chrome, not stealthy (configured, or CloakBrowser failed). */
  stealth: z.boolean().optional(),
  stealth_reason: z.string().optional(),
  /** UNI-299: set only for `display: "headless"`. */
  stealth_note: z.string().optional(),
  profile: z.string().optional(),
  /** UNI-296 (1.17.0): agents on that browser. Absent = older host. */
  agent: BrowserAgentSchema.optional(),
  /** UNI-299 (1.18.0): "hidden" | "window" | "headless". Absent = older host ("window"). */
  display: z.string().optional(),
});
export type BrowserStatusData = z.infer<typeof BrowserStatusDataSchema>;

export const BrowserOpenDataSchema = z.object({
  browser: BrowserInfoSchema,
  headless: z.boolean(),
  target: z.string(),
  tabs: z.array(BrowserTabSchema),
  engine: BrowserEngineSchema.optional(),
  stealth: z.boolean().optional(),
  stealth_reason: z.string().optional(),
  /** UNI-299: as in {@link BrowserStatusDataSchema}. */
  stealth_note: z.string().optional(),
  profile: z.string().optional(),
  agent: BrowserAgentSchema.optional(),
  /** UNI-299 (1.18.0): as in {@link BrowserStatusDataSchema}. */
  display: z.string().optional(),
});
export type BrowserOpenData = z.infer<typeof BrowserOpenDataSchema>;

const finite = z.number().finite();
const MouseButtonSchema = z.enum(["none", "left", "middle", "right"]);

export const BrowserCmdSchema = z.discriminatedUnion("op", [
  z.object({ op: z.literal("navigate"), url: BrowserUrlSchema }),
  z.object({ op: z.literal("back") }),
  z.object({ op: z.literal("forward") }),
  z.object({ op: z.literal("reload") }),
  z.object({ op: z.literal("stop") }),
  z.object({ op: z.literal("tab_new"), url: BrowserUrlSchema.optional() }),
  z.object({ op: z.literal("tab_close"), id: z.string() }),
  z.object({ op: z.literal("tab_activate"), id: z.string() }),
  z.object({ op: z.literal("viewport"), viewport: BrowserViewportSchema }),
  z.object({
    op: z.literal("mouse"),
    kind: z.enum(["pressed", "released", "moved", "wheel"]),
    x: finite,
    y: finite,
    button: MouseButtonSchema.default("none"),
    clicks: z.number().int().min(0).default(0),
    dx: finite.default(0),
    dy: finite.default(0),
    modifiers: z.number().int().min(0).default(0),
  }),
  z.object({
    op: z.literal("touch"),
    kind: z.enum(["start", "move", "end", "cancel"]),
    points: z.array(z.object({ x: finite, y: finite, id: z.number().int().min(0).default(0) })).max(10).default([]),
  }),
  z.object({ op: z.literal("pinch"), x: finite, y: finite, scale: z.number().gt(0).max(10) }),
  z.object({
    op: z.literal("key"),
    kind: z.enum(["down", "up", "press"]),
    key: z.string().min(1).max(64),
    code: z.string().optional(),
    text: z.string().optional(),
    modifiers: z.number().int().min(0).default(0),
  }),
  z.object({ op: z.literal("text"), text: z.string().max(BROWSER_MAX_TEXT) }),
  z.object({ op: z.literal("ack"), seq: z.number().int().min(0) }),
  // WP-B8: element picker — what is under (x, y)? (`pick`: the click, details wanted).
  z.object({ op: z.literal("inspect"), x: finite, y: finite, pick: z.boolean().default(false) }),
]);
export type BrowserCmd = z.input<typeof BrowserCmdSchema>;

export const BrowserFrameMetaSchema = z.object({
  seq: z.number().int().min(0),
  css_width: z.number(),
  css_height: z.number(),
  quality: z.number().int().min(1).max(100),
});

/** WP-B8: the picked element (ZCode web-element context field set). */
export const BrowserElementDetailSchema = z.object({
  url: z.string(),
  title: z.string(),
  tag: z.string(),
  role: z.string().nullish(),
  name: z.string().nullish(),
  selector: z.string().nullish(),
  xpath: z.string().nullish(),
  text: z.string().nullish(),
  attributes: z.record(z.string(), z.string()).default({}),
  html: z.string().nullish(),
});
export type BrowserElementDetail = z.infer<typeof BrowserElementDetailSchema>;

/** WP-B8: an element under the picker, in the frame's CSS px. */
export const BrowserElementSchema = z.object({
  x: z.number(),
  y: z.number(),
  width: z.number(),
  height: z.number(),
  label: z.string(),
  detail: BrowserElementDetailSchema.optional(),
});
export type BrowserElement = z.infer<typeof BrowserElementSchema>;

export const BrowserEventSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("tabs"), tabs: z.array(BrowserTabSchema), active: z.string() }),
  z.object({
    kind: z.literal("page"),
    url: z.string(),
    title: z.string(),
    loading: z.boolean(),
    can_back: z.boolean(),
    can_forward: z.boolean(),
  }),
  BrowserFrameMetaSchema.extend({ kind: z.literal("frame") }),
  z.object({ kind: z.literal("crashed"), message: z.string() }),
  z.object({ kind: z.literal("closed"), reason: z.string() }),
  // UNI-296 (1.17.0)
  z.object({ kind: z.literal("agent"), agent: BrowserAgentSchema }),
  // WP-B8 element picker answer (`element` absent = nothing there).
  z.object({ kind: z.literal("element"), pick: z.boolean(), element: BrowserElementSchema.optional() }),
]);
export type BrowserEvent = z.infer<typeof BrowserEventSchema>;
