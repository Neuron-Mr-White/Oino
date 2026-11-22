/**
 * OSS-2 semantics (ui-protocol §7.1–7.4, WP-U11): the TS port of
 * `oino_ui::settings` — layered `resolve`, `simplify`, validation and
 * `visibleIf` / `disabledIf`. The app runs the same rules as core and the TUI,
 * so a namespace resolves identically everywhere; every write is still
 * validated by core (the app never trusts its own result over the reply).
 */
import { z } from 'zod';
import type { Cond, Field, FieldScope, Json, Namespace, Section } from './types';
import { allFields, fieldScope, storesValue } from './types';

export type Diagnostic = { namespace: string; key: string; message: string };

/** The stored layers of one namespace (§7 `resolve(stored, scope)`). */
export type Layers = {
  /** `SettingsNamespace::defaults`. */
  defaults?: Record<string, Json>;
  /** `extensionSettings.<ns>` of the global file. */
  global?: Record<string, Json>;
  /** The same of the project file (`{}` when the project is untrusted). */
  project?: Record<string, Json>;
};

export type Resolved = { values: Record<string, Json>; diagnostics: Diagnostic[] };

export type WriteLayer = 'global' | 'project';

/** One write that reproduces an effective value from the layer below it. */
export type Write = { key: string; value: Json | undefined };

type Obj = Record<string, Json>;

const isObject = (v: unknown): v is Obj => typeof v === 'object' && v !== null && !Array.isArray(v);

/** Look up a dot path, tolerating v1 flat dotted keys (`{"a.b": 1}`). */
export function lookup(object: Json, key: string): Json | undefined {
  if (!isObject(object)) return undefined;
  if (key in object) return object[key];
  let cursor: Json = object;
  for (const segment of key.split('.')) {
    if (!isObject(cursor)) return undefined;
    cursor = cursor[segment];
  }
  return cursor;
}

/** Set a dot path, creating objects (a non-object on the way is replaced). */
export function setPath(object: Obj, key: string, value: Json): void {
  const segments = key.split('.');
  let cursor = object;
  for (let index = 0; index < segments.length; index += 1) {
    const segment = segments[index]!;
    if (index === segments.length - 1) {
      cursor[segment] = value;
      return;
    }
    const next = cursor[segment];
    if (!isObject(next)) cursor[segment] = {};
    cursor = cursor[segment] as Obj;
  }
}

/** Remove a dot path (missing paths are a no-op). */
export function removePath(object: Obj, key: string): void {
  const segments = key.split('.');
  let cursor: Json = object;
  for (let index = 0; index < segments.length - 1; index += 1) {
    cursor = isObject(cursor) ? cursor[segments[index]!] : undefined;
    if (!isObject(cursor)) return;
  }
  if (isObject(cursor)) delete cursor[segments[segments.length - 1]!];
}

/** Objects merge recursively, everything else replaces (`over` wins). */
export function merge(base: Obj, over: Obj): Obj {
  const out: Obj = { ...base };
  for (const [key, value] of Object.entries(over)) {
    const current = out[key];
    out[key] = isObject(current) && isObject(value) ? merge(current, value) : value;
  }
  return out;
}

/** JSON equality (objects compare key-set and value, order-insensitive). */
export function jsonEqual(a: Json, b: Json): boolean {
  if (a === b) return true;
  if (typeof a === 'number' && typeof b === 'number') return Number.isNaN(a) && Number.isNaN(b);
  if (Array.isArray(a) || Array.isArray(b)) {
    if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return false;
    return a.every((item, index) => jsonEqual(item, b[index]));
  }
  if (isObject(a) && isObject(b)) {
    const keys = Object.keys(a);
    if (keys.length !== Object.keys(b).length) return false;
    return keys.every((key) => key in b && jsonEqual(a[key], b[key]));
  }
  return false;
}

/** Schemastery truthiness: `true`, non-zero, non-empty string / array / object. */
export function isTruthy(value: Json): boolean {
  if (value === null || value === undefined) return false;
  if (typeof value === 'boolean') return value;
  if (typeof value === 'number') return value !== 0;
  if (typeof value === 'string') return value.length > 0;
  if (Array.isArray(value)) return value.length > 0;
  return isObject(value) && Object.keys(value).length > 0;
}

/** §7.4: whether the condition holds on the effective values. */
export function condEval(cond: Cond | undefined, values: Obj): boolean {
  if (!cond) return true;
  const value = lookup(values, cond.key);
  if (cond.eq !== undefined && !jsonEqual(value, cond.eq)) return false;
  if (cond.ne !== undefined && jsonEqual(value, cond.ne)) return false;
  if (cond.in !== undefined && !cond.in.some((option) => jsonEqual(option, value))) return false;
  if (cond.truthy !== undefined && isTruthy(value) !== cond.truthy) return false;
  return true;
}

