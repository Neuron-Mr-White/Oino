import { z } from 'zod';

/**
 * SettingsV1 — the one typed settings schema for the app (DESIGN.md §3.10,
 * §5.5). Every key has a default, so a missing or invalid key can always
 * fall back without crashing the app.
 */

export const TOOLBAR_KEYS = [
  'ctrl',
  'shift',
  'esc',
  'tab',
  'opt',
  'arrows',
  'ctrl_c',
  'paste',
  'shortcuts',
  'history',
  'composer',
  'keyboard',
  'enter',
  'backspace',
  'home',
  'end',
  'pgup',
  'pgdn',
] as const;
export type ToolbarKey = (typeof TOOLBAR_KEYS)[number];
export const ToolbarKeySchema = z.enum(TOOLBAR_KEYS);

/** Default toolbar order (Moshi): ctrl, shift, esc, tab, opt, arrows,
 * shortcuts, history. Composer/keyboard are pinned in the UI and are
 * not part of the scroll list. */
export const DEFAULT_TOOLBAR_KEYS: ToolbarKey[] = ['ctrl', 'shift', 'esc', 'tab', 'opt', 'arrows', 'shortcuts', 'history'];

/** The pre-H3b default list: stored blobs equal to this were never
 * customised, so they upgrade to the new default. */
export const LEGACY_DEFAULT_TOOLBAR_KEYS: ToolbarKey[] = ['ctrl', 'esc', 'tab', 'arrows', 'ctrl_c', 'paste', 'shortcuts', 'composer', 'keyboard'];

const ToolbarKeysSchema = z
  .array(ToolbarKeySchema)
  .min(1)
  .refine((keys) => keys.includes('ctrl'), {
    message: 'toolbar keys must include ctrl',
  })
  .refine((keys) => new Set(keys).size === keys.length, {
    message: 'toolbar keys must be unique',
  })
  .prefault(DEFAULT_TOOLBAR_KEYS);

export const TerminalSettingsSchema = z.object({
  fontSize: z.number().int().min(8).max(32).default(14),
  lineHeight: z.number().min(1).max(2).default(1.2),
  cursorStyle: z.enum(['block', 'underline', 'bar']).default('block'),
  cursorBlink: z.boolean().default(true),
  scrollback: z.number().int().min(0).max(100000).default(5000),
  sizePolicy: z.enum(['fit_phone', 'keep_pc']).default('fit_phone'),
  keepScreenOn: z.boolean().default(false),
  reopenLast: z.boolean().default(true),
});
export type TerminalSettings = z.infer<typeof TerminalSettingsSchema>;

export const InputSettingsSchema = z.object({
  haptics: z.boolean().default(true),
  composerAutocorrect: z.boolean().default(true),
  /** Toolbar modifier keys as icons (^ ⇧ ↺ ⇥ ⌥) instead of text labels
   * (Ctrl Shift Esc Tab Opt). Off by default, like Moshi, so the
   * mapping is obvious (UNI-127). */
  modifierIcons: z.boolean().default(false),
  toolbar: ToolbarKeysSchema,
});
export type InputSettings = z.infer<typeof InputSettingsSchema>;

