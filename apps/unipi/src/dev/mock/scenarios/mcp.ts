/**
 * Settings › MCP + project trust (OIN-101 WI-3) — capture/test scenarios, built against the contract of
 * docs/dev/v3/ux/mcp/SPEC.md before the backend (WI-1 / WI-2) lands:
 *
 *   mcp-global        Global scope, three servers (connected / stdio / disabled)
 *   mcp-project       Project scope of a trusted folder with two sources (.oino/mcp.json + read-only .mcp.json),
 *                     a project override of a global server, the global servers underneath
 *   mcp-untrusted     Project scope of a folder with no trust decision: "Not trusted" + [Trust this project]
 *   mcp-parse-error   Warning rows: broken JSON at a line, an unreadable file; the other servers still connect
 *   mcp-secret        An unresolved `${LINEAR_AUTHORIZATION}` (error row; Edit shows •••• + Replace)
 *   mcp-autonomy      A stdio server that needs Autonomy High
 *   mcp-old-host      An older host: legacy `mcp.servers` answer, no `mcp.save`, no trust ops
 *   mcp-trust-banner  An open chat whose snapshot says `project_trust: undecided` → the banner above the chat
 *
 * One small in-memory "extension" answers `mcp.servers / mcp.save / mcp.remove / mcp.set-enabled /
 * mcp.server-action` and the `_oino/trust_get` / `_oino/trust_set` host ops, so every control works in a capture.
 */
import type { MockScenario } from '../host';
import { CWD, HOST, SESSION, capSession, host, openChat } from './capabilities';

type Wire = Record<string, unknown>;
type Trust = { state: 'trusted' | 'untrusted' | 'undecided'; path?: string; protected: string[] };

type Init = {
  /** Server rows as `mcp.servers` returns them (global scope view; `project: true` rows only appear for the project). */
  servers: Wire[];
  warnings?: Array<{ file?: string; line?: number; message: string }>;
  files?: { global: Array<{ path: string; exists: boolean }>; project: Array<{ path: string; exists: boolean }> };
  trust?: Trust;
  /** Secrets already in the PC's store (names like `LINEAR_AUTHORIZATION`). */
  secrets?: string[];
  /** The extension predates WI-2: legacy answer shape, no save/remove/set-enabled, no trust ops. */
  legacy?: boolean;
};

const REFERO_TOOLS = ['refero__search_screens', 'refero__get_screen', 'refero__search_flows', 'refero__get_flow', 'refero__list_collections', 'refero__get_collection', 'refero__list_styles'];
const LINEAR_TOOLS = ['linear__list_teams', 'linear__list_issues', 'linear__get_issue', 'linear__create_issue', 'linear__update_issue', 'linear__list_projects', 'linear__list_cycles', 'linear__search_documentation', 'linear__list_comments', 'linear__create_comment', 'linear__list_users', 'linear__get_team'];

const G = '~/.oino/mcp.json';
const P = `${CWD}/.oino/mcp.json`;
const CLAUDE = `${CWD}/.mcp.json`;

const refero = (extra: Wire = {}): Wire => ({
  name: 'refero', source: 'global', scope: 'global', sourceFile: G, enabled: true, transport: 'http', status: 'running', toolCount: REFERO_TOOLS.length, tools: REFERO_TOOLS,
  entry: { url: 'https://api.refero.design/mcp', headers: { Authorization: '${REFERO_AUTHORIZATION}' } }, ...extra,
});
const context7 = (extra: Wire = {}): Wire => ({
  name: 'context7', source: 'global', scope: 'global', sourceFile: G, enabled: true, transport: 'stdio', status: 'running', toolCount: 2, tools: ['context7__resolve-library-id', 'context7__get-library-docs'],
  entry: { command: 'npx', args: ['-y', '@upstash/context7-mcp'] }, ...extra,
});
const playwright = (): Wire => ({
  name: 'playwright', source: 'global', scope: 'global', sourceFile: G, enabled: false, transport: 'stdio', status: 'stopped', toolCount: 0, tools: [],
  entry: { command: 'npx', args: ['@playwright/mcp@latest'], enabled: false },
});
const linear = (extra: Wire = {}): Wire => ({
  name: 'linear', source: 'project', scope: 'project', project: true, sourceFile: P, enabled: true, transport: 'http', status: 'running', toolCount: LINEAR_TOOLS.length, tools: LINEAR_TOOLS,
  entry: { url: 'https://mcp.linear.app/mcp', headers: { Authorization: '${LINEAR_AUTHORIZATION}' } }, ...extra,
});
const github = (extra: Wire = {}): Wire => ({
  name: 'github', source: 'project', scope: 'project', project: true, sourceFile: CLAUDE, readOnly: true, enabled: true, transport: 'stdio', status: 'running', toolCount: 5, tools: ['github__search_repositories', 'github__get_file_contents', 'github__list_issues', 'github__create_issue', 'github__create_pull_request'],
  entry: { command: 'npx', args: ['-y', '@modelcontextprotocol/server-github'], env: { GITHUB_TOKEN: '${GITHUB_TOKEN}' } }, ...extra,
});

