/**
 * Mock host (app spec §3.15, WP-A1): `MockHost implements HostClient` with
 * in-memory hosts, session cards, sessions, fs tree, git status and cron
 * data, for the capture harness (`vite.config.capture.ts`) and component
 * tests (`createMockHost(scenario)`). Never opens a socket.
 *
 * Every op is answered: scenario `ops` overrides first, then the built-in
 * table below, else `null` (logged with `unhandled: true` so a capture can
 * spot an op a screen needs). Every request, session command and stream
 * open is recorded on `world.log` (and `window.__UNIPI_MOCK_LOG__` in the
 * browser) for assertions.
 */
import { createSignal } from 'solid-js';
import { parseNamespace } from '../../ui/settings/types';
import { resolve, removePath, setPath } from '../../ui/settings/semantics';
import { createMockBoard, type MockBoard, type MockBoardProject } from './board';
import { MOCK_MODELS, mockLevelsFor, mockModelFor } from './models';
import { mockLspServers, openMockLsp, type MockLspSpec } from './lsp';
import {
  APP_HOST,
  type Closable,
  type EventsStream,
  type HostClient,
  type HostConn,
  type HubEvent,
  type LspStream,
  type RpcCommand,
  type SessionCard,
  type SessionEventDto,
  type SessionSnapshot,
  type SessionStream,
  type TermStream,
} from '../../lib/host/client';

export type MockOp = (args: Record<string, unknown>, host: MockHost) => unknown;

/** One step of a session script: an event, or the stream dropping (`down` ms unreachable; `Infinity` = stays down). */
export type MockStep = { at: number; dto: SessionEventDto } | { at: number; drop: { code?: string; message?: string; down?: number } };

/**
 * A recorded session: snapshot + script replayed on a virtual clock (from the
 * first open). A reopen with `sinceSeq` gets a resume patch (`replay` = the
 * due events after it), like the host's ring (host.md §7.4). `cmd` answers
 * session commands by `type` (default: `get_commands` from the snapshot,
 * `get_entries` from `entries`, everything else `{}`).
 */
export type MockSession = {
  snapshot: SessionSnapshot;
  events?: MockStep[];
  /** What `get_entries` answers (the host's entries after the script ran). */
  entries?: { entries: unknown[]; leafId: string | null };
  cmd?: Record<string, (cmd: RpcCommand, host: MockHost) => unknown>;
};

export type MockFsNode =
  | { kind: 'dir'; children: Record<string, MockFsNode>; git?: 'M' | 'A' | 'D' | '?'; ignored?: boolean }
  | { kind: 'file'; text: string; mtime?: number; git?: 'M' | 'A' | 'D' | '?'; ignored?: boolean };

/** One changed file of a mock repo (the `git status` row the Git pane draws). */
export type MockGitFile = {
  path: string;
  area: 'staged' | 'unstaged' | 'untracked' | 'conflict';
  /** Porcelain letter (`M`, `A`, `D`, `?`, `U`; default `M`, `?` for untracked). */
  status?: string;
  add?: number;
  del?: number;
  binary?: boolean;
  /** Rename source. */
  orig?: string;
};

/** A mock repo (WP-A15): every `git_op` action answers from this one spec. */
export type MockGit = {
  repo?: boolean;
  root?: string;
  branch?: string;
  head?: string;
  detached?: boolean;
  upstream?: string;
  ahead?: number;
  behind?: number;
  remotes?: string[];
  stashes?: number;
  operation?: string;
  /** `git status` rows, in the grouping the pane shows. */
  files?: MockGitFile[];
  /** Branch source: the base ref; `null` = no base (the pane's empty state). */
  base?: string | null;
  /** `git log` (newest first); `short`/`author`/`time`/`refs` default. */
  commits?: Array<{ hash: string; short?: string; subject: string; author?: string; time?: number; refs?: string[] }>;
  /** The status list was capped ("Showing the first 2 000 changed files"). */
  more?: boolean;
  /** Diff text per path (default: a plausible hunk). */
  diffs?: Record<string, string>;
};

/** One row of the `extensions` host_cmd answer (`ExtensionInfo`). */
export type MockExtension = {
  id: string;
  kind?: string;
  enabled?: boolean;
  loaded?: boolean;
  path?: string;
  /** Row-level note ("trapped in a session: …"), shown under the toggle. */
  note?: string;
  /** User-installed (`<agent dir>/extensions`): Settings offers Remove (ext-install). */
  removable?: boolean;
};

/** One OSS-2 settings namespace of the `get_settings_schema` answer (WP-U11):
 *  either the full dialect (`sections`, `defaults` and the stored `global` /
 *  `project` layers) or the v1 flat `fields` shorthand A24's fixtures use. */
export type MockNamespace = {
  namespace: string;
  title?: string;
  description?: string;
  extensionId?: string;
  icon?: string;
  order?: number;
  defaults?: Record<string, unknown>;
  sections?: unknown[];
  fields?: Array<{ key: string; type?: string; title?: string; description?: string }>;
  /** Stored global layer (`extensionSettings.<ns>` of ~/.oino/settings.json). */
  global?: Record<string, unknown>;
  /** Stored project layer (`<project>/.oino/settings.json`). */
  project?: Record<string, unknown>;
};

/** The settings fixtures a scenario seeds (WP-A24): the host's `config.json`,
 * the merged pi agent settings, the extension namespaces and the list. */
export type MockSettings = {
  host?: Record<string, unknown>;
  pi?: Record<string, unknown>;
  namespaces?: MockNamespace[];
  extensions?: MockExtension[];
  /** `_oino/auth_providers` rows (WP-A25 Providers page). */
  providers?: Array<{ id: string; name?: string; oauth?: boolean; configured?: boolean; stored?: string; env?: string[] }>;
};

/** One memory file of the mock store (`memory_browse`). */
export type MockMemory = { id: string; title: string; kind: string; project: string; updated_at: number; body: string };

export type MockHostSpec = {
  id: string;
  name: string;
  /** `local` = This computer (desktop app), else iroh. */
  transport?: 'local' | 'iroh';
  conn?: HostConn;
  cards?: SessionCard[];
  /** WP-A5 (Z-13): the host's trash — `sessions_list{archived:true}` rows. */
  archived?: SessionCard[];
  /** The memory extension's store behind `memory_browse` (owner fix 2026-11-03); a function builds it on first use. */
  memories?: MockMemory[] | (() => MockMemory[]);
  /** `usage_stats` answers (owner fix 2026-11-04): a builder over the requested range; `null` = the op is unsupported (old host). */
  usage?: ((days: number) => unknown) | null;
  /** `usage_stats` waits this long (ms) before answering. */
  usageDelayMs?: number;
  /** `memory_browse` waits this long (ms) before answering (loading states in captures). */
  memoryDelayMs?: number;
  sessions?: Record<string, MockSession>;
  /** A `session_new` stream with no session id serves this session's script (default: none → E_NOT_FOUND). */
  newSessionFrom?: string;
  /** Root folders → tree. */
  fs?: Record<string, MockFsNode>;
  git?: MockGit;
  cron?: unknown[];
  /** Scripted language server behind `openLsp`/`lsp_servers` (WP-A21). */
  lsp?: MockLspSpec;
  /** Board projects + tasks behind `board_cli`/`board_watch` (WP-A22). */
  board?: MockBoardProject[];
  /** Settings fixtures (WP-A24): host config, pi settings, namespaces, extensions. */
  settings?: MockSettings;
  /** Retained host notices (§7.4): replayed to every `events_open` subscriber
   *  first (the safe-mode banner names its suspects, G-19). */
  notices?: Array<{ level?: 'info' | 'warning'; text: string }>;
  /** Hub events replayed on the world clock from the first `events_open`
   *  (WP-A19): card transitions for needs-you / unread / finished captures. */
  hub?: Array<{ at: number; event: HubEvent }>;
  /** `_oino/render` answers, keyed `"<target>/<key>"` → node | null | `{never:true}` (WP-A8).
   *  `never` never resolves, so the app's 2 s render timeout path can be captured. */
  renderers?: Record<string, unknown>;
  /** WP-A25: what a typed `/command` opens — the extension's UI v2 panel as an
   *  overlay entry (real extensions push the same entry from `openPanel`). */
  panels?: Record<string, { id: string; title: string; extensionId?: string; root: unknown }>;
  ops?: Record<string, MockOp>;
};

export type MockScenario = {
  name: string;
  hosts: MockHostSpec[];
  /** App-level op overrides (`list_hosts`, `forward_list`, …). */
  appOps?: Record<string, MockOp>;
  /** 0 = settle scripts instantly (default); otherwise playback speed multiplier. */
  speed?: number;
  /** Capture only: runs in `boot.ts` after storage is cleared (seed prefs, plug stand-in pane views). */
  setup?: () => void;
};

export type MockLogEntry = { host: string; kind: 'request' | 'cmd' | 'open' | 'lsp'; op: string; args?: unknown; unhandled?: true; at: number };

declare global {
  interface Window {
    __UNIPI_MOCK_LOG__?: MockLogEntry[];
  }
}

