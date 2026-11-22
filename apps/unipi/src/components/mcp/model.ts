/**
 * Settings › MCP — the pure half (OIN-101 WI-3, docs/dev/v3/ux/mcp/SPEC.md §2): the lenient reader of the
 * extension's `mcp.servers {cwd}` answer, the status words a row shows, and the Add/Edit sheet's draft
 * (form ↔ file entry ↔ pasted JSON, secrets). Nothing here touches the host or the DOM.
 *
 * The backend answer is fixed only by its service names, so every field is read leniently and a
 * missing field means "an older host": no `trust`, no `files`, no `sourceFile`.
 */

import { parseTrust, type ProjectTrust } from '../../lib/session/trust';

export { parseTrust };
export type { ProjectTrust, TrustState } from '../../lib/session/trust';

export type McpScope = 'global' | 'project';
/** The badge a row wears: where the winning entry lives. */
export type McpBadge = 'global' | 'project' | 'override';

export type McpServer = {
  name: string;
  /** The extension's raw `source` word (`global`, `project`, `project-override`, `acp`). */
  source: string;
  badge: McpBadge;
  /** The file the entry was read from (absolute, or `~/…`). */
  sourceFile?: string;
  /** Another tool's file (`.pi`, `.unipi`, `.mcp.json`) or ACP: Oino never writes it. */
  readOnly: boolean;
  enabled: boolean;
  transport: string;
  status: string;
  error?: string;
  warning?: string;
  toolCount: number;
  tools: string[];
  /** The entry as the file holds it (secrets are `${NAME}` references, never values). */
  entry?: Record<string, unknown>;
};

export type McpFile = { scope: McpScope; path: string; exists?: boolean };
export type McpWarning = { text: string; file?: string; line?: number };
export type McpState = {
  servers: McpServer[];
  warnings: McpWarning[];
  files: McpFile[];
  /** Absent on an older host (the page says so and keeps project editing off). */
  trust?: ProjectTrust;
  totalTools: number;
  /** The answer carried the WI-2 fields (`files` / `trust` / a source file): project scope is understood. */
  modern: boolean;
};

