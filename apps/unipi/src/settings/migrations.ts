/**
 * Settings migration registry. Every persisted settings blob carries a
 * `version` number. We migrate forward step by step until we reach
 * CURRENT_SETTINGS_VERSION, then let SettingsV1Schema validate/default
 * per-key. A version newer than what this build knows is handled
 * leniently: we try to parse it as-is and surface a notice instead of
 * failing (DESIGN.md §3.10, "a newer app never breaks on old data" /
 * forward compat).
 */
import { CURRENT_SETTINGS_VERSION, DEFAULT_TOOLBAR_KEYS, LEGACY_DEFAULT_TOOLBAR_KEYS } from './schema';

export type Migration = {
  /** The version this migration upgrades *from*. */
  from: number;
  /** The version this migration upgrades *to*. */
  to: number;
  /** Pure transform of the raw (untyped) settings blob. */
  migrate: (raw: Record<string, unknown>) => Record<string, unknown>;
};

/** v0 -> v1: `terminal.font` was renamed to `terminal.fontSize`. */
const v0ToV1: Migration = {
  from: 0,
  to: 1,
  migrate: (raw) => {
    const next: Record<string, unknown> = { ...raw, version: 1 };
    const terminal = raw.terminal;
    if (terminal && typeof terminal === 'object' && !Array.isArray(terminal)) {
      const t = terminal as Record<string, unknown>;
      if ('font' in t && !('fontSize' in t)) {
        const { font, ...rest } = t;
        next.terminal = { ...rest, fontSize: font };
      }
    }
    return next;
  },
};

/** v1 (in place): a toolbar equal to the old never-customised
 * default upgrades to the new Moshi-order default. Custom orders stay.
 * Value-based, so it runs for every persisted version. */
export function migrateToolbarDefault(raw: Record<string, unknown>): Record<string, unknown> {
  const input = raw.input;
  if (input && typeof input === 'object' && !Array.isArray(input)) {
    const i = input as Record<string, unknown>;
    if (Array.isArray(i.toolbar)) {
      const stored = i.toolbar.map(String);
      const isLegacy =
        stored.length === LEGACY_DEFAULT_TOOLBAR_KEYS.length &&
        LEGACY_DEFAULT_TOOLBAR_KEYS.every((key, index) => key === stored[index]);
      if (isLegacy) {
        i.toolbar = [...DEFAULT_TOOLBAR_KEYS];
      }
    }
  }
  return raw;
}

/** v1 (in place, UNI-127): `input.modifierIcons` is new. Blobs written
 * before it existed showed icon-only modifier keys; they now get the
 * explicit new default (text labels) instead of relying on implicit
 * schema defaulting, so the upgrade is visible and testable. */
export function migrateModifierIcons(raw: Record<string, unknown>): Record<string, unknown> {
  const input = raw.input;
  if (input && typeof input === 'object' && !Array.isArray(input)) {
    const i = input as Record<string, unknown>;
    if (typeof i.modifierIcons !== 'boolean') i.modifierIcons = false;
  }
  return raw;
}

export const MIGRATIONS: Migration[] = [v0ToV1];

export type MigrationResult = {
  /** The migrated (but not yet zod-validated) settings blob. */
  raw: Record<string, unknown>;
  /** True if the input version was newer than CURRENT_SETTINGS_VERSION. */
  newerVersion: boolean;
  /** True if any migration step ran. */
  migrated: boolean;
};

/**
 * Runs the migration chain on a raw settings blob. Unknown/missing
 * version is treated as version 0. Versions newer than what this build
 * supports are passed through unmigrated with `newerVersion: true`, so
 * the caller can show a lenient "this app is older than your settings"
 * notice instead of discarding data.
 */
export function migrate(input: unknown): MigrationResult {
  if (input == null || typeof input !== 'object' || Array.isArray(input)) {
    return { raw: {}, newerVersion: false, migrated: false };
  }

  let raw = input as Record<string, unknown>;
  const startVersion = typeof raw.version === 'number' ? raw.version : 0;

  if (startVersion > CURRENT_SETTINGS_VERSION) {
    return { raw, newerVersion: true, migrated: false };
  }

  let version = startVersion;
  let migrated = false;
  // Guard against an unexpectedly long/cyclic chain.
  for (let i = 0; i < MIGRATIONS.length + 1 && version < CURRENT_SETTINGS_VERSION; i++) {
    const step = MIGRATIONS.find((m) => m.from === version);
    if (!step) break;
    raw = step.migrate(raw);
    version = step.to;
    migrated = true;
  }

  return { raw, newerVersion: false, migrated };
}