type Err = { code: string; message: string; hint?: string; retryable: boolean };
const err = (code: string, message: string, hint?: string): Err => ({ code, message, hint, retryable: code === 'E_TRANSPORT' || code === 'E_TIMEOUT' });

/** The sign-in URL the mock's `_oino/login` dialog carries (the app opens it externally). */
export const MOCK_OAUTH_URL = 'https://auth.example.com/authorize?client=unipi';

const MIME: Record<string, string> = {
  png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', gif: 'image/gif', webp: 'image/webp', svg: 'image/svg+xml',
  pdf: 'application/pdf', mp4: 'video/mp4', webm: 'video/webm', mp3: 'audio/mpeg', ogg: 'audio/ogg', wav: 'audio/wav',
  md: 'text/markdown', json: 'application/json', txt: 'text/plain', ts: 'text/x-typescript', rs: 'text/rust',
};
function mimeOf(path: string): string {
  const ext = path.slice(path.lastIndexOf('.') + 1).toLowerCase();
  return MIME[ext] ?? 'application/octet-stream';
}

/** 1×1 transparent PNG (same as `gen.ts` `TINY_PNG`), as bytes. */
const TINY_PNG_BYTES = Array.from(
  atob('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg=='),
  (ch) => ch.charCodeAt(0),
);

function walk(root: Record<string, MockFsNode> | undefined, path: string): MockFsNode | undefined {
  if (!root) return undefined;
  const base = Object.keys(root).sort((a, b) => b.length - a.length).find((r) => path === r || path.startsWith(`${r}/`));
  if (!base) return undefined;
  let node: MockFsNode | undefined = root[base];
  for (const part of path.slice(base.length).split('/').filter(Boolean)) {
    node = node?.kind === 'dir' ? node.children[part] : undefined;
  }
  return node;
}

/** The repo half of every `git_op` answer (proto `GitRepoData`). */
function mockRepo(g: MockGit) {
  return {
    repo: g.repo !== false,
    root: g.root,
    branch: g.branch ?? 'main',
    head: g.head ?? 'a1b2c3d4',
    detached: g.detached ?? false,
    upstream: g.upstream,
    ahead: g.ahead ?? 0,
    behind: g.behind ?? 0,
    remotes: g.remotes ?? ['origin'],
    stashes: g.stashes ?? 0,
    operation: g.operation,
  };
}

/** A plausible unified diff for a row (rows may carry `diffs[path]` instead). */
function mockDiff(g: MockGit, path: string, file?: MockGitFile) {
  if (g.diffs?.[path]) return { text: g.diffs[path]!, truncated: false, binary: false };
  if (file?.binary) return { text: '', truncated: false, binary: true };
  const old = `const total = items.length;\nreturn total;\n`;
  const next = `const total = items.reduce((n, item) => n + item.weight, 0);\nreturn total;\n`;
  return {
    text: [
      `diff --git a/${path} b/${path}`,
      `--- a/${path}`,
      `+++ b/${path}`,
      `@@ -12,7 +12,9 @@ export function tally(items: Item[]): number {`,
      '   // Count what the list weighs, not how many rows it has.',
      ...old.split('\n').slice(0, -1).map((line) => `-${line}`),
      ...next.split('\n').slice(0, -1).map((line) => `+${line}`),
      ' }',
      '',
    ].join('\n'),
    truncated: false,
    binary: false,
  };
}

const mockRow = (g: MockGit, f: MockGitFile) => ({
  path: f.path,
  area: f.area,
  status: f.status ?? (f.area === 'untracked' ? '?' : f.area === 'conflict' ? 'U' : 'M'),
  add: f.add ?? 0,
  del: f.del ?? 0,
  binary: f.binary ?? false,
  ...(f.orig ? { orig: f.orig } : {}),
});

/** `git_op` (WP-A15): every action answers from the spec, writes mutate `files`. */
function mockGitOp(cmd: { action: string; [k: string]: unknown }, g: MockGit): unknown {
  const files = g.files ?? [];
  const withDiff = (f: MockGitFile) => ({ ...mockRow(g, f), diff: mockDiff(g, f.path, f) });
  switch (cmd.action) {
    case 'repo':
      return mockRepo(g);
    case 'status':
      return { ...mockRepo(g), files: files.map((f) => mockRow(g, f)), more: g.more ?? false };
    case 'diff': {
      const path = String(cmd.file ?? '');
      const file = files.find((f) => f.path === path);
      return mockDiff(g, path, file);
    }
    case 'turn_files': {
      const asked = new Set((cmd.files as string[] | undefined) ?? []);
      return { ...mockRepo(g), files: files.filter((f) => asked.has(f.path)).map(withDiff) };
    }
    case 'shortstat': {
      const sum = files.reduce((t, f) => ({ add: t.add + (f.add ?? 0), del: t.del + (f.del ?? 0) }), { add: 0, del: 0 });
      return { repo: g.repo !== false, ...sum, capped: g.more ?? false };
    }
    case 'branch_files':
      return {
        ...mockRepo(g),
        base: g.base === undefined ? (g.upstream ?? 'main') : g.base,
        files: g.base === null ? [] : files.map(withDiff),
      };
    case 'log': {
      const all = g.commits ?? [];
      const skip = Number(cmd.skip ?? 0);
      const limit = Number(cmd.limit ?? 50);
      return {
        commits: all.slice(skip, skip + limit).map((c) => ({
          hash: c.hash,
          short: c.short ?? c.hash.slice(0, 7),
          subject: c.subject,
          author: c.author ?? 'You',
          time: c.time ?? 1_760_000_000,
          refs: c.refs ?? [],
        })),
        more: skip + limit < all.length,
      };
    }
    case 'show': {
      const rev = String(cmd.rev ?? '');
      const commit = (g.commits ?? []).find((c) => c.hash === rev || c.hash.startsWith(rev));
      const row = { hash: rev, short: rev.slice(0, 7), subject: commit?.subject ?? 'Commit', author: commit?.author ?? 'You', time: commit?.time ?? 1_760_000_000, refs: commit?.refs ?? [] };
      return {
        commit: row,
        body: '',
        files: files.slice(0, 4).map((f) => ({ path: f.path, status: f.status ?? 'M', add: f.add ?? 0, del: f.del ?? 0 })),
        diff: files.slice(0, 4).map((f) => mockDiff(g, f.path, f).text).join(''),
        truncated: false,
      };
    }
    case 'branches':
      return {
        branches: [
          { name: g.branch ?? 'main', current: true, upstream: g.upstream, ahead: g.ahead ?? 0, behind: g.behind ?? 0, time: 1_760_000_000, subject: g.commits?.[0]?.subject ?? 'Commit' },
          { name: 'feat/review-pane', time: 1_759_000_000, subject: 'Add the review pane' },
          ...(g.remotes ?? []).map((r) => ({ name: `${r}/${g.branch ?? 'main'}`, remote: true })),
        ],
      };
    case 'stage': {
      const paths = (cmd.paths as string[] | undefined) ?? files.map((f) => f.path);
      for (const f of files) if (paths.includes(f.path) && f.area !== 'conflict') f.area = 'staged';
      return null;
    }
    case 'unstage': {
      const paths = (cmd.paths as string[] | undefined) ?? files.map((f) => f.path);
      for (const f of files) if (paths.includes(f.path) && f.area === 'staged') f.area = f.status === 'A' ? 'untracked' : 'unstaged';
      return null;
    }
    case 'discard':
      g.files = files.filter((f) => !((cmd.paths as string[] | undefined) ?? []).includes(f.path));
      return null;
    case 'commit': {
      const subject = String(cmd.message ?? '').split('\n')[0];
      g.files = files.filter((f) => f.area !== 'staged');
      const head = { hash: g.head ?? 'a1b2c3d4', subject, author: 'You', time: Math.floor(Date.now() / 1000), refs: [`HEAD -> ${g.branch ?? 'main'}`] };
      g.commits = [head, ...(g.commits ?? [])];
      return { hash: head.hash, subject };
    }
    case 'switch':
      g.branch = String(cmd.branch ?? g.branch ?? 'main');
      return null;
    case 'pull':
      return { output: 'Already up to date.' };
    case 'push':
      return { output: `To origin\n   ${g.branch ?? 'main'} -> ${g.branch ?? 'main'}` };
    default:
      return { output: 'Done.' };
  }
}

/** Sets one dotted path of a plain object (mock settings writes). */
function setDotted(target: Record<string, unknown>, key: string, value: unknown): void {
  const parts = key.split('.').filter(Boolean);
  if (!parts.length) return;
  let node: Record<string, unknown> = target;
  for (const part of parts.slice(0, -1)) {
    if (typeof node[part] !== 'object' || node[part] === null) node[part] = {};
    node = node[part] as Record<string, unknown>;
  }
  const leaf = parts.at(-1)!;
  if (value === undefined || value === null) delete node[leaf];
  else node[leaf] = value;
}

/** Reads one dotted path of a plain object (`get_settings{key}`). */
function getDotted(source: Record<string, unknown>, key: string): unknown {
  let node: unknown = source;
  for (const part of key.split('.').filter(Boolean)) {
    if (typeof node !== 'object' || node === null) return null;
    node = (node as Record<string, unknown>)[part];
  }
  return node === undefined ? null : node;
}

