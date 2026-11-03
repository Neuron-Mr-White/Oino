/**
 * OSS-2 settings dialect as the app sees it (ui-protocol §7, WP-U11): the
 * `get_settings_schema` namespace shape the host serves, parsed tolerantly —
 * v1 namespaces (bare-string enum options, flat `fields`, missing titles,
 * dotted v1 `order.items`) load unchanged and an unknown field `type` becomes
 * `unknown` (read-only) instead of failing the namespace (D-U14).
 *
 * The Rust side is the source of truth (`oino_ui::settings`); this module only
 * mirrors its wire types.
 */
import { parseSurfaces, shownInApp, type Surface } from '../../lib/surfaces';

/** Any JSON value (the dialect keeps stored numbers/objects verbatim). */
export type Json = unknown;

export type FieldScope = 'both' | 'global' | 'project';

export type FieldType =
  | 'boolean' | 'enum' | 'multiselect' | 'string' | 'number' | 'secret' | 'order'
  | 'model' | 'action' | 'page' | 'list' | 'dict' | 'json' | 'unknown';

export type EnumDisplay = 'select' | 'segmented' | 'radio';

export type Restart = 'session' | 'host';

/** §7 `Cond`: evaluated on effective values, same scope view. */
export type Cond = { key: string; eq?: Json; ne?: Json; in?: Json[]; truthy?: boolean };

export type Choice = { value: string; label: string; description?: string };

/** §7 `Field (common)` plus the type-specific props (Rust `FieldProps`). */
export type Field = {
  key: string;
  type: FieldType;
  label: string;
  description?: string;
  hint?: string;
  badge?: string;
  default?: Json;
  scope: FieldScope;
  advanced: boolean;
  dynamic?: { service: string; op?: string };
  visibleIf?: Cond;
  disabledIf?: Cond;
  restart?: Restart;
  options: Choice[];
  allowCustom: boolean;
  display?: EnumDisplay;
  min?: number;
  max?: number;
  step?: number;
  unit?: string;
  integer: boolean;
  multiline: boolean;
  pattern?: string;
  placeholder?: string;
  minLength?: number;
  maxLength?: number;
  format?: string;
  items: string[];
  filter?: Json;
  command?: string;
  confirm?: string;
  sections: Section[];
  item?: Field;
  value?: Field;
  keyPattern?: string;
  keyLabel?: string;
  addLabel?: string;
  schema?: Json;
  /** The clients that show the field (absent = every client; `lib/surfaces.ts`). */
  surfaces?: Surface[];
};

export type Section = {
  id: string;
  title: string;
  description?: string;
  advanced: boolean;
  fields: Field[];
};

export type Namespace = {
  namespace: string;
  title: string;
  /** Owner: `"oino"` for core (pi settings), the host/app group, or an extension id. */
  extensionId: string;
  icon?: string;
  /** §6.4 sort weight (`order` desc, then title A→Z). */
  order?: number;
  description?: string;
  /** Values for keys without a field default. */
  defaults: Record<string, Json>;
  sections: Section[];
};

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);

const str = (v: unknown): string | undefined => (typeof v === 'string' ? v : undefined);
const num = (v: unknown): number | undefined => (typeof v === 'number' && Number.isFinite(v) ? v : undefined);
const flag = (v: unknown): boolean => v === true;

const FIELD_TYPES: readonly FieldType[] = [
  'boolean', 'enum', 'multiselect', 'string', 'number', 'secret', 'order',
  'model', 'action', 'page', 'list', 'dict', 'json',
];

function options(raw: unknown): Choice[] {
  if (!Array.isArray(raw)) return [];
  const out: Choice[] = [];
  for (const entry of raw) {
    if (typeof entry === 'string') out.push({ value: entry, label: entry });
    else if (isObject(entry)) {
      const value = entry.value ?? entry.label;
      if (typeof value === 'string') {
        out.push({ value, label: str(entry.label) ?? value, ...(str(entry.description) ? { description: str(entry.description) } : {}) });
      }
    }
  }
  return out;
}

/** v1 `order.items` is `[{value, label}]`; OSS-2 uses bare strings. */
function strings(raw: unknown): string[] {
  if (!Array.isArray(raw)) return [];
  const out: string[] = [];
  for (const entry of raw) {
    if (typeof entry === 'string') out.push(entry);
    else if (isObject(entry) && typeof entry.value === 'string') out.push(entry.value);
  }
  return out;
}

