/**
 * The form engine's host seam (ui-protocol §7, WP-U11): read a namespace's
 * three layers (`effective`, `global`, `project`) over `host_cmd` and write one
 * value through `_oino/set_extension_setting` / `unset_extension_setting`.
 * Core stays authoritative — it validates and resolves; the app only mirrors
 * the rules for instant feedback.
 */
import { z } from 'zod';
import type { HostResult } from '../../lib/host';
import { LOCAL_HOST } from '../../lib/host';
import { hostCmd, setExtensionSetting } from '../../settings/hostCmd';
import type { Field, Json, Namespace } from './types';
import { parseNamespaces } from './types';
import { lookup, simplifyKey, type Diagnostic, type WriteLayer } from './semantics';

/** The stored layers of one namespace, as the app holds them. */
export type NamespaceLayers = {
  /** `SettingsNamespace::defaults` (the app resolves locally, so a scope switch needs no round trip). */
  defaults: Record<string, Json>;
  global: Record<string, Json>;
  project: Record<string, Json>;
  /** Values core rejected (§7.1) — shown inline on their rows. */
  diagnostics: Diagnostic[];
};

/** `_oino/get_settings_schema` reply: the namespaces are parsed tolerantly (§7). */
const SchemaReply = z.object({ namespaces: z.array(z.unknown()) });

/** Every settings namespace of the host, in registration order (core first). */
export function settingsSchema(hostId = LOCAL_HOST): Promise<HostResult<Namespace[]>> {
  return hostCmd({ kind: 'get_settings_schema' }, SchemaReply, hostId).then((reply) =>
    reply.ok ? { ok: true as const, value: parseNamespaces(reply.value.namespaces) } : reply,
  );
}

/** `_oino/get_extension_settings` reply at one layer (§7.1 values + diagnostics). */
const ValuesReply = z.object({ values: z.unknown(), diagnostics: z.array(z.unknown()).optional() });

export type SettingsLayerArg = 'effective' | 'global' | 'project';

/** One namespace's values at one layer (`effective` resolves every stored layer). */
export function readNamespaceValues(
  namespace: string,
  layer: SettingsLayerArg = 'effective',
  hostId = LOCAL_HOST,
): Promise<HostResult<{ values: unknown; diagnostics: Diagnostic[] }>> {
  return hostCmd({ kind: 'get_extension_settings', namespace, layer }, ValuesReply, hostId).then((reply) => {
    if (!reply.ok) return reply;
    const diagnostics = (reply.value.diagnostics ?? []).flatMap((entry) => {
      const map = typeof entry === 'object' && entry !== null ? (entry as Record<string, unknown>) : {};
      return typeof map.key === 'string' && typeof map.message === 'string'
        ? [{ namespace, key: map.key, message: map.message }]
        : [];
    });
    return { ok: true as const, value: { values: reply.value.values, diagnostics } };
  });
}

const objectOf = (value: unknown): Record<string, Json> =>
  typeof value === 'object' && value !== null && !Array.isArray(value) ? (value as Record<string, Json>) : {};

/** Reads the effective + global + project views of a namespace (three calls). */
export async function loadNamespaceLayers(namespace: Namespace, hostId = LOCAL_HOST): Promise<HostResult<NamespaceLayers>> {
  const [effective, global, project] = await Promise.all([
    readNamespaceValues(namespace.namespace, 'effective', hostId),
    readNamespaceValues(namespace.namespace, 'global', hostId),
    readNamespaceValues(namespace.namespace, 'project', hostId),
  ]);
  const failed = [effective, global, project].find((reply) => !reply.ok);
  if (failed && !failed.ok) return failed as HostResult<NamespaceLayers>;
  return {
    ok: true,
    value: {
      defaults: namespace.defaults,
      global: objectOf(global.ok ? global.value.values : undefined),
      project: objectOf(project.ok ? project.value.values : undefined),
      diagnostics: effective.ok ? effective.value.diagnostics : [],
    },
  };
}

/** The layer a field writes to at the current view (§7.2 `simplify`). */
export function writeLayerOf(field: Field, view: WriteLayer): WriteLayer {
  return field.scope === 'project' ? 'project' : field.scope === 'global' ? 'global' : view;
}

/**
 * The value actually stored for `value` at `layer` (§7.2): the layer below the
 * write is `undefined` for global and the global value for project, and a value
 * equal to it (or to the field default) is an **unset** (`undefined`).
 */
export function simplifyWrite(
  field: Field,
  value: Json | undefined,
  layer: WriteLayer,
  stored: { global: Record<string, Json>; project: Record<string, Json> },
): Json | undefined {
  if (value === undefined) return undefined;
  const below = layer === 'project' ? lookup(stored.global, field.key) : undefined;
  return simplifyKey(field, below, value);
}

/** Writes one already-simplified value at `layer` (`undefined` unsets the key). */
export function writeFieldValue(
  namespace: Namespace,
  field: Field,
  stored: Json | undefined,
  layer: WriteLayer,
  hostId = LOCAL_HOST,
): Promise<HostResult<null>> {
  return setExtensionSetting(namespace.namespace, field.key, stored, layer, hostId);
}
