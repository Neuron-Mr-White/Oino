import { z } from "zod";

/**
 * v2 (host.md §9.9): the host's own settings — `<host dir>/config.json` — as
 * one Settings screen reads them.
 *
 * - `host_settings_get` → `HostSettingsData`: the whole object, secret-looking
 *   values masked (`"••••••"`). Any paired device may read it.
 * - `host_settings_set{key, value}` → the masked object. Only this computer
 *   may write (`E_UNAUTHORIZED` otherwise); `key` is a dotted path
 *   (`term.max`, `forwarding.enabled`, `cron.concurrency`, …), an unknown key
 *   or a wrong-typed value is `E_SCHEMA` and changes nothing.
 *
 * App settings (`closeAction`, autostart, UI prefs) live in the app's own
 * config dir; pi agent settings and extension settings are core's
 * (`_oino/get_settings` etc. over a session stream). This module is only the
 * middle layer.
 */

/** Mask the host puts in place of a secret-looking value. */
export const HOST_SETTINGS_SECRET_MASK = "••••••";

/** Exact dotted keys `host_settings_set` accepts (blocks and their leaves). */
export const HOST_SETTINGS_KEYS = [
  "relay",
  "relay_only",
  "forwarding",
  "forwarding.enabled",
  "forwarding.ports",
  "forwarding.devices",
  "push",
  "push.enabled",
  "push.devices",
  "fs",
  "fs.allow",
  "fs.deny",
  "term",
  "term.max",
  "update",
  "update.mode",
  "browser",
  "sharing",
] as const;

/**
 * Blocks owned by other modules accept any leaf under their prefix: their own
 * loaders are the schema (`cron` is a settings object of its own, so its bare
 * name is accepted too).
 */
const OPEN_BLOCK_KEYS = ["browser", "sharing", "cron"] as const;

const OPEN_BLOCK = /^(browser|sharing|cron)\.[^.]/;

/** A dotted host settings key; unknown keys are a schema error. */
export const HostSettingsKeySchema = z
  .string()
  .refine(
    (key) =>
      (HOST_SETTINGS_KEYS as readonly string[]).includes(key) ||
      key === "cron" ||
      (OPEN_BLOCK.test(key) && !key.endsWith(".")),
    { message: "unknown host settings key" },
  );

/** `relay`: the public N0 relays (`"n0"`) or a self-hosted https relay URL. */
export const RelaySettingSchema = z
  .string()
  .refine((value) => value === "n0" || /^https:\/\/[^/]/.test(value), {
    message: 'relay must be "n0" or an https URL',
  });

export const HostSettingsGetRequestSchema = z.object({ op: z.literal("host_settings_get") });

export const HostSettingsSetRequestSchema = z.object({
  op: z.literal("host_settings_set"),
  key: HostSettingsKeySchema,
  value: z.unknown(),
});

/** `forwarding`: off by default; `ports` is `"any"` or an explicit list. */
export const ForwardingSettingsSchema = z
  .object({
    enabled: z.boolean().optional(),
    ports: z
      .union([z.literal("any"), z.array(z.number().int().min(1).max(65535))])
      .optional(),
    devices: z.array(z.string()).optional(),
  })
  .passthrough();

export const PushSettingsSchema = z
  .object({ enabled: z.boolean().optional(), devices: z.array(z.string()).optional() })
  .passthrough();

export const FsSettingsSchema = z
  .object({ allow: z.array(z.string()).optional(), deny: z.array(z.string()).optional() })
  .passthrough();

export const TermSettingsSchema = z
  .object({ max: z.number().int().min(1).max(128).optional() })
  .passthrough();

export const UpdateSettingsSchema = z
  .object({ mode: z.enum(["prompt", "auto"]).optional() })
  .passthrough();

/** The `browser` / `sharing` / `cron` blocks are host-owned: pass them whole. */
const OpenBlockSchema = z.record(z.string(), z.unknown());

/** `host_settings_get` / `host_settings_set` data: the whole `config.json`. */
export const HostSettingsSchema = z
  .object({
    relay: RelaySettingSchema.optional(),
    relay_only: z.boolean().optional(),
    forwarding: ForwardingSettingsSchema.optional(),
    push: PushSettingsSchema.optional(),
    fs: FsSettingsSchema.optional(),
    browser: OpenBlockSchema.optional(),
    sharing: OpenBlockSchema.optional(),
    cron: OpenBlockSchema.optional(),
    term: TermSettingsSchema.optional(),
    update: UpdateSettingsSchema.optional(),
  })
  .passthrough();

export type HostSettings = z.infer<typeof HostSettingsSchema>;
export type ForwardingSettings = z.infer<typeof ForwardingSettingsSchema>;
export type UpdateSettings = z.infer<typeof UpdateSettingsSchema>;
