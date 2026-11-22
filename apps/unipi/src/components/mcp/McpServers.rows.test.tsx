// @vitest-environment jsdom
/**
 * Settings › MCP (OIN-101 WI-3): the scope picker, the header (files + trust), the status list and its actions, over a
 * fake extension that answers the SPEC's services (`mcp.servers {cwd}`, `mcp.save`, `mcp.remove`, `mcp.set-enabled`,
 * `mcp.server-action`) and the host's trust ops. Older-host answers (no `files` / `trust`, unknown service) are
 * covered too.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@solidjs/testing-library';
import { toasts } from '../../lib/toasts';

type Wire = Record<string, unknown>;
const h = vi.hoisted(() => ({
  calls: [] as { type: string; name?: string; payload?: Record<string, unknown>; path?: string; trusted?: boolean }[],
  answer: {} as Record<string, unknown>,
  throwFor: new Set<string>(),
  confirm: true,
  cards: {} as Record<string, unknown>,
}));

vi.mock('../../lib/session/store', () => ({ sessions: () => ({ keys: () => [], peek: () => undefined }) }));
vi.mock('../../lib/host/factory', () => ({ setHostClientFactory: () => {}, hostClient: () => ({ request: async (_op: string, args: { cmd: { kind: string; cwd?: string } }) => {
  const c = args.cmd;
  if (c.kind === 'get_settings') return { value: 'medium' };
  const key = c.kind === 'mcp_servers' ? 'mcp.servers' : '_oino/trust_get';
  const cmd = { type: key === 'mcp.servers' ? '_oino/service_call' : key, name: key, payload: c.cwd ? { cwd: c.cwd } : {} };
  h.calls.push(cmd);
  if (h.throwFor.has(key)) throw new Error(`unknown service ${key}`);
  const a = h.answer[key];
  return typeof a === 'function' ? (a as (c: unknown) => unknown)(cmd) : a;
} }) }));
vi.mock('../../lib/settings/run', () => ({
  settingsCommandChat: async (_host: string, _start?: unknown, cwd?: string) => ({
    cmd: async (cmd: { type: string; name?: string; payload?: Record<string, unknown>; path?: string; trusted?: boolean }) => {
      h.calls.push({ ...cmd, ...(cwd ? { payload: { ...(cmd.payload ?? {}), __chatCwd: cwd } } : {}) });
      const key = cmd.type === '_oino/service_call' ? String(cmd.name) : cmd.type;
      if (h.throwFor.has(key)) throw new Error(`unknown service ${key}`);
      const a = h.answer[key];
      return typeof a === 'function' ? (a as (c: unknown) => unknown)(cmd) : a;
    },
  }),
}));
vi.mock('../../lib/session/hub', async (orig) => ({ ...(await orig<typeof import('../../lib/session/hub')>()), hubs: () => ({ ensure: () => ({ state: { cards: h.cards } }) }) }));
vi.mock('../phone/Sheet', async (orig) => ({ ...(await orig<typeof import('../phone/Sheet')>()), confirmSheet: async () => h.confirm }));

const { McpServers } = await import('./McpServers');
const { setProjectScope } = await import('../../settings/target');

const CWD = '/home/you/projects/zm';
const P = `${CWD}/.oino/mcp.json`;
const server = (o: Wire): Wire => ({ source: 'global', sourceFile: '~/.oino/mcp.json', enabled: true, transport: 'http', status: 'running', toolCount: 3, tools: ['a__x'], ...o });

const globalAnswer = () => ({
  servers: [server({ name: 'refero' }), server({ name: 'ctx', transport: 'stdio', status: 'stopped', enabled: false, toolCount: 0 })],
  warnings: [],
  files: [{ scope: 'global', path: '~/.oino/mcp.json', exists: true }],
  totalTools: 3,
});
const projectAnswer = (trust: Wire = { state: 'trusted', path: CWD, protected: ['.oino/mcp.json'] }) => ({
  servers: [
    server({ name: 'linear', source: 'project', sourceFile: P, toolCount: 12 }),
    server({ name: 'refero', source: 'project-override', sourceFile: P, enabled: false, status: 'stopped', toolCount: 0 }),
    server({ name: 'gh', source: 'project', sourceFile: `${CWD}/.mcp.json`, transport: 'stdio', readOnly: true, toolCount: 5 }),
    server({ name: 'ctx', source: 'global', transport: 'stdio', status: 'error', error: 'spawn npx ENOENT', toolCount: 0 }),
  ],
  warnings: [{ file: `${CWD}/.pi/mcp.json`, line: 4, message: 'parse error at line 4' }],
  files: [{ scope: 'project', path: P, exists: true }, { scope: 'project', path: `${CWD}/.mcp.json`, exists: true }, { scope: 'project', path: `${CWD}/.pi/mcp.json`, exists: false }],
  trust,
  totalTools: 20,
});

beforeEach(() => {
  h.calls.length = 0;
  h.answer = { 'mcp.servers': (c: { payload?: { cwd?: string } }) => (c.payload?.cwd ? projectAnswer() : globalAnswer()) };
  h.throwFor.clear();
  h.confirm = true;
  h.cards = {};
  setProjectScope(undefined);
});
afterEach(() => {
  cleanup();
  toasts().clear();
  setProjectScope(undefined);
});

const renderPage = () => render(() => <McpServers hostId="this-pc" />);
const svc = (name: string) => h.calls.filter((c) => c.type === '_oino/service_call' && c.name === name);

describe('Global scope', () => {
  it('lists the servers with scope badge, source file, transport and the SPEC status words', async () => {
    renderPage();
    const row = await screen.findByTestId('mcp-row-refero');
    expect(within(row).getByTestId('mcp-badge-refero').textContent).toBe('Global');
    expect(within(row).getByTestId('mcp-status-refero').textContent).toBe('Connected · 3 tools');
    expect(row.textContent).toContain('~/.oino/mcp.json');
    expect(row.textContent).toContain('HTTP');
    expect(screen.getByTestId('mcp-status-ctx').textContent).toBe('Disabled');
    expect(screen.getByTestId('mcp-row-ctx').textContent).toContain('Command');
    // The header names the resolved file; no trust control in global scope.
    expect(screen.getByTestId('mcp-files').textContent).toContain('~/.oino/mcp.json');
    expect(screen.queryByTestId('mcp-trust')).toBeNull();
    // The first call carries no cwd: the host's global view.
    expect(svc('mcp.servers')[0]!.payload).toEqual({});
  });

  it('the enable switch calls mcp.set-enabled for the scope and says what happened', async () => {
    h.answer['mcp.set-enabled'] = () => ({ ok: true, servers: { ...globalAnswer(), servers: [server({ name: 'refero', enabled: false, status: 'stopped', toolCount: 0 }), server({ name: 'ctx' })] } });
    renderPage();
    const toggle = await screen.findByTestId('mcp-en-refero');
    expect(toggle.getAttribute('aria-label')).toBe('refero enabled');
    fireEvent.click(toggle.querySelector('input')!);
    await waitFor(() => expect(svc('mcp.set-enabled')[0]?.payload).toEqual({ scope: 'global', name: 'refero', enabled: false }));
    await waitFor(() => expect(screen.getByTestId('mcp-status-refero').textContent).toBe('Disabled'));
    await waitFor(() => expect(toasts().list().some((t) => t.message.includes('refero'))).toBe(true));
  });

  it('an extension without mcp.set-enabled falls back to the old server-action (global only)', async () => {
    h.throwFor.add('mcp.set-enabled');
    h.answer['mcp.server-action'] = { ok: true, servers: globalAnswer() };
    renderPage();
    fireEvent.click((await screen.findByTestId('mcp-en-refero')).querySelector('input')!);
    await waitFor(() => expect(svc('mcp.server-action')[0]?.payload).toEqual({ action: 'disable', name: 'refero' }));
  });

  it('Reconnect restarts one server; Reload all reloads every project', async () => {
    h.answer['mcp.server-action'] = { ok: true, servers: globalAnswer() };
    renderPage();
    fireEvent.click(await screen.findByTestId('mcp-reconnect-refero'));
    await waitFor(() => expect(svc('mcp.server-action')[0]?.payload).toEqual({ action: 'reconnect', scope: 'global', name: 'refero' }));
    fireEvent.click(screen.getByTestId('mcp-reload'));
    await waitFor(() => expect(svc('mcp.server-action')[1]?.payload).toEqual({ action: 'reload', scope: 'global' }));
    expect(screen.getByTestId('mcp-reload').textContent).toContain('Reload all');
  });

  it('Sync catalog is a secondary action (the catalog stays secondary)', async () => {
    h.answer['mcp.server-action'] = { ok: true, servers: globalAnswer() };
    renderPage();
    fireEvent.click(await screen.findByTestId('mcp-sync'));
    await waitFor(() => expect(svc('mcp.server-action')[0]?.payload).toEqual({ action: 'sync', scope: 'global' }));
  });

  it('Remove asks first, then calls mcp.remove', async () => {
    h.answer['mcp.remove'] = { ok: true, servers: { ...globalAnswer(), servers: [server({ name: 'ctx' })] } };
    h.confirm = false;
    renderPage();
    fireEvent.click(await screen.findByTestId('mcp-remove-refero'));
    await new Promise((r) => setTimeout(r, 30));
    expect(svc('mcp.remove')).toHaveLength(0);
    h.confirm = true;
    fireEvent.click(screen.getByTestId('mcp-remove-refero'));
    await waitFor(() => expect(svc('mcp.remove')[0]?.payload).toEqual({ scope: 'global', name: 'refero' }));
    await waitFor(() => expect(screen.queryByTestId('mcp-row-refero')).toBeNull());
  });

  it('a server that fails says why, in full', async () => {
    const long = 'MCP initialize handshake failed: HTTP 415: {"error":{"message":"Unsupported Media Type"}}';
    h.answer['mcp.servers'] = { servers: [server({ name: 'refero', status: 'error', error: long })], files: [], totalTools: 0 };
    renderPage();
    await screen.findByTestId('mcp-err-refero');
    expect(screen.getByTestId('mcp-err-refero').textContent).toContain('HTTP 415');
    // N7: the pill only says "Error"; the full text is printed once, below.
    expect(screen.getByTestId('mcp-status-refero').textContent).toBe('Error');
    expect(screen.getByTestId('mcp-row-refero').textContent!.match(/MCP initialize handshake failed/g)).toHaveLength(1);
  });

  it('the message of a host that cannot answer is shown instead of a blank page', async () => {
    h.throwFor.add('mcp.servers');
    renderPage();
    expect((await screen.findByTestId('mcp-error')).textContent).toMatch(/too old/);
    expect((screen.getByTestId('mcp-add') as HTMLButtonElement).disabled).toBe(true);
  });
});

describe('Project scope', () => {
  beforeEach(() => setProjectScope({ hostId: 'this-pc', cwd: CWD }));

  it('preselects the project, asks the host for it and shows its files + trust', async () => {
    renderPage();
    await screen.findByTestId('mcp-row-linear');
    expect((screen.getByTestId('mcp-scope') as HTMLSelectElement).value).toBe(CWD);
    expect(svc('mcp.servers')[0]!.payload).toMatchObject({ cwd: CWD });
    // Listing addresses the project without creating a command chat.
    expect(h.calls[0]!.payload).toMatchObject({ cwd: CWD });
    const files = screen.getByTestId('mcp-files').textContent!;
    expect(files).toContain('.oino/mcp.json');
    expect(files).toContain('.mcp.json');
    expect(screen.getByTestId('mcp-trust').textContent).toContain('Trusted');
  });

  it('badges: Project, Project overrides global, Global; read-only rows offer Override in Oino', async () => {
    renderPage();
    await screen.findByTestId('mcp-row-linear');
    expect(screen.getByTestId('mcp-badge-linear').textContent).toBe('Project');
    expect(screen.getByTestId('mcp-badge-refero').textContent).toBe('Project overrides global');
    expect(screen.getByTestId('mcp-badge-ctx').textContent).toBe('Global');
    const gh = screen.getByTestId('mcp-row-gh');
    expect(gh.textContent).toContain('read-only');
    expect(within(gh).getByTestId('mcp-override-gh')).toBeTruthy();
    expect(within(gh).queryByTestId('mcp-edit-gh')).toBeNull();
    expect(within(gh).queryByTestId('mcp-en-gh')).toBeNull();
    // A global server is not removable from a project's page (that would remove it everywhere).
    expect(within(screen.getByTestId('mcp-row-ctx')).queryByTestId('mcp-remove-ctx')).toBeNull();
    expect(within(screen.getByTestId('mcp-row-linear')).getByTestId('mcp-remove-linear')).toBeTruthy();
  });

  it('shows parse / skip reasons as a warning row with file and line', async () => {
    renderPage();
    const w = await screen.findByTestId('mcp-warnings');
    expect(w.textContent).toContain('.pi/mcp.json');
    expect(w.textContent).toContain(':4');
    expect(w.textContent).toContain('parse error at line 4');
  });

  it('the enable switch on a global server writes a project-scope partial override', async () => {
    h.answer['mcp.set-enabled'] = { ok: true, servers: projectAnswer() };
    renderPage();
    fireEvent.click((await screen.findByTestId('mcp-en-ctx')).querySelector('input')!);
    await waitFor(() => expect(svc('mcp.set-enabled')[0]?.payload).toMatchObject({ scope: 'project', cwd: CWD, name: 'ctx', enabled: false }));
  });

  it('untrusted: says so with one [Trust this project]; Add server waits for it; granting calls _oino/trust_set', async () => {
    let trust: Wire = { state: 'undecided', path: CWD, protected: ['.oino/mcp.json'] };
    h.answer['mcp.servers'] = () => ({ ...projectAnswer(trust), servers: trust.state === 'trusted' ? projectAnswer().servers : [server({ name: 'ctx' })] });
    h.answer['_oino/trust_set'] = (c: { trusted: boolean; path: string }) => {
      trust = { state: c.trusted ? 'trusted' : 'untrusted', path: c.path, protected: ['.oino/mcp.json'] };
      return { ok: true };
    };
    h.answer['_oino/trust_get'] = () => trust;
    renderPage();
    const box = await screen.findByTestId('mcp-trust');
    expect(box.textContent).toContain('Not trusted');
    expect(box.textContent).toContain('project servers aren’t loaded');
    expect((screen.getByTestId('mcp-add') as HTMLButtonElement).disabled).toBe(true);
    expect(screen.queryByTestId('mcp-row-linear')).toBeNull();
    fireEvent.click(screen.getByTestId('mcp-trust-grant'));
    await waitFor(() => expect(h.calls.find((c) => c.type === '_oino/trust_set')).toMatchObject({ path: CWD, trusted: true }));
    await screen.findByTestId('mcp-row-linear');
    expect(screen.getByTestId('mcp-trust').textContent).toContain('Trusted');
    expect((screen.getByTestId('mcp-add') as HTMLButtonElement).disabled).toBe(false);
    // Revoke saves `false` on the folder itself.
    fireEvent.click(screen.getByTestId('mcp-trust-revoke'));
    await waitFor(() => expect(h.calls.filter((c) => c.type === '_oino/trust_set')[1]).toMatchObject({ path: CWD, trusted: false }));
    await waitFor(() => expect(screen.getByTestId('mcp-trust').textContent).toContain('Not trusted'));
  });

  it('the status words for Needs Autonomy High and Not trusted rows', async () => {
    h.answer['mcp.servers'] = { ...projectAnswer(), servers: [server({ name: 'fs', transport: 'stdio', status: 'needs-autonomy-high', toolCount: 0 }), server({ name: 'x', status: 'not-trusted' })] };
    renderPage();
    expect((await screen.findByTestId('mcp-status-fs')).textContent).toBe('Needs Autonomy High');
    expect(screen.getByTestId('mcp-status-x').textContent).toBe('Not trusted');
    // Reconnect stays available on Needs Autonomy High (after switching to High it connects), not on Not trusted.
    expect((screen.getByTestId('mcp-reconnect-fs') as HTMLButtonElement).disabled).toBe(false);
    expect((screen.getByTestId('mcp-reconnect-x') as HTMLButtonElement).disabled).toBe(true);
  });

  it('an older host (no files / trust): says so, shows no servers under the project, keeps project editing off', async () => {
    h.answer['mcp.servers'] = { servers: [server({ name: 'refero', sourceFile: undefined })], errors: [], totalTools: 3 };
    h.throwFor.add('_oino/trust_get');
    renderPage();
    expect((await screen.findByTestId('mcp-old-host')).textContent).toMatch(/older/);
    expect(screen.getByTestId('mcp-trust').textContent).toMatch(/doesn’t report project trust/);
    expect((screen.getByTestId('mcp-add') as HTMLButtonElement).disabled).toBe(true);
    // Its (global) servers are not passed off as the project's.
    expect(screen.queryByTestId('mcp-row-refero')).toBeNull();
  });
});

describe('the scope picker', () => {
  it('offers Global and the known projects; picking one writes the shared project scope', async () => {
    h.cards = {
      a: { session_id: 'a', cwd: '/home/you/projects/zm', updated_at: 2 },
      b: { session_id: 'b', cwd: '/home/you/projects/oino', updated_at: 1 },
    };
    renderPage();
    const select = (await screen.findByTestId('mcp-scope')) as HTMLSelectElement;
    expect([...select.options].map((o) => o.textContent)).toEqual(['Global', 'Project: zm', 'Project: oino']);
    expect(select.value).toBe('global');
    fireEvent.change(select, { target: { value: '/home/you/projects/oino' } });
    await waitFor(() => expect(svc('mcp.servers').some((c) => c.payload?.cwd === '/home/you/projects/oino')).toBe(true));
  });
});


it('OIN-101 D2 shows a working global fallback and its warning on the row', async () => {
  h.answer['mcp.servers'] = { ...globalAnswer(), servers: [server({ name: 'refero', warning: 'Project server could not start. Using global server; Reconnect retries the project entry.' })] };
  renderPage();
  const row = await screen.findByTestId('mcp-row-refero');
  expect(within(row).getByTestId('mcp-status-refero').textContent).toContain('Connected');
  expect(within(row).getByTestId('mcp-warning-refero').textContent).toContain('Using global server');
});
it('OIN-101 D11 groups project rows before global rows', async () => {
  setProjectScope({ hostId: 'this-pc', cwd: CWD });
  renderPage();
  await screen.findByTestId('mcp-row-linear');
  const badges = [...document.querySelectorAll('.mcps-list .mcps-badge')].map(e => e.textContent);
  expect(badges.at(-1)).toBe('Global');
  expect(badges.slice(0, -1)).not.toContain('Global');
});

describe('OIN-101 UI2: autonomy attribution (N5) and the High hint (N10)', () => {
  it('N5: Global scope has no project, so no "Settings default (no open chat in this project)" line', async () => {
    renderPage();
    await screen.findByTestId('mcp-row-refero');
    expect(screen.queryByTestId('mcp-autonomy-refero')).toBeNull();
    expect(document.body.textContent).not.toContain('no open chat in this project');
  });

  it('a project with no open chat says it uses the Settings default', async () => {
    setProjectScope({ hostId: 'this-pc', cwd: CWD });
    renderPage();
    await screen.findByTestId('mcp-row-linear');
    await waitFor(() => expect(screen.getByTestId('mcp-autonomy-linear').textContent).toBe('Autonomy medium · Settings default (no open chat in this project)'));
  });

  it('N2/N6: an open chat of the project (known from the hub card, even when the registry let it go) sets the level, by its title', async () => {
    const { rememberAutonomy, resetRememberedAutonomy } = await import('../../lib/session/chatAutonomy');
    resetRememberedAutonomy();
    rememberAutonomy('this-pc', 'a1b2', 'high');
    h.cards = { a1b2: { session_id: 'a1b2', cwd: CWD, name: 'zm migration', run_state: 'idle', updated_at: 5 } };
    setProjectScope({ hostId: 'this-pc', cwd: CWD });
    renderPage();
    await screen.findByTestId('mcp-row-linear');
    await waitFor(() => expect(screen.getByTestId('mcp-autonomy-linear').textContent).toBe('Autonomy high · from chat “zm migration”'));
    expect(document.body.textContent).not.toContain('a1b2');
    resetRememberedAutonomy();
  });

  it('N6: an untitled chat is "Untitled chat", never a session id', async () => {
    const { rememberAutonomy, resetRememberedAutonomy } = await import('../../lib/session/chatAutonomy');
    resetRememberedAutonomy();
    rememberAutonomy('this-pc', '524e8836-49a2-5444-983f-ea141c3f77c4', 'medium');
    h.cards = { x: { session_id: '524e8836-49a2-5444-983f-ea141c3f77c4', cwd: CWD, name: null, run_state: 'idle' } };
    setProjectScope({ hostId: 'this-pc', cwd: CWD });
    renderPage();
    await screen.findByTestId('mcp-row-linear');
    await waitFor(() => expect(screen.getByTestId('mcp-autonomy-linear').textContent).toBe('Autonomy medium · from chat “Untitled chat”'));
    expect(document.body.textContent).not.toContain('524e8836');
    resetRememberedAutonomy();
  });

  it('N10: "needs Autonomy High" says what to do', async () => {
    setProjectScope({ hostId: 'this-pc', cwd: CWD });
    h.answer['mcp.servers'] = { ...projectAnswer(), servers: [server({ name: 'refero', source: 'project-override', sourceFile: P, warning: 'Project server could not start: Failed to spawn MCP server: needs Autonomy High. Using global server; Reconnect retries the project entry.' })] };
    renderPage();
    const hint = await screen.findByTestId('mcp-autonomy-hint-refero');
    expect(hint.textContent).toBe('Open a chat in this project, switch it to High autonomy, then Reconnect.');
    const warning = screen.getByTestId('mcp-warning-refero').textContent!;
    expect(warning).toContain('needs Autonomy High. Using global server;');
    expect(warning).not.toContain('Reconnect retries the project entry');
  });
});
