/**
 * Hub-level host commands (app spec §3.14, WP-A24): the `_oino/*` settings /
 * auth / extension subset over `host_cmd`, plus the host's own `config.json`
 * (`host_settings_get/set`, host.md §9.9) — all without a session, through the
 * `lib/host` seam so the mock host stands in for captures and tests.
 */
import { z } from 'zod';
import { HostSettingsSchema, type HostSettings } from '@unipi/protocol';
import { hostClient, requestParsed } from '../lib/host';
import { markReservedGatesStale } from '../ui/slots/reserved';
import type { HostResult } from '../lib/host';
import { settingsHostId } from './target';

/** One row of the `extensions` answer (`ExtensionInfo`, camelCase like core's). */
export const ExtensionInfoSchema = z.object({
  id: z.string(),
  kind: z.string(),
  enabled: z.boolean(),
  loaded: z.boolean(),
  path: z.string().optional(),
  /** From the package's extension.json (the host adds them; older hosts don't). */
  name: z.string().optional(),
  version: z.string().optional(),
  description: z.string().optional(),
  /** User-installed (`<agent dir>/extensions`), so Settings may offer Remove; bundled
   *  packages are the installer's (older hosts omit it → not removable). */
  removable: z.boolean().optional(),
});
export type ExtensionInfo = z.infer<typeof ExtensionInfoSchema>;

/** One field of an OSS-2 settings namespace (what search indexes). */
export const NamespaceFieldSchema = z.object({
  key: z.string(),
  type: z.string().optional(),
  title: z.string().optional(),
  description: z.string().optional(),
});
export type NamespaceField = z.infer<typeof NamespaceFieldSchema>;

/** One extension settings namespace (`get_settings_schema` row). */
export const NamespaceSchema = z.object({
  namespace: z.string(),
  title: z.string().optional(),
  description: z.string().optional(),
  extensionId: z.string().optional(),
  fields: z.array(NamespaceFieldSchema).optional(),
});
export type Namespace = z.infer<typeof NamespaceSchema>;

/** The `host_cmd` commands A24 uses (§3.14; `login` is refused host-side). */
export type HostCmdOp =
  | { kind: 'get_settings'; key?: string }
  | { kind: 'set_setting'; key: string; value?: unknown; scope: 'global' | 'project' }
  | { kind: 'get_settings_schema' }
  /* `cwd`: the project whose services answer (its `.oino/settings.json` is the project
     layer); omitted = the host's primary cwd. Older hosts ignore it. */
  | { kind: 'get_extension_settings'; namespace: string; layer?: 'effective' | 'global' | 'project'; cwd?: string }
  | { kind: 'set_extension_setting'; namespace: string; key: string; value?: unknown; scope: 'global' | 'project'; cwd?: string }
  | { kind: 'unset_extension_setting'; namespace: string; key: string; scope: 'global' | 'project'; cwd?: string }
  | { kind: 'extensions' }
  | { kind: 'set_extension_enabled'; id: string; enabled: boolean }
  | { kind: 'install_extension'; path: string; replace?: boolean }
  | { kind: 'remove_extension'; id: string }
  | { kind: 'auth_providers'; validate?: boolean }
  | { kind: 'default_model' }
  | { kind: 'available_models' }
  | { kind: 'logout'; provider: string }
  | { kind: 'set_api_key'; provider: string; key: string }
  | { kind: 'get_shortcuts' };

/** `host_cmd{cmd}` on one host (default This computer), parsed with `schema`. */
export function hostCmd<T>(cmd: HostCmdOp, schema: z.ZodType<T>, hostId = settingsHostId()): Promise<HostResult<T>> {
  return requestParsed(hostClient(hostId), 'host_cmd', { cmd }, schema);
}

const ExtensionsSchema = z.object({ extensions: z.array(ExtensionInfoSchema) });
const NamespacesSchema = z.object({ namespaces: z.array(NamespaceSchema) });
const PiSettingsSchema = z.object({ settings: z.record(z.string(), z.unknown()).optional() });
const PiValueSchema = z.object({ value: z.unknown().optional() });
const ValuesSchema = z.object({ values: z.unknown(), diagnostics: z.array(z.unknown()).optional() });

/** `_oino/extensions`: loaded + disabled extensions of the host. */
export function listExtensions(hostId = settingsHostId()): Promise<HostResult<ExtensionInfo[]>> {
  return hostCmd({ kind: 'extensions' }, ExtensionsSchema, hostId).then((r) =>
    r.ok ? { ok: true as const, value: r.value.extensions } : r,
  );
}

/**
 * `_oino/set_extension_enabled`: the toggle (round-trips through `extensions`).
 * A reserved surface follows its owner live (ui-protocol §8 rule 2, WP-U12):
 * the registry gate refetches, so the nav item/tab appears or disappears at once.
 */