/** Whether the row is rendered for `values` (`visibleIf`). */
export function fieldVisible(field: Field, values: Obj): boolean {
  return condEval(field.visibleIf, values);
}

/** Whether the row is editable (`disabledIf`) — an invisible row never is. */
export function fieldEnabled(field: Field, values: Obj): boolean {
  if (!fieldVisible(field, values)) return false;
  return field.disabledIf === undefined || !condEval(field.disabledIf, values);
}

const scopeAllows = (scope: FieldScope, layer: WriteLayer): boolean =>
  scope === 'both' || scope === layer;

/** Stored layers that contribute to a field, highest precedence first. */
function storedLayers(scope: FieldScope, global: Obj, project: Obj): Obj[] {
  if (scope === 'global') return [global];
  if (scope === 'project') return [project];
  return [project, global];
}

/** Resolve `ns` over `layers` (§7.1): invalid stored values fall back a layer. */
export function resolve(namespace: Namespace, layers: Layers = {}): Resolved {
  const defaults = layers.defaults ?? namespace.defaults;
  const global = layers.global ?? {};
  const project = layers.project ?? {};
  const values = merge(merge(defaults, global), project);
  const diagnostics: Diagnostic[] = [];
  for (const field of allFields(namespace.sections)) {
    if (!storesValue(field)) continue;
    const stored = storedLayers(fieldScope(field), global, project);
    const storedCount = stored.length;
    let effective: Json | undefined;
    let found = false;
    const candidates: Json[] = [...stored, defaults];
    for (let index = 0; index < candidates.length; index += 1) {
      const layer = candidates[index]!;
      const candidate = lookup(layer, field.key);
      // A stored null is an unset optional key (core writes them on a fresh install): not a wrong-type value.
      if (candidate === undefined || candidate === null) continue;
      const message = checkValue(field, candidate);
      if (!message) {
        effective = candidate;
        found = true;
        break;
      }
      // Schema defaults are not stored values: no diagnostic for them.
      if (index < storedCount) diagnostics.push({ namespace: namespace.namespace, key: field.key, message });
    }
    if (!found && field.default !== undefined && !checkValue(field, field.default)) {
      effective = field.default;
      found = true;
    }
    if (found) setPath(values, field.key, effective);
    // Invalid everywhere and no usable default: the key is unset.
    else removePath(values, field.key);
  }
  return { values, diagnostics };
}

/** §7.2: the writes that turn an effective object into one stored layer. */
export function simplify(namespace: Namespace, layer: WriteLayer, layers: Layers, effective: Obj): Write[] {
  const defaults = layers.defaults ?? namespace.defaults;
  const global = layers.global ?? {};
  const below = layer === 'global' ? defaults : merge(defaults, global);
  const writes: Write[] = [];
  for (const field of allFields(namespace.sections)) {
    if (!storesValue(field) || !scopeAllows(fieldScope(field), layer)) continue;
    const current = lookup(effective, field.key);
    if (current === undefined) continue;
    const base = lookup(below, field.key) ?? field.default;
    writes.push({ key: field.key, value: jsonEqual(base, current) ? undefined : current });
  }
  return writes;
}

/** §7.2 for one write: writing the layer-below value unsets the key. */
export function simplifyKey(field: Field, below: Json | undefined, value: Json): Json | undefined {
  const base = below !== undefined ? below : field.default;
  return jsonEqual(base, value) ? undefined : value;
}

function oneOf(field: Field): string {
  return `expected one of ${field.options.map((option) => option.value).join(', ')}`;
}

function checkLength(field: Field, text: string): string | undefined {
  const length = [...text].length;
  if (field.minLength !== undefined && length < field.minLength) return `must be at least ${field.minLength} characters`;
  if (field.maxLength !== undefined && length > field.maxLength) return `must be at most ${field.maxLength} characters`;
  return undefined;
}

function checkCount(field: Field, count: number): string | undefined {
  if (field.min !== undefined && count < field.min) return `must have at least ${field.min} items`;
  if (field.max !== undefined && count > field.max) return `must have at most ${field.max} items`;
  return undefined;
}

/**
 * Check `value` against `field` (§7 `Field types`) — the port of core's
 * `check_value`. Returns a human-readable reason, or `undefined` when valid.
 */