/** The stored layers of a namespace fixture: explicit `global` / `project`,
 *  else the A24 `pi.extensionSettings.<ns>` seed for the global layer. */
function mockLayers(fixture: MockNamespace, pi: Record<string, unknown>): { defaults?: Record<string, unknown>; global?: Record<string, unknown>; project?: Record<string, unknown> } {
  const seeded = (pi.extensionSettings as Record<string, Record<string, unknown>> | undefined)?.[fixture.namespace];
  return {
    defaults: (fixture.defaults ?? {}) as Record<string, never>,
    global: (fixture.global ?? seeded ?? {}) as Record<string, never>,
    project: (fixture.project ?? {}) as Record<string, never>,
  };
}

/** `host_cmd` (WP-A24): the hub-level `_oino/*` subset over the scenario's
 *  settings fixtures — pi settings, extension namespaces, the extension list
 *  (toggles mutate it, so a round trip is observable) and the auth shell. */
function mockHostCmd(cmd: { kind: string } & Record<string, unknown>, h: MockHost): unknown {
  const fixtures = h.spec.settings ?? {};
  const pi = (fixtures.pi ??= {});
  const namespaces = fixtures.namespaces ?? [];
  switch (cmd.kind) {
    case 'get_settings':
      return cmd.key === undefined || cmd.key === null
        ? { settings: pi }
        : { value: getDotted(pi, String(cmd.key)) };
    case 'set_setting': {
      if (!String(cmd.key ?? '')) throw err('E_SCHEMA', 'Setting key cannot be empty');
      setDotted(pi, String(cmd.key), cmd.value ?? null);
      return null;
    }
    case 'get_settings_schema':
      return { namespaces };
    case 'get_extension_settings': {
      const ns = String(cmd.namespace ?? '');
      const fixture = namespaces.find((n) => n.namespace === ns);
      if (!fixture) throw err('E_FAILED', `Unknown settings namespace: ${ns}`);
      const parsed = parseNamespace(fixture)!;
      const layers = mockLayers(fixture, pi);
      const layer = String(cmd.layer ?? 'effective');
      if (layer === 'global') return { values: layers.global, diagnostics: [] };
      if (layer === 'project') return { values: layers.project, diagnostics: [] };
      const resolved = resolve(parsed, layers);
      return { values: resolved.values, diagnostics: resolved.diagnostics };
    }
    case 'set_extension_setting':
    case 'unset_extension_setting': {
      const ns = String(cmd.namespace ?? '');
      const fixture = namespaces.find((n) => n.namespace === ns);
      if (!fixture) throw err('E_FAILED', `Unknown settings namespace: ${ns}`);
      const layer = String(cmd.scope ?? 'global') === 'project' ? 'project' : 'global';
      const stored = (fixture[layer] ??= {});
      const key = String(cmd.key ?? '');
      if (cmd.kind === 'set_extension_setting' && cmd.value !== undefined && cmd.value !== null) {
        setPath(stored as Record<string, unknown>, key, cmd.value);
      } else {
        removePath(stored as Record<string, unknown>, key);
      }
      // Keep the pi-facing view coherent for fixtures that seed extensionSettings.
      return null;
    }
    case 'extensions': {
      const list = (fixtures.extensions ??= []);
      return { extensions: list.map((e) => ({ id: e.id, kind: e.kind ?? 'extension', enabled: e.enabled !== false, loaded: e.loaded !== false && e.enabled !== false, ...(e.path ? { path: e.path } : {}), ...(e.removable !== undefined ? { removable: e.removable } : {}) })) };
    }
    case 'reload_extensions': {
      // [owner-fix-8] the host discovers its packages again: the mock answers
      // with the current list (a test pushes the new package before the call).
      const list = (fixtures.extensions ??= []);
      return { extensions: list.map((e) => ({ id: e.id, kind: e.kind ?? 'extension', enabled: e.enabled !== false, loaded: e.loaded !== false && e.enabled !== false, ...(e.path ? { path: e.path } : {}), ...(e.removable !== undefined ? { removable: e.removable } : {}) })) };
    }
    case 'set_extension_enabled': {
      const list = (fixtures.extensions ??= []);
      const row = list.find((e) => e.id === cmd.id);
      if (!row) throw err('E_NOT_FOUND', `No extension ${String(cmd.id)}`);
      row.enabled = cmd.enabled === true;
      row.loaded = cmd.enabled === true;
      return null;
    }
    case 'auth_providers':
      return {
        providers: (fixtures.providers ?? []).map((p) => ({
          id: p.id,
          name: p.name ?? p.id,
          oauth: p.oauth ?? false,
          configured: p.configured ?? false,
          ...(p.stored ? { stored: p.stored } : {}),
          env: p.env ?? [],
        })),
      };
    case 'login':
      throw err('E_UNSUPPORTED', 'OAuth sign-in opens from a chat session; set an API key instead');
    case 'logout': {
      const row = (fixtures.providers ?? []).find((p) => p.id === cmd.provider);
      if (row) {
        row.configured = false;
        delete row.stored;
      }
      return null;
    }
    case 'set_api_key': {
      const row = (fixtures.providers ?? []).find((p) => p.id === cmd.provider);
      if (row) {
        row.configured = true;
        row.stored = 'api_key';
      }
      return null;
    }
    case 'get_shortcuts':
      return { shortcuts: [] };
    default:
      throw err('E_SCHEMA', `Unknown host_cmd: ${cmd.kind}`);
  }
}