export function parseCond(raw: unknown): Cond | undefined {
  if (!isObject(raw) || typeof raw.key !== 'string') return undefined;
  const cond: Cond = { key: raw.key };
  if ('eq' in raw) cond.eq = raw.eq as Json;
  if ('ne' in raw) cond.ne = raw.ne as Json;
  if (Array.isArray(raw.in)) cond.in = raw.in as Json[];
  if (typeof raw.truthy === 'boolean') cond.truthy = raw.truthy;
  return cond;
}

/** One field; `key` is empty inside a `list.item` / `dict.value` template. */
export function parseField(raw: unknown, fallbackKey = ''): Field {
  const map = isObject(raw) ? raw : {};
  const key = str(map.key) ?? fallbackKey;
  const type = (FIELD_TYPES as readonly string[]).includes(str(map.type) ?? '') ? (map.type as FieldType) : 'unknown';
  const display = str(map.display);
  const restart = str(map.restart);
  const dynamic = isObject(map.dynamic) && typeof map.dynamic.service === 'string'
    ? { service: map.dynamic.service, ...(str(map.dynamic.op) ? { op: str(map.dynamic.op) } : {}) }
    : undefined;
  return {
    key,
    type,
    label: str(map.label) ?? key,
    description: str(map.description),
    hint: str(map.hint),
    badge: str(map.badge),
    default: 'default' in map ? (map.default as Json) : undefined,
    scope: map.scope === 'global' || map.scope === 'project' ? map.scope : 'both',
    advanced: flag(map.advanced),
    dynamic,
    visibleIf: parseCond(map.visibleIf),
    disabledIf: parseCond(map.disabledIf),
    restart: restart === 'session' || restart === 'host' ? restart : undefined,
    options: options(map.options),
    allowCustom: flag(map.allowCustom),
    display: display === 'select' || display === 'segmented' || display === 'radio' ? display : undefined,
    min: num(map.min),
    max: num(map.max),
    step: num(map.step),
    unit: str(map.unit),
    integer: flag(map.integer),
    multiline: flag(map.multiline),
    pattern: str(map.pattern),
    placeholder: str(map.placeholder),
    minLength: num(map.minLength),
    maxLength: num(map.maxLength),
    format: str(map.format),
    items: strings(map.items),
    filter: 'filter' in map ? (map.filter as Json) : undefined,
    command: str(map.command),
    confirm: str(map.confirm),
    sections: parseSections(map.sections),
    item: isObject(map.item) ? parseField(map.item) : undefined,
    value: isObject(map.value) ? parseField(map.value) : undefined,
    keyPattern: str(map.keyPattern),
    keyLabel: str(map.keyLabel),
    addLabel: str(map.addLabel),
    schema: 'schema' in map ? (map.schema as Json) : undefined,
    ...(parseSurfaces(map.surfaces) ? { surfaces: parseSurfaces(map.surfaces) } : {}),
  };
}

export function parseSections(raw: unknown): Section[] {
  if (!Array.isArray(raw)) return [];
  const out: Section[] = [];
  for (const entry of raw) {
    if (!isObject(entry)) continue;
    const id = str(entry.id) ?? str(entry.title) ?? '';
    const title = str(entry.title) ?? id;
    out.push({
      id,
      title,
      description: str(entry.description),
      advanced: flag(entry.advanced),
      fields: Array.isArray(entry.fields) ? entry.fields.map((field) => parseField(field)) : [],
    });
  }
  return out;
}

/**
 * One namespace. v1 namespaces carry a flat `fields` array instead of
 * `sections`; those become one section so every namespace is rendered by the
 * same form engine.
 */
export function parseNamespace(raw: unknown): Namespace | undefined {
  if (!isObject(raw)) return undefined;
  const namespace = str(raw.namespace);
  if (!namespace || !/^[a-z0-9][a-z0-9.-]*$/.test(namespace)) return undefined;
  let sections = parseSections(raw.sections);
  if (sections.length === 0 && Array.isArray(raw.fields)) {
    sections = [{ id: namespace, title: str(raw.title) ?? namespace, advanced: false, fields: raw.fields.map((field) => parseField(field)) }];
  }
  return {
    namespace,
    title: str(raw.title) ?? namespace,
    extensionId: str(raw.extensionId) ?? '',
    icon: str(raw.icon),
    order: num(raw.order),
    description: str(raw.description),
    defaults: isObject(raw.defaults) ? (raw.defaults as Record<string, Json>) : {},
    sections,
  };
}