const FILES = {
  global: [{ path: G, exists: true }],
  project: [
    { path: P, exists: true },
    { path: `${CWD}/.pi/mcp.json`, exists: false },
    { path: `${CWD}/.unipi/config/mcp/mcp-config.json`, exists: false },
    { path: CLAUDE, exists: true },
  ],
};

/** A few catalog entries as the extension's `mcp.catalog` answers them (placeholders are `${VAR}`, never stored secrets). */
const CATALOG = [
  { id: 'modelcontextprotocol/postgres', name: 'postgresql', label: 'PostgreSQL', description: 'PostgreSQL database access — query, inspect schema, and manage data', entry: { command: 'npx', args: ['-y', '@modelcontextprotocol/server-postgres'], env: { POSTGRES_CONNECTION_STRING: '${POSTGRES_CONNECTION_STRING}' } } },
  { id: 'neondatabase/neon', name: 'neon', label: 'Neon', description: 'Neon serverless Postgres — database management, branching, and queries', entry: { command: 'npx', args: ['-y', '@neondatabase/mcp-server-neon'], env: { NEON_API_KEY: '${NEON_API_KEY}' } } },
  { id: 'github/github-mcp-server', name: 'github', label: 'GitHub MCP Server', description: 'GitHub’s official MCP server — repositories, issues, pull requests and code search, with a description long enough to wrap onto a second line in the narrow phone layout', entry: { command: 'npx', args: ['-y', '@modelcontextprotocol/server-github'], env: { GITHUB_PERSONAL_ACCESS_TOKEN: '${GITHUB_PERSONAL_ACCESS_TOKEN}' } } },
];