const BUILTIN: Record<string, MockOp> = {
  host_info: (_a, h) => ({ name: h.spec.name, version: '2.0.0', proto: '2.0.0', tools: ['oino', 'unipi-kanboard'] }),
  // host.md §7.2 / app spec §3.14 (WP-A5, Z-19): the draft's first prompt
  // creates the session; its card goes live at once.
  session_new: (a, h) => {
    const id = `s-new-${(h.spec.cards?.length ?? 0) + (h.spec.archived?.length ?? 0) + 1}-${Math.random().toString(36).slice(2, 8)}`;
    const card: SessionCard = {
      session_id: id,
      name: null,
      // Like the real host: no cwd = "No workspace" = the chat's own scratch folder, never home.
      cwd: typeof a.cwd === 'string' && a.cwd ? a.cwd : `/home/you/.local/share/oino/scratch/${id}`,
      run_state: 'idle',
      updated_at: Date.now(),
      message_count: 0,
    };
    h.spec.cards = [...(h.spec.cards ?? []), card];
    h.emit({ kind: 'session_changed', card });
    return { ...card };
  },
  // host.md §7.2: a bare array of cards, newest activity first (§3.14's
  // `sessions_list{archived:true}` answers the trash instead, WP-A5).
  sessions_list: (a, h) => (a?.archived === true ? h.spec.archived ?? [] : h.spec.cards ?? []),
  // Owner fix 2026-11-03: Settings › Memory › Browse memories (mirrors unipi-host memory_browse).
  memory_browse: async (a, h) => {
    if (typeof h.spec.memories === 'function') h.spec.memories = h.spec.memories();
    const all = (h.spec.memories ?? []) as MockMemory[];
    if (h.spec.memoryDelayMs) await new Promise((r) => setTimeout(r, h.spec.memoryDelayMs));
    const project = typeof a?.project === 'string' ? a.project : undefined;
    const words = String(a?.query ?? '').toLowerCase().split(/\s+/).filter(Boolean);
    const scoped = all
      .filter((m) => !project || m.project === project)
      .filter((m) => words.every((w) => `${m.title}\n${m.project}\n${m.body}`.toLowerCase().includes(w)))
      .sort((x, y) => y.updated_at - x.updated_at);
    const limit = Math.min(100, Math.max(1, Number(a?.limit ?? 50)));
    const start = Number.parseInt(String(a?.cursor ?? '0'), 10) || 0;
    const items = scoped.slice(start, start + limit).map((m) => ({ ...m, body: m.body.slice(0, 4000) }));
    const counts = new Map<string, number>();
    for (const m of all) counts.set(m.project, (counts.get(m.project) ?? 0) + 1);
    const projects = a?.cursor ? [] : [...counts].map(([name, count]) => ({ name, count })).sort((x, y) => y.count - x.count || x.name.localeCompare(y.name));
    const next = start + items.length;
    return { items, total: scoped.length, ...(next < scoped.length ? { next_cursor: String(next) } : {}), projects, store: all.length > 0 };
  },
  // Owner fix 2026-11-04: Settings › Usage (mirrors unipi-host usage_stats).
  usage_stats: async (a, h) => {
    if (h.spec.usageDelayMs) await new Promise((r) => setTimeout(r, h.spec.usageDelayMs));
    if (h.spec.usage === null || !h.spec.usage) throw err('E_UNSUPPORTED', 'usage_stats needs a newer host.');
    return h.spec.usage(Math.min(366, Math.max(1, Number(a?.days ?? 30))));
  },
  // Owner fix 2026-11-03: the host-side paged session search (mirrors unipi_proto::session_search_page).
  session_search: (a, h) => {
    const list = (a?.archived === true ? h.spec.archived ?? [] : h.spec.cards ?? []) as SessionCard[];
    const words = String(a?.query ?? '').toLowerCase().split(/\s+/).filter(Boolean);
    const hits = list
      .filter((c) => words.every((w) => `${c.name ?? ''} ${c.cwd ?? ''} ${c.session_id} ${c.model ?? ''}`.toLowerCase().includes(w)))
      .sort((x, y) => (y.last_activity ?? y.updated_at ?? 0) - (x.last_activity ?? x.updated_at ?? 0));
    const limit = Math.min(200, Math.max(1, Number(a?.limit ?? 50)));
    const start = Number.parseInt(String(a?.cursor ?? '0'), 10) || 0;
    const cards = hits.slice(start, start + limit);
    const next = start + cards.length;
    return { cards: cards.map((c) => ({ ...c })), total: hits.length, ...(next < hits.length ? { next_cursor: String(next) } : {}) };
  },
  // App spec §3.14 (WP-A5): rename sets the card's name and the hub stream
  // carries it (the app locks the title separately via session_meta_set).
  session_rename: (a, h) => {
    const card = h.spec.cards?.find((c) => c.session_id === a.session) ?? h.spec.archived?.find((c) => c.session_id === a.session);
    if (!card) throw err('E_NOT_FOUND', `No session ${String(a.session)}`);
    if (typeof a.name !== 'string' || !a.name.trim()) throw err('E_SCHEMA', 'A name is required');
    (card as Record<string, unknown>).name = a.name.trim();
    if (h.spec.cards?.includes(card)) h.emit({ kind: 'session_changed', card: { ...card } });
    return { ...card };
  },
  // "Move to a workspace…": only a scratch chat moves, only to a non-scratch folder (one way).
  session_move: (a, h) => {
    const card = h.spec.cards?.find((c) => c.session_id === a.session);
    if (!card) throw err('E_NOT_FOUND', `No session ${String(a.session)}`);
    const scratch = (p: unknown) => typeof p === 'string' && /[\\/]oino[\\/]scratch[\\/][^\\/]+[\\/]?$/.test(p);
    // An older chat (cloned from UniPi) that ran in the bare home moves too — never copied from.
    const home = (p: unknown) => typeof p === 'string' && /^\/home\/[^/]+\/?$|^\/Users\/[^/]+\/?$|^\/root\/?$/.test(p);
    if (!scratch(card.cwd) && !home(card.cwd)) throw err('E_INTERNAL', 'Only a chat without a workspace can move to one — this chat already has a workspace');
    if (typeof a.cwd !== 'string' || !a.cwd.startsWith('/') || /[\\/]oino[\\/]scratch([\\/]|$)/.test(a.cwd)) throw err('E_INTERNAL', 'Pick a workspace folder outside UniPi\'s scratch folders');
    if (home(a.cwd)) throw err('E_INTERNAL', 'Pick a workspace folder, not your whole home folder');
    if (home(card.cwd) && (a.copy_files === true || a.copyFiles === true)) throw err('E_INTERNAL', 'This chat ran in your home folder — its files can\'t be copied; move it without copying');
    (card as Record<string, unknown>).cwd = a.cwd;
    h.emit({ kind: 'session_changed', card: { ...card } });
    return { card: { ...card }, copied: 0, skipped: 0 };
  },
  // App spec §3.14 (WP-A4): patch the card, tell the hub stream, return the card.
  session_meta_set: (a, h) => {
    const card = h.spec.cards?.find((c) => c.session_id === a.session);
    if (!card) throw err('E_NOT_FOUND', `No session ${String(a.session)}`);
    for (const key of ['pinned', 'unread', 'title_locked'] as const) {
      if (typeof a[key] === 'boolean') (card as Record<string, unknown>)[key] = a[key];
    }
    // Like the real host, the card goes to *this* host's connections only.
    h.emit({ kind: 'session_changed', card: { ...card } });
    return { ...card };
  },
  // Owner 2026-11-06: host-synced composer drafts; the card's `draft_at` follows.
  draft_get: (a, h) => ({ draft: h.drafts.get(String(a.session)) ?? null }),
  draft_set: (a, h) => {
    const id = String(a.session);
    const card = h.spec.cards?.find((c) => c.session_id === id);
    if (a.draft && typeof a.draft === 'object') h.drafts.set(id, a.draft);
    else h.drafts.delete(id);
    if (!card) return { session_id: id };
    if (h.drafts.has(id)) (card as Record<string, unknown>).draft_at = Date.now();
    else delete (card as Record<string, unknown>).draft_at;
    h.emit({ kind: 'session_changed', card: { ...card } });
    return { ...card };
  },
  // App spec §3.14 / host.md §4.2 (WP-A5, Z-13): archive moves the card to the
  // host's trash; restore puts it back; delete/trash_empty unlink for good.
  session_archive: (a, h) => {
    const card = h.spec.cards?.find((c) => c.session_id === a.session);
    if (!card) throw err('E_NOT_FOUND', `No session ${String(a.session)}`);
    h.spec.cards = (h.spec.cards ?? []).filter((c) => c.session_id !== card.session_id);
    h.spec.archived = [card, ...(h.spec.archived ?? [])];
    h.emit({ kind: 'session_removed', session: card.session_id });
    return { ...card };
  },
  session_restore: (a, h) => {
    const card = h.spec.archived?.find((c) => c.session_id === a.session);
    if (!card) throw err('E_NOT_FOUND', `No archived session ${String(a.session)}`);
    h.spec.archived = (h.spec.archived ?? []).filter((c) => c.session_id !== card.session_id);
    h.spec.cards = [...(h.spec.cards ?? []), card];
    h.emit({ kind: 'session_changed', card: { ...card } });
    return { ...card };
  },
  session_delete: (a, h) => {
    const card = h.spec.archived?.find((c) => c.session_id === a.session);
    if (!card) throw err('E_NOT_FOUND', `No archived session ${String(a.session)}`);
    h.spec.archived = (h.spec.archived ?? []).filter((c) => c.session_id !== card.session_id);
    return null;
  },
  trash_empty: (_a, h) => {
    const deleted = h.spec.archived?.length ?? 0;
    h.spec.archived = [];
    return { deleted };
  },
  // [ux-sessions] (owner fix 2026-11-03): batch archive / restore — one op, events in
  // chunks of 200 (mirrors `SessionHub::archive_many` / `restore_many`).
  sessions_archive: (a, h) => {
    const ids = new Set((a?.sessions as string[] | undefined) ?? []);
    const moving = (h.spec.cards ?? []).filter((c) => ids.has(c.session_id) && c.run_state !== 'working' && !c.needs_you);
    const done = new Set(moving.map((c) => c.session_id));
    h.spec.cards = (h.spec.cards ?? []).filter((c) => !done.has(c.session_id));
    h.spec.archived = [...moving, ...(h.spec.archived ?? [])];
    const doneIds = [...done];
    for (let i = 0; i < doneIds.length; i += 200) h.emit({ kind: 'sessions_removed', sessions: doneIds.slice(i, i + 200), archived: true });
    const failed = [...ids].filter((id) => !done.has(id)).map((session) => ({ session, message: 'the session is running' }));
    return { done: doneIds, ...(failed.length ? { failed } : {}) };
  },
  sessions_restore: (a, h) => {
    const ids = new Set((a?.sessions as string[] | undefined) ?? []);
    const back = (h.spec.archived ?? []).filter((c) => ids.has(c.session_id));
    h.spec.archived = (h.spec.archived ?? []).filter((c) => !ids.has(c.session_id));
    h.spec.cards = [...(h.spec.cards ?? []), ...back];
    for (let i = 0; i < back.length; i += 200) h.emit({ kind: 'sessions_changed', cards: back.slice(i, i + 200).map((c) => ({ ...c })) });
    return { done: back.map((c) => c.session_id) };
  },
  // [ux-sessions]: the sidebar search — title (name, else folder) + first user message
  // (`first_message` on mock cards), live / archived / all, a few host-side filters.
  sessions_query: (a, h) => {
    const scope = (a?.scope as string | undefined) ?? 'live';
    const rows: Array<{ card: SessionCard; archived: boolean }> = [
      ...(scope !== 'archived' ? (h.spec.cards ?? []).map((card) => ({ card, archived: false })) : []),
      ...(scope !== 'live' ? (h.spec.archived ?? []).map((card) => ({ card, archived: true })) : []),
    ];
    const words = String(a?.query ?? '').toLowerCase().split(/\s+/).filter(Boolean);
    const f = (a?.filters ?? {}) as { workspaces?: string[]; automation?: string[] };
    const title = (c: SessionCard) => (c.name?.trim() || (c.cwd ? (c.cwd.replace(/[\\/]+$/, '').split(/[\\/]/).pop() ?? '') : '') || 'New chat').toLowerCase();
    const first = (c: SessionCard) => String((c as { first_message?: unknown }).first_message ?? '').toLowerCase();
    const hits = rows
      .filter(({ card }) => words.every((w) => title(card).includes(w) || first(card).includes(w)))
      .filter(({ card }) => !f.workspaces?.length || f.workspaces.includes(card.cwd ?? ''))
      .filter(({ card }) => !f.automation?.length || f.automation.includes(card.source === 'cron' ? 'cron' : 'manual'))
      .sort((x, y) => (y.card.last_activity ?? y.card.updated_at ?? 0) - (x.card.last_activity ?? x.card.updated_at ?? 0));
    const limit = Math.min(100, Math.max(1, Number(a?.limit ?? 50)));
    const start = Number.parseInt(String(a?.cursor ?? '0'), 10) || 0;
    const page = hits.slice(start, start + limit);
    const next = start + page.length;
    return {
      hits: page.map(({ card, archived }) => ({ card: { ...card }, ...(archived ? { archived } : {}) })),
      total: hits.length,
      ...(next < hits.length ? { next_cursor: String(next) } : {}),
    };
  },
  // WP-A19 (G-12): `notify_register` on the host seam records the endpoint.
  notify_register: () => ({ registered: true }),
  fs_roots: (_a, h) => ({ roots: Object.keys(h.spec.fs ?? {}) }),
  // The UI's `lsp_list` command → the wire `lsp_servers` reply.
  lsp_list: (_a, h) => mockLspServers(h.spec.lsp, Object.keys(h.spec.fs ?? {})[0]),
  fs_list: (a, h) => {
    const path = String(a.path ?? '');
    const node = walk(h.spec.fs, path);
    if (node?.kind !== 'dir') throw err('E_NOT_FOUND', `No folder ${path}`);
    const entries = Object.entries(node.children).map(([name, child]) => ({
      name,
      kind: child.kind,
      size: child.kind === 'file' ? child.text.length : 0,
      mtime: child.kind === 'file' ? (child.mtime ?? 0) : 0,
      ...(child.git ? { git: child.git } : {}),
      ...(child.ignored ? { ignored: true } : {}),
    }));
    return { entries, more: false };
  },
  fs_stat: (a, h) => {
    const node = walk(h.spec.fs, String(a.path ?? ''));
    if (!node) throw err('E_NOT_FOUND', 'Not found');
    return { kind: node.kind, size: node.kind === 'file' ? node.text.length : 0, mtime: 0 };
  },
  fs_read: (a, h) => {
    const path = String(a.path ?? '');
    const node = walk(h.spec.fs, path);
    if (node?.kind !== 'file') throw err('E_NOT_FOUND', 'Not found');
    // Binary fixtures stay binary (the editor routes them to the previewer).
    if (/\.(png|jpe?g|gif|webp|pdf|mp4|mp3)$/i.test(path) && a.as_text !== true) throw err('E_BINARY', `${path.slice(path.lastIndexOf('/') + 1)} isn't text`);
    return { size: node.text.length, hash: `h${node.text.length}`, mtime: node.mtime ?? 0, truncated: false, text: node.text };
  },
  fs_write: (a, h) => {
    const path = String(a.path ?? '');
    const text = String(a.text ?? '');
    const dir = walk(h.spec.fs, path.slice(0, path.lastIndexOf('/')));
    const name = path.slice(path.lastIndexOf('/') + 1);
    if (!dir || dir.kind !== 'dir') throw err('E_NOT_FOUND', `No folder ${path}`);
    if (!(name in dir.children) && a.create !== true) throw err('E_NOT_FOUND', 'Not found');
    dir.children[name] = { kind: 'file', text, mtime: Date.now(), git: 'M' };
    return { hash: `h${text.length}`, mtime: Date.now() };
  },
  fs_mkdir: (a, h) => {
    const path = String(a.path ?? '');
    const dir = walk(h.spec.fs, path.slice(0, path.lastIndexOf('/')));
    const name = path.slice(path.lastIndexOf('/') + 1);
    if (!dir || dir.kind !== 'dir') throw err('E_NOT_FOUND', `No folder ${path}`);
    dir.children[name] = { kind: 'dir', children: {}, git: '?' };
    return null;
  },
  fs_rename: (a, h) => {
    const from = String(a.from ?? '');
    const to = String(a.to ?? '');
    const node = walk(h.spec.fs, from);
    const dir = walk(h.spec.fs, from.slice(0, from.lastIndexOf('/')));
    const name = from.slice(from.lastIndexOf('/') + 1);
    const toDir = walk(h.spec.fs, to.slice(0, to.lastIndexOf('/')));
    const toName = to.slice(to.lastIndexOf('/') + 1);
    if (!node || !dir || dir.kind !== 'dir' || !(name in dir.children)) throw err('E_NOT_FOUND', 'Not found');
    if (!toDir || toDir.kind !== 'dir') throw err('E_NOT_FOUND', `No folder ${to}`);
    if (toName in toDir.children) throw err('E_CONFLICT', `${toName} already exists`);
    delete dir.children[name];
    toDir.children[toName] = node;
    return null;
  },
  fs_delete: (a, h) => {
    const path = String(a.path ?? '');
    if (a.confirm !== true) throw err('E_CONFIRM', 'Pass confirm: true');
    const dir = walk(h.spec.fs, path.slice(0, path.lastIndexOf('/')));
    const name = path.slice(path.lastIndexOf('/') + 1);
    if (!dir || dir.kind !== 'dir' || !(name in dir.children)) throw err('E_NOT_FOUND', 'Not found');
    delete dir.children[name];
    return null;
  },
  fs_import: (a, h) => {
    const to = String(a.to ?? '');
    const blob = h.blobs.get(String(a.sha256 ?? ''));
    const dir = walk(h.spec.fs, to.slice(0, to.lastIndexOf('/')));
    const name = to.slice(to.lastIndexOf('/') + 1);
    if (!blob) throw err('E_NOT_FOUND', 'No such blob');
    if (!dir || dir.kind !== 'dir') throw err('E_NOT_FOUND', `No folder ${to}`);
    if (name in dir.children && a.overwrite !== true) throw err('E_CONFLICT', `${name} already exists`);
    dir.children[name] = { kind: 'file', text: blob.text, mtime: Date.now(), git: 'A' };
    return null;
  },
  fs_search: (a, h) => {
    const root = String(a.root ?? '');
    const query = String(a.query ?? '').toLowerCase();
    const kind = a.kind === 'content' ? 'content' : 'name';
    const results: Array<{ path: string; line?: number; preview?: string }> = [];
    const scan = (path: string, node: MockFsNode) => {
      if (results.length >= 200) return;
      if (node.kind === 'dir') {
        for (const [name, child] of Object.entries(node.children)) scan(`${path}/${name}`, child);
        return;
      }
      if (kind === 'name') {
        if (path.toLowerCase().includes(query)) results.push({ path });
      } else {
        node.text.split('\n').forEach((line, at) => {
          if (results.length < 200 && line.toLowerCase().includes(query)) results.push({ path, line: at + 1, preview: line.trim().slice(0, 160) });
        });
      }
    };
    const base = Object.keys(h.spec.fs ?? {}).find((r) => root === r || root.startsWith(`${r}/`));
    const node = base !== undefined ? walk(h.spec.fs, root) : undefined;
    if (node) scan(root, node);
    return { results, more: false };
  },
  blob_put_bytes: (a, h) => {
    const bytes = Array.isArray(a.bytes) ? (a.bytes as number[]) : [];
    const text = bytes.map((b) => String.fromCharCode(b)).join('');
    const sha256 = `b${bytes.length}-${text.slice(0, 8)}`;
    h.blobs.set(sha256, { text });
    return { sha256, size: bytes.length, deduped: false, path: `/tmp/${sha256}` };
  },
  blob_get_bytes: (a, h) => {
    if (a.path) {
      const path = String(a.path);
      const node = walk(h.spec.fs, path);
      if (node?.kind !== 'file') throw err('E_NOT_FOUND', 'Not found');
      // A real 1×1 transparent PNG so image previews have something to render.
      if (path.endsWith('.png')) return { mime: 'image/png', bytes: TINY_PNG_BYTES };
      return { mime: mimeOf(path), bytes: Array.from(new TextEncoder().encode(node.text)) };
    }
    const blob = h.blobs.get(String(a.sha256 ?? ''));
    if (!blob) throw err('E_NOT_FOUND', 'No such blob');
    return { mime: 'application/octet-stream', bytes: Array.from(new TextEncoder().encode(blob.text)) };
  },
  // WP-A15: the whole `git` op family answers from the one mock repo.
  git_op: (a, h) => mockGitOp(a.cmd as { action: string }, h.spec.git ?? {}),
  // host.md §9.9 (WP-A24): the host's `config.json`, mutations in place.
  host_settings_get: (_a, h) => h.spec.settings?.host ?? {},
  host_settings_set: (a, h) => {
    const settings = (h.spec.settings ??= {});
    const host = (settings.host ??= {});
    setDotted(host, String(a.key ?? ''), a.value);
    return host;
  },
  // App spec §3.14 (WP-A24): the hub-level `_oino/*` subset over the fixtures.
  host_cmd: (a, h) => mockHostCmd(a.cmd as { kind: string } & Record<string, unknown>, h),
  // WP-A22 (app.md §3.14): one `unipi-kanboard` process per call, §5.5 outcome.
  board_cli: (a, h) => h.board.cli((a.args as string[] | undefined) ?? [], (a.project as string | undefined) ?? undefined),
  // WP-A22: the refcounted file watcher (writes then answer `board_changed`).
  board_watch: (a, h) => ({ watching: h.setBoardWatching(a.on !== false) }),
  // WP-A15 (Z-98): the refcounted watcher; the root `git_changed` events carry.
  git_watch: (a, h) => {
    const g = h.spec.git ?? {};
    h.watch(String(a.root ?? g.root ?? ''), a.on !== false);
    return { root: g.repo === false ? null : (g.root ?? a.root ?? null) };
  },
  cron_list: (_a, h) => ({ jobs: h.spec.cron ?? [], paused: false }),
  cron_status: () => ({ paused: false, concurrency: 1, jobs: [] }),
  ports_list: () => ({ ports: [], any: true, allowed: [] }),
  push_list: (_a, h) => ({ enabled: true, host_name: h.spec.name, pushes: [] }),
  feature_requests: () => ({ requests: [] }),
  devices_list: () => ({ devices: [] }),
};

