/**
 * loadSettings: read raw settings through an adapter, migrate, and
 * validate with per-key fallback to defaults. A missing or invalid key
 * never crashes the app or discards sibling keys (DESIGN.md §3.10).
 */
import { z } from 'zod';
import {
  BoardSettingsSchema,
  ConnectionSettingsSchema,
  DEFAULT_SETTINGS,
  EditorSettingsSchema,
  SETTINGS_GROUP_SCHEMAS,
  SettingsV1Schema,
  type SettingsV1,
  type SettingsGroup,
} from './schema';
import { migrate, migrateModifierIcons, migrateToolbarDefault } from './migrations';

function ensureObject(raw: unknown): Record<string, unknown> {
  return raw != null && typeof raw === 'object' && !Array.isArray(raw) ? (raw as Record<string, unknown>) : {};
}
import type { SettingsAdapter } from './adapter';

export type LoadSettingsResult = {
  settings: SettingsV1;
  /** Dotted-path keys that were missing or invalid and got reset to their default. */
  resetKeys: string[];
  /** True if the persisted settings were migrated from an older version. */
  migrated: boolean;
  /** True if the persisted settings were newer than this build understands. */
  newerVersion: boolean;
  notice?: 'newer_version';
};

type GroupName = SettingsGroup;
const GROUP_NAMES = Object.keys(SETTINGS_GROUP_SCHEMAS) as GroupName[];

/**
 * Validates one top-level group against its schema. On failure, falls
 * back key-by-key within the group so one bad field doesn't reset its
 * siblings. Returns the validated group value plus the list of dotted
 * keys that were reset.
 */
function validateGroup<G extends GroupName>(
  group: G,
  rawValue: unknown,
): { value: SettingsV1[G]; resetKeys: string[] } {
  const schema = SETTINGS_GROUP_SCHEMAS[group];

  // Group itself is invalid (not an object, or multiple bad fields): fall
  // back key by key against the group default.
  const defaultGroup = DEFAULT_SETTINGS[group] as Record<string, unknown>;
  const rawObj: Record<string, unknown> =
    rawValue != null && typeof rawValue === 'object' && !Array.isArray(rawValue)
      ? (rawValue as Record<string, unknown>)
      : {};

  const resetKeys: string[] = [];
  const result: Record<string, unknown> = {};
  for (const key of Object.keys(defaultGroup)) {
    if (!(key in rawObj)) {
      result[key] = defaultGroup[key];
      continue;
    }
    // Validate this single key by merging it onto the default group and
    // parsing through the group schema, then checking the field came out
    // equal to what we fed in (i.e. it was accepted, not coerced away).
    const candidate = { ...defaultGroup, [key]: rawObj[key] };
    const singleCheck = schema.safeParse(candidate);
    if (singleCheck.success) {
      result[key] = (singleCheck.data as Record<string, unknown>)[key];
    } else {
      result[key] = defaultGroup[key];
      resetKeys.push(`${group}.${key}`);
    }
  }

  // Re-parse the assembled object to apply any cross-field defaults.
  const final = schema.parse(result);
  return { value: final as SettingsV1[G], resetKeys };
}

/**
 * Validates a raw (already migrated) settings blob against SettingsV1,
 * with per-key fallback. Returns the fully defaulted settings plus the
 * dotted-path keys that were reset.
 */
export function validateSettings(raw: unknown): { settings: SettingsV1; resetKeys: string[] } {
  const obj: Record<string, unknown> =
    raw != null && typeof raw === 'object' && !Array.isArray(raw) ? (raw as Record<string, unknown>) : {};

  const resetKeys: string[] = [];
  const groups = {} as { [K in GroupName]: SettingsV1[K] };
  for (const group of GROUP_NAMES) {
    const { value, resetKeys: groupResets } = validateGroup(group, obj[group]);
    (groups as Record<GroupName, unknown>)[group] = value;
    resetKeys.push(...groupResets);
  }

  if ('version' in obj && !z.literal(1).safeParse(obj.version).success) {
    resetKeys.push('version');
  }

  const settings = SettingsV1Schema.parse({ version: 1, ...groups });
  return { settings, resetKeys };
}

export function loadSettings(rawPersisted: unknown): LoadSettingsResult {
  // The toolbar-default upgrade only applies to real stored blobs, so
  // a missing blob still reads as "not migrated".
  const stored =
    rawPersisted != null && typeof rawPersisted === 'object' && !Array.isArray(rawPersisted)
      ? migrateModifierIcons(migrateToolbarDefault(rawPersisted as Record<string, unknown>))
      : rawPersisted;
  const { raw, migrated, newerVersion } = migrate(stored);
  const { settings, resetKeys } = validateSettings(raw);
  return { settings, resetKeys, migrated, newerVersion, ...(newerVersion ? { notice: 'newer_version' as const } : {}) };
}

export async function loadSettingsFromAdapter(adapter: SettingsAdapter): Promise<LoadSettingsResult> {
  return loadSettings(await adapter.read());
}