/** Every namespace of a `get_settings_schema` answer, invalid rows dropped. */
export function parseNamespaces(raw: unknown): Namespace[] {
  const list = Array.isArray(raw) ? raw : isObject(raw) && Array.isArray(raw.namespaces) ? raw.namespaces : [];
  return list.map(parseNamespace).filter((ns): ns is Namespace => !!ns);
}

/** §6.4: `order` desc, then title A→Z (extensions default to 0). */
export function sortNamespaces(list: Namespace[]): Namespace[] {
  return [...list].sort((a, b) => (b.order ?? 0) - (a.order ?? 0) || a.title.localeCompare(b.title));
}

/** Core (pi) namespaces: owner `oino`. */
export function isCoreNamespace(ns: Namespace): boolean {
  return ns.extensionId === 'oino' || ns.namespace.startsWith('oino.');
}

/**
 * Fields in render order (§7): section order, `page` drill-ins inlined in
 * place of the page field itself.
 */
export function allFields(sections: Section[]): Field[] {
  const out: Field[] = [];
  for (const section of sections) {
    for (const field of section.fields) {
      if (field.type === 'page') {
        out.push(...allFields(field.sections));
        continue;
      }
      out.push(field);
    }
  }
  return out;
}

export function namespaceFields(ns: Namespace): Field[] {
  return allFields(ns.sections);
}

/** Whether the field holds a value at all (`action` / `page` / `unknown` do not). */
export function storesValue(field: Field): boolean {
  return field.type !== 'action' && field.type !== 'page' && field.type !== 'unknown';
}

/** The scope a field's value lives in (`both` when unset on the wire). */
export function fieldScope(field: Field): FieldScope {
  return field.scope ?? 'both';
}

/** The row anchors `settings/search` jumps to (`#field-<ns>-<key>`). */
export function fieldAnchor(namespace: string, key: string): string {
  const clean = (text: string) => text.replace(/[^A-Za-z0-9_-]+/g, '_');
  return `field-${clean(namespace)}-${clean(key)}`;
}

/** A page row's drill-in anchor (the nested page opens under it). */
export function pageAnchor(namespace: string, key: string): string {
  return `${fieldAnchor(namespace, key)}-page`;
}

/**
 * Fields that exist in the schema for the CLI but do nothing in the app, so the
 * app's settings never show them (`namespace:key`). `quietStartup` only hides the
 * terminal startup banner. Hosts from before `surfaces` existed send it unmarked.
 */
const CLI_ONLY = new Set(['oino.agent:quietStartup']);

/** Whether the app hides a field: marked for other clients (`surfaces`), or a known CLI-only key. */
export function appHidden(namespace: string, key: string, field?: Pick<Field, 'surfaces'>): boolean {
  if (field && !shownInApp(field.surfaces)) return true;
  return CLI_ONLY.has(`${namespace}:${key}`) || (key === 'quietStartup' && namespace.startsWith('oino.'));
}

/** A section list without the fields the app hides (page drill-ins too); sections left empty go. */
function appSections(namespace: string, sections: Section[]): Section[] {
  const out: Section[] = [];
  for (const section of sections) {
    const fields: Field[] = [];
    for (const field of section.fields) {
      if (appHidden(namespace, field.key, field)) continue;
      if (field.type === 'page' && field.sections.length > 0) {
        const inner = appSections(namespace, field.sections);
        if (inner.length === 0) continue;
        fields.push({ ...field, sections: inner });
      } else fields.push(field);
    }
    if (fields.length > 0 || section.fields.length === 0) out.push({ ...section, fields });
  }
  return out;
}

/**
 * The namespaces as the app shows them ([gui-cleanup]): terminal-only fields
 * dropped, and a namespace whose every field is terminal-only (core
 * `oino.tui`) dropped whole, so no empty "Terminal UI" page is left behind.
 */
export function appNamespaces(list: readonly Namespace[]): Namespace[] {
  const out: Namespace[] = [];
  for (const ns of list) {
    const sections = appSections(ns.namespace, ns.sections);
    if (sections.length === 0 && ns.sections.length > 0) continue;
    out.push(sections.length === ns.sections.length && sections.every((s, i) => s === ns.sections[i]) ? ns : { ...ns, sections });
  }
  return out;
}