const APP_BUILTIN: Record<string, MockOp> = {
  forward_list: () => [],
  local_requests: () => [],
  local_host: () => ({ installed: false }),
  push_save_as: () => null,
  push_open: () => null,
  // WP-A24 (D-20): the H9 lifetime commands, in memory for captures/tests.
  close_action_state: (_a, h) => h.world.appState.closeAction ?? 'ask',
  close_action_set: (a, h) => (h.world.appState.closeAction = a.action === 'tray' || a.action === 'quit' ? a.action : 'ask'),
  autostart_state: (_a, h) => h.world.appState.autostart ?? false,
  autostart_set: (a, h) => (h.world.appState.autostart = a.enabled === true),
  // WP-A19 (G-12): the host's notify path answers — the phone push settings
  // and `notify_register` fan-out work against the mock too.
  notify_config: () => ({ server: 'https://ntfy.example', topic: 'unipi-test', key: 'a2V5a2V5a2V5a2V5a2V5a2V5a2V5a2V5a2V5', app_detail: 'minimal' }),
  notify_register: (_a, h) => {
    h.emit({ kind: 'host_notice', level: 'info', text: 'Phone push registered' });
    return { registered: true };
  },
  notify_test: () => null,
};

export class MockHost implements HostClient {
  readonly hostId: string;
  readonly conn;
  private readonly setConn;
  /** In-memory blob store behind `blob_put_bytes`/`blob_get_bytes` (uploads, previews). */
  readonly blobs = new Map<string, { text: string }>();
  /** Host-synced composer drafts (`draft_get` / `draft_set`) by session id. */
  readonly drafts = new Map<string, unknown>();
  /** Repo roots a `git_watch{on:true}` asked for (off drops them), like the host's refcount. */
  readonly watched = new Set<string>();
  /** The in-memory board behind `board_cli`; every write emits `board_changed`. */
  readonly board: MockBoard;
  /** `board_watch{on}`: whether the UI has the watcher on (last call wins). */
  boardWatching = false;
  /** WP-A14: the model + thinking level a mock session has switched to, and the
   *  top event seq per `host/session` — a command that emits an event must not
   *  reuse a seq the store already applied. */
  readonly picked = new Map<string, { provider: string; id: string; level: string }>();
  private readonly seqTop = new Map<string, number>();
  /** WP-A25: the registry rev per session, for the panels a typed `/command` opens. */
  private readonly uiRev = new Map<string, number>();