/** An in-memory stand-in for the extension's services + the host's trust ops. */
function extension(init: Init) {
  const rows = init.servers.map((s) => ({ ...s }));
  let trust = init.trust ? { ...init.trust } : undefined;
  const secrets = new Set(init.secrets ?? []);
  const projectLive = () => trust?.state === 'trusted';

  const unresolved = (entry: Wire): string | undefined => {
    const text = JSON.stringify(entry);
    for (const m of text.matchAll(/\$\{([A-Z0-9_]+)\}/g)) if (!secrets.has(m[1]!)) return `${m[1]} is not set — add it in Settings › MCP`;
    return undefined;
  };
  const settle = (s: Wire): Wire => {
    if (s.enabled === false) return { ...s, status: 'stopped', error: undefined, toolCount: 0, tools: [] };
    const missing = unresolved(s.entry as Wire);
    if (missing) return { ...s, status: 'error', error: missing, toolCount: 0, tools: [] };
    if (s.status === 'error' || s.status === 'stopped') return { ...s, status: 'running', error: undefined, toolCount: 3, tools: [`${s.name}__ping`, `${s.name}__list`, `${s.name}__get`] };
    return s;
  };
  const view = (cwd?: string): Wire => {
    // An untrusted folder's project files are not read at all (SPEC §1): its rows are simply absent.
    const visible = rows.filter((s) => !s.project || (cwd && projectLive()));
    if (init.legacy) {
      return { servers: visible.map(({ name, source, enabled, transport, status, error, toolCount, tools }) => ({ name, source: String(source).startsWith('project') ? 'project' : source, enabled, transport, status, error, toolCount, tools })), errors: [], totalTools: visible.reduce((n, s) => n + Number(s.toolCount ?? 0), 0) };
    }
    return {
      servers: visible,
      warnings: cwd ? (init.warnings ?? []) : [],
      files: [...FILES.global.map((f) => ({ scope: 'global', ...f })), ...(cwd ? (init.files?.project ?? FILES.project).map((f) => ({ scope: 'project', ...f })) : [])].filter((f) => cwd ? f.scope === 'project' : f.scope === 'global'),
      ...(cwd && trust ? { trust } : {}),
      totalTools: visible.reduce((n, s) => n + Number(s.toolCount ?? 0), 0),
    };
  };

  const service = (cmd: { name?: string; payload?: Record<string, unknown> }): unknown => {
    const p = cmd.payload ?? {};
    const cwd = typeof p.cwd === 'string' ? p.cwd : undefined;
    const name = String(p.name ?? '');
    switch (cmd.name) {
      case 'mcp.servers':
        return view(cwd);
      case 'mcp.save': {
        if (init.legacy) throw new Error('unknown service mcp.save');
        const scope = p.scope === 'project' ? 'project' : 'global';
        for (const k of Object.keys((p.secrets ?? {}) as Record<string, string>)) secrets.add(k);
        for (const [n, entry] of Object.entries((p.entries ?? {}) as Record<string, Wire>)) {
          const transport = typeof entry.url === 'string' ? 'http' : 'stdio';
          const next = settle({ name: n, source: scope, scope, project: scope === 'project', sourceFile: scope === 'project' ? P : G, enabled: entry.enabled !== false, transport, status: 'stopped', toolCount: 0, tools: [], entry });
          const i = rows.findIndex((r) => r.name === n && !!r.project === (scope === 'project'));
          if (i >= 0) rows[i] = next;
          else rows.push(next);
        }
        return { ok: true, servers: view(cwd), notice: undefined };
      }
      case 'mcp.remove': {
        if (init.legacy) throw new Error('unknown service mcp.remove');
        const i = rows.findIndex((r) => r.name === name && !!r.project === (p.scope === 'project'));
        if (i >= 0) rows.splice(i, 1);
        return { ok: true, servers: view(cwd) };
      }
      case 'mcp.set-enabled': {
        if (init.legacy) throw new Error('unknown service mcp.set-enabled');
        const r = rows.find((x) => x.name === name);
        if (r) Object.assign(r, settle({ ...r, enabled: p.enabled !== false }));
        return { ok: true, servers: view(cwd) };
      }
      case 'mcp.server-action': {
        const r = rows.find((x) => x.name === name);
        const action = String(p.action);
        if (r && action === 'reconnect' && r.status !== 'needs-autonomy-high') Object.assign(r, settle({ ...r, status: 'stopped' }));
        if (r && (action === 'enable' || action === 'disable')) Object.assign(r, settle({ ...r, enabled: action === 'enable' }));
        return { ok: true, servers: view(cwd) };
      }
      case 'mcp.catalog': {
        const q = String(p.query ?? '').toLowerCase();
        return { entries: CATALOG.filter((e) => !q || `${e.label} ${e.description}`.toLowerCase().includes(q)) };
      }
      default:
        return {};
    }
  };

  const ops: Record<string, (c: never) => unknown> = {
    '_oino/service_call': service as never,
    '_oino/trust_get': (() => {
      if (init.legacy) throw new Error('unknown command _oino/trust_get');
      return trust ?? { state: 'untrusted', protected: [] };
    }) as never,
    '_oino/trust_set': ((c: { path: string; trusted: boolean }) => {
      if (init.legacy) throw new Error('unknown command _oino/trust_set');
      trust = { state: c.trusted ? 'trusted' : 'untrusted', path: c.path, protected: trust?.protected ?? [] };
      return { ok: true, project_trust: trust };
    }) as never,
  };
  return ops;
}

function scenario(name: string, init: Init, opts: { project?: boolean; hash?: string; snapshotTrust?: Trust } = {}): MockScenario {
  const session = capSession();
  session.cmd = extension(init) as never;
  if (opts.snapshotTrust) session.snapshot.project_trust = { ...opts.snapshotTrust, path: opts.snapshotTrust.path ?? CWD };
  return {
    name,
    hosts: [host({ sessions: { [SESSION]: session } })],
    setup: () => {
      if (opts.hash) {
        // The banner capture shows the chat itself (no Settings in between: chat → Settings unmount is a known crash).
        location.hash = opts.hash;
        return;
      }
      openChat();
      if (opts.project) void import('../../../settings/target').then((t) => t.setProjectScope({ hostId: HOST, cwd: CWD }));
      location.hash = '#/settings/mcp';
    },
  };
}