export function checkValue(field: Field, value: Json): string | undefined {
  switch (field.type) {
    case 'boolean':
      return typeof value === 'boolean' ? undefined : 'expected a boolean';
    case 'number': {
      if (typeof value !== 'number' || !Number.isFinite(value)) return 'expected a number';
      if (field.integer && !Number.isInteger(value)) return 'expected an integer';
      if (field.min !== undefined && value < field.min) return `must be at least ${field.min}`;
      if (field.max !== undefined && value > field.max) return `must be at most ${field.max}`;
      return undefined;
    }
    case 'string': {
      if (typeof value !== 'string') return 'expected a string';
      return checkLength(field, value);
    }
    case 'secret':
    case 'model':
      return typeof value === 'string' ? undefined : 'expected a string';
    case 'enum': {
      if (typeof value !== 'string') return 'expected a string';
      if (!field.allowCustom && !field.options.some((option) => option.value === value)) return oneOf(field);
      return undefined;
    }
    case 'multiselect': {
      if (!Array.isArray(value) || value.some((item) => typeof item !== 'string')) return 'expected a list of strings';
      if (!field.allowCustom && value.some((item) => !field.options.some((option) => option.value === item))) return oneOf(field);
      return checkCount(field, value.length);
    }
    case 'order': {
      if (!Array.isArray(value) || value.some((item) => typeof item !== 'string')) return 'expected a list of strings';
      const known = field.items;
      if (known.length) {
        const unknown = value.find((item) => !known.includes(item as string));
        if (unknown !== undefined) return `${String(unknown)} is not one of ${known.join(', ')}`;
      }
      return undefined;
    }
    case 'list': {
      if (!Array.isArray(value)) return 'expected a list';
      const countError = checkCount(field, value.length);
      if (countError) return countError;
      if (field.item) {
        for (const entry of value) {
          const message = checkValue(field.item, entry);
          if (message) return message;
        }
      }
      return undefined;
    }
    case 'dict': {
      if (!isObject(value)) return 'expected an object';
      if (field.value) {
        for (const entry of Object.values(value)) {
          const message = checkValue(field.value, entry);
          if (message) return message;
        }
      }
      return undefined;
    }
    case 'json':
      return undefined;
    case 'action':
    case 'page':
    case 'unknown':
      return undefined;
  }
}

/**
 * `toZod(field)` (§7.3): the same rules as a zod schema, generated at runtime
 * for instant feedback in the form. `pattern` / `keyPattern` are regexes core
 * also applies, so the app can reject early; core's reply stays authoritative.
 */
export function toZod(field: Field): z.ZodTypeAny {
  switch (field.type) {
    case 'boolean':
      return z.boolean();
    case 'number': {
      let schema = z.number();
      if (field.integer) schema = schema.int();
      if (field.min !== undefined) schema = schema.min(field.min);
      if (field.max !== undefined) schema = schema.max(field.max);
      return schema;
    }
    case 'string': {
      let schema = z.string();
      if (field.minLength !== undefined) schema = schema.min(field.minLength);
      if (field.maxLength !== undefined) schema = schema.max(field.maxLength);
      if (field.pattern) {
        try {
          schema = schema.regex(new RegExp(field.pattern));
        } catch {
          // A pattern the app cannot compile is core's to reject, not the form's.
        }
      }
      return schema;
    }
    case 'secret':
    case 'model':
      return z.string();
    case 'enum': {
      if (field.allowCustom || field.options.length === 0) return z.string();
      const values = field.options.map((option) => option.value);
      return z.string().refine((value) => values.includes(value), { message: oneOf(field) });
    }
    case 'multiselect': {
      let schema = z.array(z.string());
      if (!field.allowCustom && field.options.length) {
        const values = field.options.map((option) => option.value);
        schema = schema.refine((items) => items.every((item) => values.includes(item)), { message: oneOf(field) });
      }
      if (field.min !== undefined) schema = schema.refine((items) => items.length >= field.min!, { message: `must have at least ${field.min} items` });
      if (field.max !== undefined) schema = schema.refine((items) => items.length <= field.max!, { message: `must have at most ${field.max} items` });
      return schema;
    }
    case 'order':
      return z.array(z.string()).refine((items) => !field.items.length || items.every((item) => field.items.includes(item)), {
        message: `must be a subset of ${field.items.join(', ')}`,
      });
    case 'list': {
      let schema = z.array(field.item ? toZod(field.item) : z.unknown());
      if (field.min !== undefined) schema = schema.refine((items) => items.length >= field.min!, { message: `must have at least ${field.min} items` });
      if (field.max !== undefined) schema = schema.refine((items) => items.length <= field.max!, { message: `must have at most ${field.max} items` });
      return schema;
    }
    case 'dict':
      return z.record(z.string(), field.value ? toZod(field.value) : z.unknown());
    case 'json':
      return z.unknown();
    case 'action':
    case 'page':
    case 'unknown':
      return z.unknown();
  }
}

/** The first zod error message for `value`, or `undefined` when it passes. */
export function zodMessage(schema: z.ZodTypeAny, value: Json): string | undefined {
  const result = schema.safeParse(value);
  if (result.success) return undefined;
  return result.error.issues[0]?.message ?? 'invalid value';
}

/** Rows of a section that are visible for `values`, `advanced` unfolded on demand. */
export function visibleFields(section: Section, values: Obj, advanced: boolean): Field[] {
  return section.fields.filter((field) => fieldVisible(field, values) && (advanced || !field.advanced));
}