  /** The next event seq for a session (its snapshot and any script steps included). */
  private topSeq(key: string, session: MockSession): number {
    const known = this.seqTop.get(key);
    if (known !== undefined) return known;
    let top = session.snapshot.seq;
    for (const step of session.events ?? []) if ('dto' in step) top = Math.max(top, step.dto.seq);
    this.seqTop.set(key, top);
    return top;
  }

  /** Pushes one live event to the session's open streams with a fresh seq. */
  private emitSession(key: string, session: MockSession, event: Record<string, unknown>): void {
    const seq = this.topSeq(key, session) + 1;
    this.seqTop.set(key, seq);
    const sessionId = session.snapshot.session_id;
    this.world.push(this.hostId, sessionId, [{ session: sessionId, seq, event } as SessionEventDto]);
  }

  /** The model + thinking level a session starts on (its snapshot state). */
  private levelOf(key: string, session: MockSession): { provider: string; id: string; level: string } {
    const known = this.picked.get(key);
    if (known) return known;
    const state = (session.snapshot.state ?? {}) as { model?: { provider?: unknown; id?: unknown }; thinkingLevel?: unknown };
    const init = {
      provider: typeof state.model?.provider === 'string' ? state.model.provider : 'anthropic',
      id: typeof state.model?.id === 'string' ? state.model.id : 'claude-sonnet-4-5',
      level: typeof state.thinkingLevel === 'string' ? state.thinkingLevel : 'medium',
    };
    this.picked.set(key, init);
    return init;
  }
  constructor(
    readonly spec: MockHostSpec,
    /** The shared world (recorded log, event sinks): public for the builtin ops. */
    readonly world: MockWorld,
  ) {
    this.hostId = spec.id;
    this.board = createMockBoard(spec.board ?? [], (project) => this.emit({ kind: 'board_changed', project }));
    const [conn, setConn] = createSignal<HostConn>(spec.conn ?? { state: 'connected' });
    this.conn = conn;
    this.setConn = setConn;
  }

  /** `board_watch{on}`: records the state the UI asked for and answers it. */
  setBoardWatching(on: boolean): boolean {
    this.boardWatching = on;
    return this.boardWatching;
  }

  /** Scenario/test control: flip the connection (e.g. go offline mid-capture). */
  setConnection(next: HostConn): void {
    this.setConn(next);
  }

  async retry(): Promise<void> {
    this.world.record({ host: this.hostId, kind: 'request', op: 'host_info' });
  }

  async request<T = unknown>(op: string, args: Record<string, unknown> = {}): Promise<T> {
    if (this.hostId === APP_HOST) return this.world.appRequest(op, args) as Promise<T>;
    const handler: MockOp | undefined = this.spec.ops?.[op] ?? BUILTIN[op];
    this.world.record({ host: this.hostId, kind: 'request', op, args, ...(handler === undefined ? { unhandled: true as const } : {}) });
    if (this.conn().state === 'offline') {
      throw err('E_TRANSPORT', this.conn().message ?? `Can't reach ${this.spec.name}`, this.conn().hint);
    }
    return (handler ? await handler(args, this) : null) as T;
  }