const TRUSTED: Trust = { state: 'trusted', path: CWD, protected: ['.oino/mcp.json', '.mcp.json'] };

export const mcpGlobal = scenario('mcp-global', { servers: [refero(), context7(), playwright()] });

export const mcpProject = scenario(
  'mcp-project',
  {
    servers: [
      linear(),
      refero({ source: 'project-override', scope: 'override', project: true, sourceFile: P, enabled: false, status: 'stopped', toolCount: 0, tools: [], entry: { enabled: false } }),
      github(),
      context7(),
    ],
    trust: TRUSTED,
    secrets: ['LINEAR_AUTHORIZATION', 'GITHUB_TOKEN', 'REFERO_AUTHORIZATION'],
  },
  { project: true },
);

export const mcpUntrusted = scenario(
  'mcp-untrusted',
  { servers: [linear(), github(), context7()], trust: { state: 'undecided', path: CWD, protected: ['.oino/mcp.json', '.mcp.json'] }, secrets: ['LINEAR_AUTHORIZATION', 'GITHUB_TOKEN'] },
  { project: true },
);

export const mcpParseError = scenario(
  'mcp-parse-error',
  {
    servers: [linear(), context7()],
    warnings: [
      { file: CLAUDE, line: 7, message: 'parse error at line 7: expected `,` or `}` — its servers were skipped' },
      { file: `${CWD}/.pi/mcp.json`, message: 'unreadable: permission denied' },
    ],
    trust: TRUSTED,
    secrets: ['LINEAR_AUTHORIZATION'],
  },
  { project: true },
);

export const mcpSecret = scenario(
  'mcp-secret',
  { servers: [linear({ status: 'error', toolCount: 0, tools: [], error: 'LINEAR_AUTHORIZATION is not set — add it in Settings › MCP' }), context7()], trust: TRUSTED, secrets: [] },
  { project: true },
);

export const mcpAutonomy = scenario(
  'mcp-autonomy',
  {
    servers: [
      { name: 'filesystem', source: 'global', scope: 'global', sourceFile: G, enabled: true, transport: 'stdio', status: 'needs-autonomy-high', warning: 'Project server could not start: Failed to spawn MCP server: needs Autonomy High. Using global server; Reconnect retries the project entry.', toolCount: 0, tools: [], entry: { command: 'npx', args: ['-y', '@modelcontextprotocol/server-filesystem', '/home/you/projects'] } },
      refero(),
      context7(),
    ],
  },
);

/** The same, in a trusted project (the hint names the chat or says to open one). */
export const mcpAutonomyProject = scenario(
  'mcp-autonomy-project',
  {
    servers: [
      { name: 'refero', source: 'project-override', scope: 'project', project: true, sourceFile: P, enabled: true, transport: 'stdio', status: 'running', toolCount: REFERO_TOOLS.length, tools: REFERO_TOOLS, warning: 'Project server could not start: Failed to spawn MCP server: needs Autonomy High. Using global server; Reconnect retries the project entry.', entry: { command: 'npx', args: ['-y', 'refero-mcp'], env: { REFERO_AUTH: '${REFERO_AUTH}' } } },
      linear(),
    ],
    trust: TRUSTED,
    secrets: ['LINEAR_AUTHORIZATION', 'REFERO_AUTH'],
  },
  { project: true },
);

export const mcpOldHost = scenario('mcp-old-host', { servers: [refero(), context7()], legacy: true }, { project: true });

export const mcpTrustBanner = scenario(
  'mcp-trust-banner',
  { servers: [], trust: { state: 'undecided', path: CWD, protected: ['.oino/mcp.json', '.mcp.json', '.oino/skills'] } },
  { hash: `#/s/${HOST}/${SESSION}`, snapshotTrust: { state: 'undecided', path: CWD, protected: ['.oino/mcp.json', '.mcp.json', '.oino/skills'] } },
);