const RelaySchema = z.discriminatedUnion('mode', [
  z.object({ mode: z.literal('n0') }),
  z.object({ mode: z.literal('custom'), url: z.string().url().regex(/^https:\/\//, 'relay url must be https') }),
]);
export type Relay = z.infer<typeof RelaySchema>;
const DEFAULT_RELAY: Relay = { mode: 'n0' };

export const ConnectionSettingsSchema = z.object({
  relay: RelaySchema.prefault(DEFAULT_RELAY),
  pathPreference: z.enum(['auto', 'prefer_ssh']).default('auto'),
});
export type ConnectionSettings = z.infer<typeof ConnectionSettingsSchema>;

export const BoardSettingsSchema = z.object({
  defaultProject: z.string().nullable().default(null),
  showBacklog: z.boolean().default(false),
});
export type BoardSettings = z.infer<typeof BoardSettingsSchema>;

/** Editor screen (UNI-190): vim keybindings via @replit/codemirror-vim. */
export const EditorSettingsSchema = z.object({
  vim: z.boolean().default(false),
});
export type EditorSettings = z.infer<typeof EditorSettingsSchema>;

/** Phone notifications (UNI-161 §4b): the user's own ntfy server/topic/token,
 *  pasted or scanned, so the app can poll it ("Keep listening", no
 *  distributor) and/or register a UnifiedPush endpoint with it. Never
 *  logged — same rule as the host/pi side (notify's ntfy-config.ts). */
export const NotifySettingsSchema = z.object({
  enabled: z.boolean().default(false),
  serverUrl: z.string().default('https://ntfy.sh'),
  topic: z.string().default(''),
  token: z.string().default(''),
  /** In-app poll of the topic while open/resumed — the fallback when no
   *  UnifiedPush distributor is installed (no foreground service yet). */
  keepListening: z.boolean().default(false),
  /** UNI-161 §1: the per-pairing E2E key (base64, 32 raw bytes), fetched
   *  from the host via `notify_config` ("Get from this PC"). When set, a
   *  received ntfy message starting with `unipi1:` is decrypted before
   *  the drawer/native notification ever shows its content. Never logged. */
  key: z.string().default(''),
  /** The host id `key`/server/topic/token came from (`notify_config`),
   *  so re-pairing with a different host clears a stale key silently. */
  keySourceHostId: z.string().nullable().default(null),
});
export type NotifySettings = z.infer<typeof NotifySettingsSchema>;

/**
 * Every group schema of [`SettingsV1Schema`], in schema order — the ONE
 * list of settings groups, which both `load.ts` (validation) and
 * `context.tsx` (`update('group.key')`) derive their group handling from.
 *
 * UNI-286: this list used to be written out twice, and `notify` was missing
 * from the copy in load.ts, so the persisted notification settings were
 * dropped and re-defaulted on every app start. The type check below keeps it
 * honest: adding a group to `SettingsV1Schema` without a schema here (or
 * listing a name that isn't one) is a compile error.
 */
export const SETTINGS_GROUP_SCHEMAS = {
  terminal: TerminalSettingsSchema,
  input: InputSettingsSchema,
  connection: ConnectionSettingsSchema,
  board: BoardSettingsSchema,
  notify: NotifySettingsSchema,
  editor: EditorSettingsSchema,
} as const;
export type SettingsGroup = keyof typeof SETTINGS_GROUP_SCHEMAS;

type SettingsGroupNames = Exclude<keyof SettingsV1, 'version'>;
/// Compile-time: SETTINGS_GROUP_SCHEMAS is exactly the SettingsV1 groups.
type GroupCheck =
  SettingsGroupNames extends SettingsGroup
    ? SettingsGroup extends SettingsGroupNames
      ? Record<SettingsGroupNames, unknown>
      : { _stray_name_not_a_SettingsV1_group: SettingsGroup }
    : { _group_missing_from_SETTINGS_GROUP_SCHEMAS: SettingsGroupNames };
const group_check: GroupCheck = {};
void group_check;

export const SettingsV1Schema = z.object({
  version: z.literal(1).default(1),
  terminal: TerminalSettingsSchema.prefault({}),
  input: InputSettingsSchema.prefault({}),
  connection: ConnectionSettingsSchema.prefault({}),
  board: BoardSettingsSchema.prefault({}),
  notify: NotifySettingsSchema.prefault({}),
  editor: EditorSettingsSchema.prefault({}),
});
export type SettingsV1 = z.infer<typeof SettingsV1Schema>;

/** The fully-defaulted SettingsV1 object, used as the base for fallbacks. */
export const DEFAULT_SETTINGS: SettingsV1 = SettingsV1Schema.parse({});

/** Current schema version this build understands as "native" (no migration needed). */
export const CURRENT_SETTINGS_VERSION = 1 as const;