  openSession(req: Parameters<HostClient['openSession']>[0], sink: Parameters<HostClient['openSession']>[1]): SessionStream {
    this.world.record({ host: this.hostId, kind: 'open', op: `session_${req.kind}`, args: req });
    const timers: ReturnType<typeof setTimeout>[] = [];
    let open = true;
    const fromNew = (req.kind === 'new' || req.kind === 'superagent') && !req.session ? this.spec.newSessionFrom : undefined;
    const id = req.session ?? fromNew ?? '';
    const session = id ? this.spec.sessions?.[id] : undefined;
    const key = `${this.hostId}/${id}`;
    const end = (error?: Err) => {
      if (!open) return;
      open = false;
      timers.forEach(clearTimeout);
      this.world.streams.get(key)?.delete(handle);
      sink.closed(error as never);
    };
    const handle = { push: (items: SessionEventDto[]) => open && sink.events(items), drop: (error?: Err) => end(error), resync: () => open && sink.resync?.('oversize frame') };
    queueMicrotask(() => {
      if (!open) return;
      if (this.conn().state === 'offline') return end(err('E_TRANSPORT', `Can't reach ${this.spec.name}`));
      if (!session) return end(err('E_NOT_FOUND', `No session ${req.session ?? ''}`));
      const clock = this.world.clock(key);
      if (this.world.isDown(key)) return end(err('E_TRANSPORT', 'The host stopped answering'));
      (this.world.streams.get(key) ?? this.world.streams.set(key, new Set()).get(key)!).add(handle);
      const script = session.events ?? [];
      const speed = this.world.scenario.speed ?? 0;
      const elapsed = speed ? clock.elapsed() * speed : Infinity;
      const since = req.sinceSeq ?? 0;
      const dueEvents = script.flatMap((s) => ('dto' in s && s.at <= elapsed && s.dto.seq > since ? [s.dto] : []));
      if (since > 0) {
        sink.snapshot({ session_id: id, reset: false, seq: Math.max(since, ...dueEvents.map((e) => e.seq)), replay: dueEvents } as SessionSnapshot);
      } else {
        sink.snapshot(session.snapshot);
        if (dueEvents.length) sink.events(dueEvents);
      }
      script.forEach((step, i) => {
        if (!('drop' in step)) {
          if (step.at > elapsed) timers.push(setTimeout(() => open && sink.events([step.dto]), (step.at - elapsed) / speed));
          return;
        }
        const fire = () => {
          if (!open || this.world.fired(key, i)) return;
          this.world.markDown(key, (step.drop.down ?? 0) / (speed || 1));
          end(err(step.drop.code ?? 'E_TRANSPORT', step.drop.message ?? 'The connection dropped'));
        };
        // Speed 0 settles at once: a drop that has not fired yet takes the stream down right after the replay.
        if (step.at <= elapsed) queueMicrotask(fire);
        else timers.push(setTimeout(fire, (step.at - elapsed) / speed));
      });
    });
    return {
      cmd: async <T,>(cmd: RpcCommand): Promise<T> => {
        this.world.record({ host: this.hostId, kind: 'cmd', op: cmd.type, args: cmd });
        if (!open) throw err('E_TRANSPORT', 'The session stream closed');
        const handler = session?.cmd?.[cmd.type];
        if (handler) return (await handler(cmd, this)) as T;
        if (cmd.type === 'get_commands') return { commands: session?.snapshot.commands ?? [] } as T;
        // WP-A18 (Z-79): whole-session search over the mock's entries.
        if (cmd.type === 'search') return { hits: mockSearch(String(cmd.query ?? ''), session) } as T;
        // WP-A18 (G-7): the extension shortcuts the real unipi extensions register.
        if (cmd.type === '_oino/get_shortcuts') return { shortcuts: MOCK_SHORTCUTS } as T;
        // The sticky topics the scenario seeded (subscribing merges them into the store).
        if (cmd.type === '_oino/topics') return { topics: session?.snapshot.topics ?? [] } as T;
        if (cmd.type === '_oino/render') {
          const spec = this.spec.renderers?.[`${String(cmd.target)}/${String(cmd.key)}`];
          if (spec && typeof spec === 'object' && 'never' in (spec as Record<string, unknown>)) return new Promise<never>(() => {});
          return { node: spec === undefined ? null : spec } as T;
        }
        if (cmd.type === 'get_entries' && session?.entries) {
          const since = typeof cmd.since === 'string' ? cmd.since : undefined;
          const all = session.entries.entries as Array<{ id?: string }>;
          const at = since ? all.findIndex((e) => e.id === since) : -1;
          return { entries: since ? all.slice(at + 1) : all, leafId: session.entries.leafId } as T;
        }
        // WP-A25: a typed `/command` is the app's "Manage…" route into an
        // extension panel — the scenario says which panel it opens.
        if (cmd.type === 'prompt' || cmd.type === 'steer' || cmd.type === 'follow_up') {
          const m = /^\/(\S+)/.exec(String(cmd.message ?? '').trim());
          const panel = m ? this.spec.panels?.[m[1]!] : undefined;
          if (panel && session) {
            const base = session.snapshot.ui as { rev?: number } | undefined;
            const rev = (this.uiRev.get(key) ?? Number(base?.rev ?? 0)) + 1;
            this.uiRev.set(key, rev);
            this.emitSession(key, session, {
              type: 'oino_ui_patch',
              scope: 'session',
              rev,
              ops: [
                {
                  op: 'put',
                  entry: { id: panel.id, extensionId: panel.extensionId ?? 'extension', scope: 'session', slot: 'overlay', order: 0, seq: 100, rev: 1, title: panel.title, view: { kind: 'tree', root: panel.root } },
                },
              ],
            });
          }
          return {} as T;
        }
        // WP-A25: the session-level settings / auth / export commands the app's
        // built-in routing and the picker's cycle set use. Settings read and
        // write the scenario's pi fixture, so a round trip is observable.
        if (cmd.type === '_oino/get_settings') {
          const settings = (this.spec.settings ??= {});
          const pi = (settings.pi ??= {});
          return (cmd.key === undefined ? { settings: pi } : { value: getDotted(pi, String(cmd.key)) }) as T;
        }
        if (cmd.type === '_oino/set_setting') {
          const settings = (this.spec.settings ??= {});
          const pi = (settings.pi ??= {});
          if (cmd.value === undefined || cmd.value === null) removePath(pi, String(cmd.key));
          else setDotted(pi, String(cmd.key), cmd.value);
          return null as T;
        }
        if (cmd.type === '_oino/export_jsonl') {
          const file = String((session?.snapshot.state as { sessionFile?: string } | undefined)?.sessionFile ?? '');
          return { path: file || `/home/you/.oino/sessions/${key}.jsonl` } as T;
        }
        if (cmd.type === '_oino/import_jsonl') return { cancelled: false } as T;
        if (cmd.type === '_oino/login') {
          // The real provider asks its questions on this session (U10 docks them);
          // the capture shows a URL-carrying dialog so the app opens the browser.
          if (session) {
            this.emitSession(key, session, {
              type: 'extension_ui_request',
              id: `login-${String(cmd.provider)}`,
              method: 'input',
              title: 'Sign in',
              message: `Open ${MOCK_OAUTH_URL} and paste the code`,
              placeholder: 'code',
            });
          }
          return null as T;
        }
        if (cmd.type === '_oino/logout' || cmd.type === '_oino/set_api_key') return null as T;
        // WP-A14: the model / thinking commands answer from the catalogue and
        // push the event the real host emits, so the chips follow a switch.
        if (cmd.type === 'get_available_models') return { models: MOCK_MODELS } as T;
        if (cmd.type === 'get_available_thinking_levels') {
          const model = session?.snapshot.state as { model?: { provider?: string; id?: string } } | undefined;
          const row = model?.model ? mockModelFor(String(model.model.provider ?? ''), String(model.model.id ?? '')) : undefined;
          return { levels: mockLevelsFor(row) } as T;
        }
        if (cmd.type === 'set_model' && session) {
          const row = mockModelFor(String(cmd.provider ?? ''), String(cmd.modelId ?? ''));
          if (!row) throw err('E_NOT_FOUND', 'Model not found');
          const previous = this.levelOf(key, session);
          this.picked.set(key, { provider: row.provider, id: row.id, level: previous.level });
          this.emitSession(key, session, { type: 'model_changed', model: row, source: 'set' });
          return row as T;
        }
        if (cmd.type === 'set_thinking_level' && session) {
          const level = String(cmd.level ?? '');
          const current = this.levelOf(key, session);
          this.picked.set(key, { ...current, level });
          this.emitSession(key, session, { type: 'thinking_level_changed', level });
          return null as T;
        }
        if (cmd.type === 'cycle_thinking_level' && session) {
          const current = this.levelOf(key, session);
          const row = mockModelFor(current.provider, current.id);
          const levels = mockLevelsFor(row);
          const at = levels.indexOf(current.level);
          const level = levels[(at + 1) % levels.length]!;
          this.picked.set(key, { ...current, level });
          this.emitSession(key, session, { type: 'thinking_level_changed', level });
          return { level } as T;
        }
        return {} as T;
      },
      close: () => {
        open = false;
        timers.forEach(clearTimeout);
        this.world.streams.get(key)?.delete(handle);
      },
    };
  }

  openEvents(sink: (event: HubEvent) => void): EventsStream {
    this.world.record({ host: this.hostId, kind: 'open', op: 'events_open' });
    this.world.eventSinks.add(sink);
    const hostSinks = this.world.hostSinks.get(this.hostId) ?? this.world.hostSinks.set(this.hostId, new Set()).get(this.hostId)!;
    hostSinks.add(sink);
    // §7.4: retained notices first (the safe-mode banner, G-19), like the real host.
    for (const notice of this.spec.notices ?? []) {
      queueMicrotask(() => sink({ kind: 'host_notice', level: notice.level ?? 'info', text: notice.text }));
    }
    this.playHubScript();
    return {
      // §3.14: the mock clears the focused card's `unread` like the host does (Z-82).
      focus: (session) => {
        this.world.record({ host: this.hostId, kind: 'request', op: 'session_focus', args: { session } });
        const card = session ? this.spec.cards?.find((c) => c.session_id === session) : undefined;
        if (card?.unread) {
          card.unread = false;
          this.emit({ kind: 'session_changed', card: { ...card } });
        }
      },
      close: () => {
        this.world.eventSinks.delete(sink);
        hostSinks.delete(sink);
      },
    };
  }

  /** `git_watch`: remembers the watched roots (off drops, like the host's refcount). */
  watch(root: string, on: boolean): void {
    if (!root) return;
    if (on) this.watched.add(root);
    else this.watched.delete(root);
  }

  /** Test control: one host-side `git_changed` for a watched root (WP-A15, Z-98). */
  gitChanged(root = [...this.watched][0]): void {
    if (root) this.emit({ kind: 'git_changed', root });
  }

  /** Sends a hub event to this host's `openEvents` subscribers only (like the real host). */
  emit(event: HubEvent): void {
    for (const sink of this.world.hostSinks.get(this.hostId) ?? []) sink(event);
  }

  /** WP-A19: replays `spec.hub` steps once, on the world clock, from the
   *  first `events_open`. Speed 0 still uses the step delays as real
   *  milliseconds — the cold `sessions_list` reconcile settles first, then
   * the transitions arrive (the order the real host guarantees). */
  private playHubScript(): void {
    const script = this.spec.hub ?? [];
    if (script.length === 0 || this.hubPlayed) return;
    this.hubPlayed = true;
    const elapsed = Date.now() - this.world.t0;
    const speed = this.world.scenario.speed ?? 0;
    for (const step of script) {
      const delay = speed > 0 ? Math.max(0, (step.at - elapsed) / speed) : Math.max(0, step.at - elapsed);
      this.hubTimers.push(setTimeout(() => this.emit(step.event), delay));
    }
  }
  private hubPlayed = false;
  private hubTimers: ReturnType<typeof setTimeout>[] = [];