const obj = (v: unknown): Record<string, unknown> => (v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : {});
const str = (v: unknown): string | undefined => (typeof v === 'string' && v ? v : undefined);
const strings = (v: unknown): string[] => (Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string' && !!x) : []);

const WRITABLE_FILE = /(^|[\\/])\.oino[\\/]mcp\.json$/;

function badgeOf(s: Record<string, unknown>): McpBadge {
  const word = (str(s.scope) ?? str(s.source) ?? 'global').toLowerCase();
  if (word.includes('override')) return 'override';
  if (word.startsWith('project')) return 'project';
  return 'global';
}

/** `mcp.servers`, leniently. Pure. */
export function parseMcpState(raw: unknown): McpState {
  const o = obj(raw);
  const servers = (Array.isArray(o.servers) ? o.servers : [])
    .map(obj)
    .filter((s) => typeof s.name === 'string' && s.name)
    .map((s): McpServer => {
      const source = String(s.source ?? 'global');
      const sourceFile = str(s.sourceFile) ?? str(s.source_file) ?? str(s.file);
      const readOnly = typeof s.readOnly === 'boolean' ? s.readOnly : typeof s.readonly === 'boolean' ? s.readonly : source === 'acp' || (!!sourceFile && !WRITABLE_FILE.test(sourceFile));
      // The file entry (`entry`), or a host that spreads it into the row (`url` / `command` / `args` / `headers` / `env`).
      const flat = ['url', 'command', 'args', 'headers', 'env'].some((k) => k in s) ? Object.fromEntries(['url', 'command', 'args', 'headers', 'env'].filter((k) => k in s).map((k) => [k, s[k]])) : undefined;
      const entry = s.entry && typeof s.entry === 'object' && !Array.isArray(s.entry) ? (s.entry as Record<string, unknown>) : flat;
      return {
        name: String(s.name),
        source,
        badge: badgeOf(s),
        ...(sourceFile ? { sourceFile } : {}),
        readOnly,
        enabled: s.enabled !== false,
        transport: String(s.transport ?? (entry && typeof entry.url === 'string' ? 'http' : 'stdio')),
        status: String(s.status ?? 'stopped'),
        ...(str(s.error) ? { error: str(s.error) } : {}),
        ...(str(s.warning) ? { warning: str(s.warning) } : {}),
        toolCount: typeof s.toolCount === 'number' ? s.toolCount : 0,
        tools: strings(s.tools),
        ...(entry ? { entry } : {}),
      };
    });
  const warnings: McpWarning[] = [];
  for (const e of Array.isArray(o.errors) ? o.errors : []) {
    if (typeof e === 'string' && e) warnings.push({ text: e });
  }
  for (const w of Array.isArray(o.warnings) ? o.warnings : []) {
    if (typeof w === 'string') {
      if (w) warnings.push({ text: w });
      continue;
    }
    const x = obj(w);
    const text = str(x.message) ?? str(x.reason);
    if (!text) continue;
    warnings.push({ text, ...(str(x.file) ?? str(x.path) ? { file: str(x.file) ?? str(x.path) } : {}), ...(typeof x.line === 'number' ? { line: x.line } : {}) });
  }
  const files = (Array.isArray(o.files) ? o.files : [])
    .map(obj)
    .filter((f) => typeof f.path === 'string' && f.path)
    .map((f): McpFile => ({ scope: String(f.scope) === 'project' ? 'project' : 'global', path: String(f.path), ...(typeof f.exists === 'boolean' ? { exists: f.exists } : {}) }));
  const trust = parseTrust(o.trust ?? o.project_trust);
  return {
    servers,
    warnings,
    files,
    ...(trust ? { trust } : {}),
    totalTools: typeof o.totalTools === 'number' ? o.totalTools : servers.reduce((n, s) => n + s.toolCount, 0),
    modern: files.length > 0 || !!trust || servers.some((s) => !!s.sourceFile),
  };
}

/* ── status words (SPEC §2) ─────────────────────────────────────────────── */

export type StatusKind = 'connected' | 'disabled' | 'error' | 'untrusted' | 'autonomy' | 'connecting' | 'stopped';

/** What a row says: `Connected · N tools`, `Disabled`, `Error: …`, `Not trusted`, `Needs Autonomy High`. `errorShownBelow`: the full error text is printed under the row (N7), so the pill only says `Error`. */
export function serverStatus(s: Pick<McpServer, 'status' | 'enabled' | 'error' | 'toolCount'>, opts: { errorShownBelow?: boolean } = {}): { kind: StatusKind; label: string } {
  const st = s.status.toLowerCase().replace(/[\s_]+/g, '-');
  if (st === 'not-trusted' || st === 'untrusted') return { kind: 'untrusted', label: 'Not trusted' };
  if (st === 'needs-autonomy-high' || st === 'needs-autonomy' || st === 'autonomy') return { kind: 'autonomy', label: 'Needs Autonomy High' };
  if (!s.enabled || st === 'disabled') return { kind: 'disabled', label: 'Disabled' };
  if (st === 'error' || st === 'failed' || s.error) return { kind: 'error', label: opts.errorShownBelow && s.error ? 'Error' : `Error: ${firstLine(s.error ?? 'could not connect')}` };
  if (st === 'running' || st === 'connected') return { kind: 'connected', label: `Connected · ${s.toolCount} ${s.toolCount === 1 ? 'tool' : 'tools'}` };
  if (st === 'starting' || st === 'connecting') return { kind: 'connecting', label: 'Connecting…' };
  return { kind: 'stopped', label: 'Stopped' };
}

/**
 * N10: a server that cannot start below High says what to do. `note` is the extension's warning without its vague
 * "Reconnect retries…" tail; `hint` is the instruction (the chat to switch when one of the project is open).
 */
export function autonomyHint(s: Pick<McpServer, 'status' | 'warning' | 'error'>, chat?: { title: string; level: string }, inProject = true): { note?: string; hint: string } | undefined {
  const needs = serverStatus({ status: s.status, enabled: true, toolCount: 0 }).kind === 'autonomy' || /needs autonomy high/i.test(`${s.warning ?? ''} ${s.error ?? ''}`);
  if (!needs) return undefined;
  const note = s.warning?.replace(/\s*Reconnect retries the project entry\.?\s*$/, '').trim();
  const hint = !inProject
    ? 'Set the default autonomy to High under Settings › Autonomy, then Reconnect.'
    : chat?.level === 'high'
    ? `The chat “${chat.title}” is on High autonomy: press Reconnect.`
    : chat
    ? `Switch the chat “${chat.title}” to High autonomy, then Reconnect.`
    : 'Open a chat in this project, switch it to High autonomy, then Reconnect.';
  return { ...(note ? { note } : {}), hint };
}

const firstLine = (t: string): string => t.split('\n', 1)[0]!.trim();

export const BADGE_LABEL: Record<McpBadge, string> = { global: 'Global', project: 'Project', override: 'Project overrides global' };

/** `~/.oino/mcp.json` → `mcp.json`-style short name for the "source file" cell. */
export function shortFile(path: string): string {
  return path.replace(/^\/home\/[^/]+/, '~').replace(/^\/Users\/[^/]+/, '~');
}

/** The folder name a project scope shows (`/a/b/demoeditor/` → `demoeditor`). */
export function projectLabel(cwd: string): string {
  return cwd.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || cwd;
}

/* ── the Add/Edit draft ─────────────────────────────────────────────────── */

export type McpType = 'http' | 'command';

/** One header / env row. `ref` = the `${NAME}` the file already holds for a stored secret. */
export type KvRow = { id: number; key: string; value: string; secret: boolean; ref?: string; /** the user flipped Secret by hand: the key no longer decides it */ touched?: boolean; /** Replace was pressed on a stored secret: an empty field still means "keep" */ replacing?: boolean; /** a catalog placeholder (`${VAR}`): nothing is stored, the user must enter a value */ required?: boolean };

export type Draft = {
  name: string;
  scope: McpScope;
  type: McpType;
  url: string;
  command: string;
  /** One line; split like a shell (`splitArgs`). */
  args: string;
  headers: KvRow[];
  env: KvRow[];
  /** Keys the form has no field for (`enabled`, `exposure`, …) ride along unchanged. */
  extra: Record<string, unknown>;
};

/** Secret toggle default (SPEC §2): on for keys matching authorization|token|key|secret. */
export const isSecretKey = (key: string): boolean => /authorization|token|key|secret|auth|password|pat|credential/i.test(key);

/** A whole-value `${NAME}` reference. */
export const REF = /^\$\{([A-Za-z_][A-Za-z0-9_]*)\}$/;

let rowSeq = 0;
export const newRow = (key = '', value = '', secret = isSecretKey(key)): KvRow => ({ id: ++rowSeq, key, value, secret });

export function emptyDraft(scope: McpScope = 'global'): Draft {
  return { name: '', scope, type: 'http', url: '', command: '', args: '', headers: [], env: [], extra: {} };
}

/** Shell-ish split: whitespace separates, `"…"`/`'…'` group, backslash escapes inside double quotes. */
export function splitArgs(line: string): string[] {
  const out: string[] = [];
  let cur = '';
  let quote: '"' | "'" | undefined;
  let has = false;
  for (let i = 0; i < line.length; i++) {
    const c = line[i]!;
    if (quote) {
      if (c === quote) quote = undefined;
      else if (c === '\\' && quote === '"' && i + 1 < line.length) cur += line[++i];
      else cur += c;
    } else if (c === '"' || c === "'") {
      quote = c;
      has = true;
    } else if (/\s/.test(c)) {
      if (cur || has) out.push(cur);
      cur = '';
      has = false;
    } else cur += c;
  }
  if (cur || has) out.push(cur);
  return out;
}

export function joinArgs(args: readonly string[]): string {
  return args.map((a) => (a === '' ? '""' : /[\s"']/.test(a) ? `"${a.replace(/["\\]/g, '\\$&')}"` : a)).join(' ');
}

const ENTRY_KEYS = ['url', 'command', 'serverUrl', 'httpUrl', 'type', 'transport', 'args', 'env', 'headers'];
const FORM_KEYS = new Set(['url', 'command', 'args', 'env', 'headers', 'type', 'transport', 'serverUrl', 'httpUrl', 'name']);

function rowsOf(v: unknown): KvRow[] {
  return Object.entries(obj(v)).map(([key, raw]) => {
    const value = typeof raw === 'string' ? raw : raw === undefined || raw === null ? '' : String(raw);
    const ref = REF.exec(value) || (value === '••••' ? [value] : null);
    const secret = !!ref || isSecretKey(key) || /^bearer\s/i.test(value);
    return ref ? { ...newRow(key, '', true), ref: value } : newRow(key, value, secret);
  });
}

/** A file entry (or one from Claude Code / Cursor / pi) → the form's draft. */
export function draftFromEntry(name: string, entry: Record<string, unknown>, scope: McpScope): Draft {
  const url = str(entry.url) ?? str(entry.serverUrl) ?? str(entry.httpUrl);
  const type: McpType = url || (!entry.command && /http|sse/i.test(String(entry.type ?? entry.transport ?? ''))) ? 'http' : 'command';
  const extra: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(entry)) if (!FORM_KEYS.has(k)) extra[k] = v;
  return {
    name,
    scope,
    type,
    url: url ?? '',
    command: typeof entry.command === 'string' ? entry.command : '',
    args: joinArgs(strings(entry.args)),
    headers: rowsOf(entry.headers),
    env: rowsOf(entry.env),
    extra,
  };
}

/**
 * A catalog entry → the form's draft (N4). Its `${VAR}` env / header values are placeholders for what the user must
 * supply, not references to a stored secret: they start empty and required, never as "•••• Replace".
 */
export function draftFromCatalog(name: string, entry: Record<string, unknown>, scope: McpScope): Draft {
  const d = draftFromEntry(name, entry, scope);
  const open = (rows: KvRow[]): KvRow[] => rows.map((r) => (r.ref && REF.test(r.ref) ? { ...r, ref: undefined, value: '', required: true } : r));
  return { ...d, headers: open(d.headers), env: open(d.env) };
}

/** Names of the required rows (catalog placeholders) still empty in the visible kind of the draft. */
export function missingRequired(d: Draft): string[] {
  const rows = d.type === 'http' ? d.headers : d.env;
  return rows.filter((r) => r.required && !r.ref && !r.value.trim()).map((r) => r.key.trim() || 'a value');
}

const upperVar = (t: string): string => t.toUpperCase().replace(/[^A-Z0-9]+/g, '_').replace(/^_+|_+$/g, '');

/**
 * `Linear` + `Authorization` → `LINEAR_AUTHORIZATION` (the store key `unipi.mcp/<SERVER>_<KEY>` and the `${…}` the file keeps).
 * N8: a key that already starts with the server's name is not prefixed again (`refero` + `REFERO_AUTH` → `REFERO_AUTH`,
 * not `REFERO_REFERO_AUTH`). Must stay the same function as `storage::secret_key` in extensions/unipi/mcp.
 */
export function secretVar(server: string, key: string): string {
  const s = upperVar(server);
  const k = upperVar(key);
  return s && (k === s || k.startsWith(`${s}_`)) ? k : upperVar(`${s}_${k}`);
}

export type Built = { entry: Record<string, unknown>; secrets: Record<string, string> };

function buildRows(server: string, rows: readonly KvRow[], secrets: Record<string, string>): Record<string, string> | undefined {
  const out: Record<string, string> = {};
  for (const r of rows) {
    const key = r.key.trim();
    if (!key) continue;
    if (r.ref && !r.value) out[key] = r.ref; // an untouched stored secret stays a reference
    else if (r.secret && r.value && !REF.test(r.value.trim())) {
      const v = secretVar(server, key);
      secrets[v] = r.value;
      out[key] = `\${${v}}`;
    } else out[key] = r.value;
  }
  return Object.keys(out).length ? out : undefined;
}

/** The draft as the file entry plus the secret values to store (the entry never holds a secret value). */
export function buildEntry(d: Draft): Built {
  const secrets: Record<string, string> = {};
  const name = d.name.trim();
  const entry: Record<string, unknown> = { ...d.extra };
  if (d.type === 'http') {
    entry.url = d.url.trim();
    const headers = buildRows(name, d.headers, secrets);
    if (headers) entry.headers = headers;
  } else {
    entry.command = d.command.trim();
    const args = splitArgs(d.args);
    if (args.length) entry.args = args;
    const env = buildRows(name, d.env, secrets);
    if (env) entry.env = env;
  }
  return { entry, secrets };
}

export type Problems = Partial<Record<'name' | 'url' | 'command' | 'headers' | 'env', string>>;

/** Validation the sheet shows before Save. `taken` = names already in the chosen scope (add mode only). */
export function validateDraft(d: Draft, opts: { taken?: readonly string[] } = {}): Problems {
  const p: Problems = {};
  const name = d.name.trim();
  if (!name) p.name = 'Give the server a name';
  else if (!/^[A-Za-z0-9][A-Za-z0-9_-]*$/.test(name)) p.name = 'Use letters, numbers, - and _';
  else if (opts.taken?.includes(name)) p.name = `“${name}” already exists in this scope — edit it instead`;
  if (d.type === 'http') {
    const url = d.url.trim();
    if (!url) p.url = 'Enter the server URL';
    else if (!/^https?:\/\/[^\s/]+/i.test(url)) p.url = 'The URL starts with http:// or https://';
  } else if (!d.command.trim()) p.command = 'Enter the command to run';
  const dup = (rows: readonly KvRow[]) => {
    const seen = new Set<string>();
    for (const r of rows) {
      const k = r.key.trim().toLowerCase();
      if (!k) continue;
      if (seen.has(k)) return `“${r.key.trim()}” appears twice`;
      seen.add(k);
    }
    return undefined;
  };
  const h = d.type === 'http' ? dup(d.headers) : undefined;
  const e = d.type === 'command' ? dup(d.env) : undefined;
  const need = missingRequired(d);
  const needMsg = need.length ? `Enter a value for ${need.join(', ')}` : undefined;
  if (h ?? (d.type === 'http' ? needMsg : undefined)) p.headers = (h ?? needMsg)!;
  if (e ?? (d.type === 'command' ? needMsg : undefined)) p.env = (e ?? needMsg)!;
  return p;
}

/* ── Paste JSON ─────────────────────────────────────────────────────────── */

export type Pasted = { name?: string; entry: Record<string, unknown> };
export type PasteResult = { ok: true; items: Pasted[]; shape: 'entry' | 'named' | 'mcpServers' } | { ok: false; error: string };

const looksLikeEntry = (o: Record<string, unknown>): boolean => ENTRY_KEYS.some((k) => k in o) && (typeof o.url === 'string' || typeof o.command === 'string' || typeof o.serverUrl === 'string' || typeof o.httpUrl === 'string');

/**
 * One entry, `{name: entry}`, or a whole `{mcpServers: {…}}` (Claude Code / Cursor / pi). A pasted
 * `{"mcpServers": …}` may be a fragment (`"name": {…}` without braces): that is wrapped too.
 */
export function parsePaste(text: string): PasteResult {
  const src = text.trim();
  if (!src) return { ok: false, error: 'Paste the server’s JSON first' };
  type Attempt = { ok: true; v: unknown } | { ok: false; e: Error };
  const attempt = (t: string): Attempt => {
    try {
      return { ok: true, v: JSON.parse(t) as unknown };
    } catch (e) {
      return { ok: false, e: e as Error };
    }
  };
  let r = attempt(src);
  // A fragment copied from inside a file: `"linear": { … }`.
  if (!r.ok && /^"[^"]+"\s*:/.test(src)) {
    const wrapped = attempt(`{${src.replace(/,\s*$/, '')}}`);
    if (wrapped.ok) r = wrapped;
  }
  if (!r.ok) return { ok: false, error: `That is not valid JSON${lineOf(src, r.e)}` };
  const value = r.v;
  const root = obj(value);
  if (!value || typeof value !== 'object' || Array.isArray(value) || !Object.keys(root).length) return { ok: false, error: 'Expected a JSON object with one MCP server entry' };
  const named = (o: Record<string, unknown>): Pasted[] | undefined => {
    const items: Pasted[] = [];
    for (const [name, v] of Object.entries(o)) {
      const e = obj(v);
      if (!v || typeof v !== 'object' || Array.isArray(v) || !looksLikeEntry(e)) return undefined;
      items.push({ name, entry: e });
    }
    return items;
  };
  if ('mcpServers' in root) {
    const inner = obj(root.mcpServers);
    const items = named(inner);
    if (!items?.length) return { ok: false, error: '“mcpServers” holds no servers with a url or a command' };
    return { ok: true, items, shape: 'mcpServers' };
  }
  if (looksLikeEntry(root)) return { ok: true, items: [{ entry: root }], shape: 'entry' };
  const items = named(root);
  if (items?.length) return { ok: true, items, shape: 'named' };
  return { ok: false, error: 'No server found: an entry needs a "url" (HTTP) or a "command"' };
}

function lineOf(src: string, e: Error): string {
  const pos = /position (\d+)/.exec(e.message)?.[1];
  if (pos === undefined) return '.';
  const line = src.slice(0, Number(pos)).split('\n').length;
  return ` (line ${line})`;
}

/** Pasted items → drafts (the single entry takes the name typed in the form). */
export function draftsFromPaste(items: readonly Pasted[], scope: McpScope, fallbackName: string): Draft[] {
  return items.map((it) => draftFromEntry(it.name ?? fallbackName, it.entry, scope));
}