export function setExtensionEnabled(id: string, enabled: boolean, hostId = settingsHostId()): Promise<HostResult<null>> {
  return hostCmd({ kind: 'set_extension_enabled', id, enabled }, z.null(), hostId).then((result) => {
    if (result.ok) markReservedGatesStale();
    return result;
  });
}

/** `install_extension` reply: installed, or a same-id package needs a confirm first. */
export const InstallReplySchema = z.object({
  installed: z.boolean(),
  confirm: z
    .object({ id: z.string(), name: z.string(), version: z.string(), installedVersion: z.string().nullish() })
    .optional(),
  id: z.string(),
  name: z.string(),
  version: z.string(),
  path: z.string().optional(),
  /** The replaced user package's version ('' when unreadable). */
  replaced: z.string().optional(),
  warnings: z.array(z.string()).default([]),
  settingsNamespaces: z.array(z.string()).default([]),
  /** The running host loads it only after "Restart agent host". */
  needsRestart: z.boolean().default(false),
});
export type InstallReply = z.infer<typeof InstallReplySchema>;

export const RemoveReplySchema = z.object({
  id: z.string(),
  path: z.string(),
  bundledRemains: z.boolean().default(false),
  needsRestart: z.boolean().default(false),
});
export type RemoveReply = z.infer<typeof RemoveReplySchema>;

/** `install_extension`: install the package at `path` **on the host** (folder or .zip). */
export function installExtension(path: string, replace = false, hostId = settingsHostId()): Promise<HostResult<InstallReply>> {
  const cmd: HostCmdOp = replace ? { kind: 'install_extension', path, replace: true } : { kind: 'install_extension', path };
  return hostCmd(cmd, InstallReplySchema, hostId);
}

/** `remove_extension`: delete a user-installed package (bundled ones are refused). */
export function removeExtension(id: string, hostId = settingsHostId()): Promise<HostResult<RemoveReply>> {
  return hostCmd({ kind: 'remove_extension', id }, RemoveReplySchema, hostId);
}

/** `_oino/get_settings_schema`: every extension settings namespace. */
export function settingsSchema(hostId = settingsHostId()): Promise<HostResult<Namespace[]>> {
  return hostCmd({ kind: 'get_settings_schema' }, NamespacesSchema, hostId).then((r) =>
    r.ok ? { ok: true as const, value: r.value.namespaces } : r,
  );
}

/** `_oino/get_extension_settings`: one namespace's resolved values. */
export function extensionSettings(namespace: string, hostId = settingsHostId()): Promise<HostResult<unknown>> {
  return hostCmd({ kind: 'get_extension_settings', namespace }, ValuesSchema, hostId).then((r) =>
    r.ok ? { ok: true as const, value: r.value.values } : r,
  );
}

/** `_oino/set_extension_setting` / `unset_extension_setting`. */
export function setExtensionSetting(
  namespace: string,
  key: string,
  value: unknown,
  scope: 'global' | 'project' = 'global',
  hostId = settingsHostId(),
): Promise<HostResult<null>> {
  const cmd: HostCmdOp =
    value === undefined
      ? { kind: 'unset_extension_setting', namespace, key, scope }
      : { kind: 'set_extension_setting', namespace, key, value, scope };
  return hostCmd(cmd, z.null(), hostId);
}

/** `_oino/get_settings`: the whole merged pi settings object. */
export function piSettings(hostId = settingsHostId()): Promise<HostResult<Record<string, unknown>>> {
  return hostCmd({ kind: 'get_settings' }, PiSettingsSchema, hostId).then((r) =>
    r.ok ? { ok: true as const, value: r.value.settings ?? {} } : r,
  );
}

/** `_oino/set_setting`: write (or with `undefined`, delete) one pi settings key. */
export function setPiSetting(key: string, value: unknown, scope: 'global' | 'project' = 'global', hostId = settingsHostId()): Promise<HostResult<null>> {
  const cmd: HostCmdOp = value === undefined ? { kind: 'set_setting', key, scope } : { kind: 'set_setting', key, value, scope };
  return hostCmd(cmd, z.null(), hostId);
}

// ── host settings (`config.json`, host.md §9.9) ─────────────────────────────

/** `host_settings_get`: the whole object, secrets masked. */
export function hostSettingsGet(hostId = settingsHostId()): Promise<HostResult<HostSettings>> {
  return requestParsed(hostClient(hostId), 'host_settings_get', undefined, HostSettingsSchema);
}

/** `host_settings_set{key, value}` → the updated object. Local host only. */
export function hostSettingsSet(key: string, value: unknown, hostId = settingsHostId()): Promise<HostResult<HostSettings>> {
  return requestParsed(hostClient(hostId), 'host_settings_set', { key, value }, HostSettingsSchema);
}