  openTerm(req: Parameters<HostClient['openTerm']>[0]): TermStream {
    this.world.record({ host: this.hostId, kind: 'open', op: 'term_open', args: req });
    return { write() {}, resize() {}, close() {} };
  }

  openLsp(req: Parameters<HostClient['openLsp']>[0], sink: Parameters<HostClient['openLsp']>[1]): LspStream {
    this.world.record({ host: this.hostId, kind: 'open', op: 'lsp_open', args: req });
    return openMockLsp(this.spec.lsp, sink, {
      record: (entry) => this.world.record({ ...entry, kind: 'lsp', op: entry.op } as Omit<MockLogEntry, 'at'>),
      hostId: this.hostId,
    });
  }
}

type StreamHandle = { push(items: SessionEventDto[]): void; drop(error?: Err): void; resync?(): void };

/** `{id, defaultKeys, description, extensionId}` — what the unipi extensions register (G-7). */
const MOCK_SHORTCUTS = [
  { id: 'unipi.permission.cycle', defaultKeys: ['alt+m'], description: 'Cycle permission mode (ask → auto → full)', extensionId: 'unipi.workflow' },
  { id: 'unipi.plan.toggle', defaultKeys: ['alt+p'], description: 'Toggle plan mode', extensionId: 'unipi.workflow' },
  { id: 'unipi.tray.open', defaultKeys: ['shift+down'], description: 'Open the work tray', extensionId: 'unipi.core' },
  { id: 'unipi.subagents.foreground', defaultKeys: ['ctrl+b'], description: 'Move foreground subagents to the background', extensionId: 'unipi.subagents' },
  { id: 'unipi.bg.clear', defaultKeys: ['ctrl+alt+c'], description: 'Clear finished background task footer notices', extensionId: 'unipi.background-tasks' },
];

/** The `search` reply: hits over message texts (the old bridge shape, Z-79 fallback). */
function mockSearch(query: string, session: MockSession | undefined): unknown[] {
  const q = query.trim().toLowerCase();
  if (!q || !session) return [];
  const hits: Array<{ id: string; role: string; snippet: string; marks: Array<[number, number]>; score: number; timestamp?: string }> = [];
  const entries = [...(session.snapshot.entries ?? []), ...((session.entries?.entries as unknown[] | undefined) ?? [])] as Array<Record<string, unknown>>;
  for (const entry of entries) {
    const message = entry.message as { role?: string; content?: unknown } | undefined;
    if (!message || typeof message.role !== 'string') continue;
    const parts = Array.isArray(message.content)
      ? (message.content as Array<{ type?: string; text?: string }>)
      : [{ type: 'text', text: typeof message.content === 'string' ? message.content : '' }];
    for (const part of parts) {
      const text = typeof part?.text === 'string' ? part.text : '';
      const at = text.toLowerCase().indexOf(q);
      if (at < 0) continue;
      const from = Math.max(0, at - 40);
      const snippet = `${from > 0 ? '…' : ''}${text.slice(from, at + q.length + 60)}`;
      const offset = snippet.toLowerCase().indexOf(q);
      hits.push({
        id: String(entry.id ?? ''),
        role: message.role,
        snippet,
        marks: [[Math.max(0, offset), Math.max(0, offset) + q.length]],
        score: 1,
        ...(typeof entry.timestamp === 'string' ? { timestamp: entry.timestamp } : {}),
      });
    }
  }
  return hits.slice(0, 60);
}

export class MockWorld {
  readonly log: MockLogEntry[] = [];
  readonly eventSinks = new Set<(event: HubEvent) => void>();
  /** Per-host event sinks: `MockHost.emit` reaches only its own host's subscribers. */
  readonly hostSinks = new Map<string, Set<(event: HubEvent) => void>>();
  /** Open session streams by `host/session` (tests push events / drop them). */
  readonly streams = new Map<string, Set<StreamHandle>>();
  /** App-level state behind the app builtin ops (WP-A24 lifetime commands). */
  readonly appState: { closeAction?: 'ask' | 'tray' | 'quit'; autostart?: boolean } = {};
  private readonly clocks = new Map<string, number>();
  private readonly down = new Map<string, number>();
  private readonly firedSteps = new Set<string>();
  private readonly clients = new Map<string, MockHost>();
  readonly t0 = Date.now();

  constructor(readonly scenario: MockScenario) {
    for (const spec of scenario.hosts) this.clients.set(spec.id, new MockHost(spec, this));
    this.clients.set(APP_HOST, new MockHost({ id: APP_HOST, name: 'App' }, this));
  }

  /** The `HostClientFactory` (`setHostClientFactory(world.client)` / `window.__UNIPI_HOST__`). */
  readonly client = (hostId: string): MockHost => {
    let c = this.clients.get(hostId);
    if (!c) {
      // An unknown id (a stale selection, a forgotten PC) is an unreachable host.
      c = new MockHost({ id: hostId, name: hostId, conn: { state: 'offline', message: 'Unknown PC' } }, this);
      this.clients.set(hostId, c);
    }
    return c;
  };

  record(entry: Omit<MockLogEntry, 'at'>): void {
    const full = { ...entry, at: Date.now() - this.t0 };
    this.log.push(full);
    if (typeof window !== 'undefined' && window.__UNIPI_HOST__ === this.client) (window.__UNIPI_MOCK_LOG__ ??= []).push(full);
  }

  /** The virtual clock of a session script (starts at its first open). */
  clock(key: string): { elapsed(): number } {
    if (!this.clocks.has(key)) this.clocks.set(key, Date.now());
    const t0 = this.clocks.get(key)!;
    return { elapsed: () => Date.now() - t0 };
  }

  /** A script drop: the session's host is unreachable for `ms` real milliseconds (`Infinity` = stays down). */
  markDown(key: string, ms: number): void {
    this.down.set(key, Date.now() + ms);
  }

  isDown(key: string): boolean {
    return Date.now() < (this.down.get(key) ?? 0);
  }

  /** Marks script step `index` of a session fired; true when it already had. */
  fired(key: string, index: number): boolean {
    const id = `${key}#${index}`;
    if (this.firedSteps.has(id)) return true;
    this.firedSteps.add(id);
    return false;
  }

  /** Test control: the session's host answers again. */
  restore(hostId: string, sessionId: string): void {
    this.down.delete(`${hostId}/${sessionId}`);
  }

  /** Test control: pushes events to every open stream of a session. */
  push(hostId: string, sessionId: string, items: SessionEventDto[]): void {
    for (const s of this.streams.get(`${hostId}/${sessionId}`) ?? []) s.push(items);
  }

  /** Test control: the transport skipped an oversize frame on every open stream of a session
   *  (`chat.rs` `SessionMsg::Resync`, fix-frames). */
  skipFrame(hostId: string, sessionId: string): void {
    for (const s of [...(this.streams.get(`${hostId}/${sessionId}`) ?? [])]) s.resync?.();
  }

  /** Test control: drops every open stream of a session (`E_TRANSPORT` by default). */
  drop(hostId: string, sessionId: string, error: Err = err('E_TRANSPORT', 'The connection dropped')): void {
    for (const s of [...(this.streams.get(`${hostId}/${sessionId}`) ?? [])]) s.drop(error);
  }

  /** Pushes a hub event to every `openEvents` subscriber. */
  emit(event: HubEvent): void {
    for (const sink of this.eventSinks) sink(event);
  }

  async appRequest(op: string, args: Record<string, unknown>): Promise<unknown> {
    const handler: MockOp | undefined = this.scenario.appOps?.[op] ?? APP_BUILTIN[op];
    if (op === 'list_hosts' && !handler) {
      this.record({ host: APP_HOST, kind: 'request', op, args });
      return this.scenario.hosts.map((h, i) => ({
        id: h.id,
        name: h.name,
        ticket: `mock-ticket-${h.id}`,
        paired_at: 1_790_000_000_000 - i * 86_400_000,
        last_path: h.transport === 'local' ? 'local' : 'direct',
        ...(h.transport ? { transport: h.transport } : {}),
      }));
    }
    // Legacy `call(op, {hostId, …})` wrappers: route to that host.
    if (!handler && typeof args.hostId === 'string' && args.hostId !== APP_HOST) {
      const { hostId, ...rest } = args;
      return this.client(hostId as string).request(op, rest);
    }
    this.record({ host: APP_HOST, kind: 'request', op, args, ...(handler === undefined ? { unhandled: true as const } : {}) });
    return handler ? handler(args, this.client(APP_HOST)) : null;
  }
}

/** A mock host world for a scenario: `world.client(hostId)` is the `HostClientFactory`. */
export function createMockHost(scenario: MockScenario): MockWorld {
  return new MockWorld(scenario);
}
