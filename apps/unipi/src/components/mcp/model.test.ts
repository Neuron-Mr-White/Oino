import { describe, expect, it } from 'vitest';
import {
  buildEntry, draftFromEntry, draftsFromPaste, emptyDraft, isSecretKey, joinArgs, newRow, parseMcpState, parsePaste, secretVar, serverStatus, draftFromCatalog, autonomyHint, splitArgs, validateDraft,
} from './model';

describe('parseMcpState', () => {
  it('reads the project answer: source file, badge, read-only, files, warnings, trust', () => {
    const s = parseMcpState({
      servers: [
        { name: 'linear', source: 'project', sourceFile: '/p/.oino/mcp.json', enabled: true, transport: 'http', status: 'running', toolCount: 12, tools: ['linear__list_teams'], entry: { url: 'https://mcp.linear.app/mcp' } },
        { name: 'refero', source: 'project-override', sourceFile: '/p/.oino/mcp.json', enabled: false, status: 'stopped' },
        { name: 'gh', source: 'project', sourceFile: '/p/.mcp.json', transport: 'stdio', status: 'running' },
        { name: 'ctx', source: 'global', sourceFile: '/home/me/.oino/mcp.json', status: 'running' },
      ],
      warnings: [{ file: '/p/.mcp.json', line: 7, message: 'parse error at line 7' }, 'plain'],
      files: [{ scope: 'project', path: '/p/.oino/mcp.json', exists: true }],
      trust: { state: 'trusted', path: '/p', protected: ['.oino/mcp.json'] },
    });
    expect(s.modern).toBe(true);
    expect(s.servers.map((x) => [x.name, x.badge, x.readOnly])).toEqual([['linear', 'project', false], ['refero', 'override', false], ['gh', 'project', true], ['ctx', 'global', false]]);
    expect(s.servers[0]!.entry).toEqual({ url: 'https://mcp.linear.app/mcp' });
    expect(s.warnings).toEqual([{ text: 'parse error at line 7', file: '/p/.mcp.json', line: 7 }, { text: 'plain' }]);
    expect(s.files).toEqual([{ scope: 'project', path: '/p/.oino/mcp.json', exists: true }]);
    expect(s.trust).toEqual({ state: 'trusted', path: '/p', protected: ['.oino/mcp.json'] });
  });

  it('an older answer (no files / trust / source file) is read, flagged not modern, and never throws', () => {
    const s = parseMcpState({ servers: [{ name: 'refero', source: 'global', enabled: true, transport: 'http', status: 'running', toolCount: 3 }], errors: ['x'], totalTools: 3 });
    expect(s.modern).toBe(false);
    expect(s.trust).toBeUndefined();
    expect(s.servers[0]!.readOnly).toBe(false);
    expect(parseMcpState(undefined).servers).toEqual([]);
    expect(parseMcpState('nonsense').modern).toBe(false);
  });

  it('takes the entry from a host that spreads it into the row', () => {
    const s = parseMcpState({ servers: [{ name: 'a', url: 'https://x.dev/mcp', headers: { Authorization: '${A_AUTHORIZATION}' } }] });
    expect(s.servers[0]!.entry).toEqual({ url: 'https://x.dev/mcp', headers: { Authorization: '${A_AUTHORIZATION}' } });
    expect(s.servers[0]!.transport).toBe('http');
  });
});

describe('serverStatus — the words a row shows (SPEC §2)', () => {
  const base = { enabled: true, toolCount: 0 } as const;
  it.each([
    [{ ...base, status: 'running', toolCount: 12 }, 'connected', 'Connected · 12 tools'],
    [{ ...base, status: 'running', toolCount: 1 }, 'connected', 'Connected · 1 tool'],
    [{ ...base, status: 'stopped', enabled: false }, 'disabled', 'Disabled'],
    [{ ...base, status: 'error', error: 'LINEAR_AUTHORIZATION is not set — add it in Settings › MCP\nmore' }, 'error', 'Error: LINEAR_AUTHORIZATION is not set — add it in Settings › MCP'],
    [{ ...base, status: 'not-trusted' }, 'untrusted', 'Not trusted'],
    [{ ...base, status: 'needs-autonomy-high' }, 'autonomy', 'Needs Autonomy High'],
    [{ ...base, status: 'starting' }, 'connecting', 'Connecting…'],
    [{ ...base, status: 'stopped' }, 'stopped', 'Stopped'],
  ])('%j → %s', (server, kind, label) => {
    expect(serverStatus(server)).toEqual({ kind, label });
  });
});

describe('parsePaste — the three shapes', () => {
  const linear = { type: 'http', url: 'https://mcp.linear.app/mcp', headers: { Authorization: 'Bearer lin_x' } };
  it('one entry (no name)', () => {
    const r = parsePaste(JSON.stringify(linear));
    expect(r).toEqual({ ok: true, shape: 'entry', items: [{ entry: linear }] });
  });
  it('{ name: entry }', () => {
    const r = parsePaste(JSON.stringify({ linear, fs: { command: 'npx', args: ['-y', 'x'] } }));
    expect(r.ok && r.shape).toBe('named');
    expect(r.ok && r.items.map((i) => i.name)).toEqual(['linear', 'fs']);
  });
  it('a full { mcpServers } from Claude Code / Cursor / pi', () => {
    const r = parsePaste(JSON.stringify({ mcpServers: { linear, fs: { command: 'npx', args: ['-y', 'x'], env: { A: '1' } } }, other: 1 }));
    expect(r.ok && r.shape).toBe('mcpServers');
    expect(r.ok && r.items).toHaveLength(2);
  });
  it('a fragment copied from inside a file ("name": { … },)', () => {
    const r = parsePaste(`"linear": ${JSON.stringify(linear)},`);
    expect(r.ok && r.items[0]!.name).toBe('linear');
  });
  it('says what is wrong, with a line for broken JSON', () => {
    expect(parsePaste('')).toEqual({ ok: false, error: 'Paste the server’s JSON first' });
    const broken = parsePaste('{\n  "url": "https://x.dev",\n  oops\n}');
    expect(!broken.ok && broken.error).toMatch(/not valid JSON.*line 3/);
    expect(!parsePaste('[1,2]').ok).toBe(true);
    expect(!parsePaste('{"hello": {"a": 1}}').ok).toBe(true);
    expect(!parsePaste('{"mcpServers": {}}').ok).toBe(true);
  });
});

describe('draft ↔ entry', () => {
  it('http: an Authorization header is Secret by default and leaves the entry as ${NAME}, never the value', () => {
    const d = draftsFromPaste([{ name: 'linear', entry: { type: 'http', url: 'https://mcp.linear.app/mcp', headers: { Authorization: 'Bearer lin_x', 'X-Team': 'MON' } } }], 'project', '')[0]!;
    expect(d.type).toBe('http');
    expect(d.headers.map((r) => [r.key, r.secret])).toEqual([['Authorization', true], ['X-Team', false]]);
    const { entry, secrets } = buildEntry(d);
    expect(entry).toEqual({ url: 'https://mcp.linear.app/mcp', headers: { Authorization: '${LINEAR_AUTHORIZATION}', 'X-Team': 'MON' } });
    expect(secrets).toEqual({ LINEAR_AUTHORIZATION: 'Bearer lin_x' });
    expect(JSON.stringify(entry)).not.toContain('lin_x');
  });

  it('a stored secret (${NAME}) is kept untouched unless replaced', () => {
    const d = draftFromEntry('linear', { url: 'https://x.dev/mcp', headers: { Authorization: '${LINEAR_AUTHORIZATION}' } }, 'project');
    expect(d.headers[0]).toMatchObject({ key: 'Authorization', secret: true, ref: '${LINEAR_AUTHORIZATION}', value: '' });
    expect(buildEntry(d)).toEqual({ entry: { url: 'https://x.dev/mcp', headers: { Authorization: '${LINEAR_AUTHORIZATION}' } }, secrets: {} });
    d.headers[0]!.value = 'Bearer new';
    expect(buildEntry(d).secrets).toEqual({ LINEAR_AUTHORIZATION: 'Bearer new' });
  });

  it('command: args split like a shell, env secret keys follow the rule, unknown keys ride along', () => {
    const d = draftFromEntry('fs', { command: 'npx', args: ['-y', '@x/server', '/my dir'], env: { API_KEY: 'k', REGION: 'us' }, enabled: false, exposure: 'direct' }, 'global');
    expect(d.type).toBe('command');
    expect(d.args).toBe('-y @x/server "/my dir"');
    const { entry, secrets } = buildEntry(d);
    expect(entry).toEqual({ command: 'npx', args: ['-y', '@x/server', '/my dir'], env: { API_KEY: '${FS_API_KEY}', REGION: 'us' }, enabled: false, exposure: 'direct' });
    expect(secrets).toEqual({ FS_API_KEY: 'k' });
  });

  it('turning Secret off on a literal keeps it a literal', () => {
    const d = emptyDraft('global');
    d.name = 'x';
    d.url = 'https://x.dev';
    d.headers = [{ ...newRow('Authorization', 'Bearer t'), secret: false }];
    expect(buildEntry(d)).toEqual({ entry: { url: 'https://x.dev', headers: { Authorization: 'Bearer t' } }, secrets: {} });
  });

  it('the secret-key rule and the variable name', () => {
    for (const k of ['Authorization', 'X-Api-Key', 'GITHUB_TOKEN', 'client_secret']) expect(isSecretKey(k)).toBe(true);
    for (const k of ['X-Team', 'REGION', 'Accept']) expect(isSecretKey(k)).toBe(false);
    expect(secretVar('linear', 'Authorization')).toBe('LINEAR_AUTHORIZATION');
    expect(secretVar('my-server', 'x-api-key')).toBe('MY_SERVER_X_API_KEY');
  });

  it('splitArgs / joinArgs round-trip', () => {
    expect(splitArgs(`-y "a b" 'c d' e\\ f ""`)).toEqual(['-y', 'a b', 'c d', 'e\\', 'f', '']);
    const args = ['-y', 'a b', 'say "hi"', ''];
    expect(splitArgs(joinArgs(args))).toEqual(args);
  });
});

describe('validateDraft', () => {
  it('needs a name, a URL or a command; names stay simple and unique when adding', () => {
    const d = emptyDraft('global');
    expect(validateDraft(d)).toMatchObject({ name: expect.any(String), url: expect.any(String) });
    d.type = 'command';
    expect(validateDraft(d).command).toBeTruthy();
    d.name = 'bad name!';
    expect(validateDraft(d).name).toMatch(/letters/);
    d.name = 'linear';
    d.command = 'npx';
    expect(validateDraft(d)).toEqual({});
    expect(validateDraft(d, { taken: ['linear'] }).name).toMatch(/already exists/);
  });
  it('an HTTP server needs an http(s) URL and no repeated header', () => {
    const d = { ...emptyDraft('global'), name: 'a', url: 'ftp://x' };
    expect(validateDraft(d).url).toMatch(/http/);
    d.url = 'https://x.dev';
    d.headers = [newRow('A', '1'), newRow('a', '2')];
    expect(validateDraft(d).headers).toMatch(/twice/);
  });
});

it('OIN-101 D5 masks auth/password/PAT/credential and bearer-looking imports', () => {
  for (const key of ['REFERO_AUTH', 'DB_PASSWORD', 'GH_PAT', 'APP_CREDENTIAL']) expect(isSecretKey(key)).toBe(true);
  const d = draftFromEntry('refero', {command: 'mcp-remote', env: { REFERO_AUTH: 'fixture', CUSTOM: 'Bearer fixture' }}, 'project');
  expect(d.env.every(r => r.secret)).toBe(true);
  const built = buildEntry(d);
  expect(JSON.stringify(built.entry)).not.toContain('fixture');
});

it('masked compatibility credentials stay masked and preserve their keep sentinel on Override', () => {
  const d = draftFromEntry('refero', { command: 'mcp-remote', env: { REFERO_AUTH: '••••' } }, 'project');
  expect(d.env[0]!.value).toBe('');
  expect(d.env[0]!.ref).toBe('••••');
  expect(buildEntry(d).entry.env).toEqual({ REFERO_AUTH: '••••' });
});

describe('OIN-101 UI2', () => {
  it('N8: the secret name does not repeat the server', () => {
    expect(secretVar('refero', 'REFERO_AUTH')).toBe('REFERO_AUTH');
    expect(secretVar('refero', 'AUTH')).toBe('REFERO_AUTH');
    expect(secretVar('ref', 'REFERO_AUTH')).toBe('REF_REFERO_AUTH');
    expect(secretVar('linear', 'Authorization')).toBe('LINEAR_AUTHORIZATION');
  });
  it('N8: building the refero entry stores REFERO_AUTH', () => {
    const d = { ...emptyDraft('project'), name: 'refero', type: 'command' as const, command: 'npx', env: [newRow('REFERO_AUTH', 'tok')] };
    const { entry, secrets } = buildEntry(d);
    expect(entry.env).toEqual({ REFERO_AUTH: '${REFERO_AUTH}' });
    expect(secrets).toEqual({ REFERO_AUTH: 'tok' });
  });
  it('N4: a catalog placeholder is an empty required row and fails validation until filled', () => {
    const d = draftFromCatalog('pg', { command: 'npx', env: { POSTGRES_CONNECTION_STRING: '${POSTGRES_CONNECTION_STRING}' } }, 'global');
    expect(d.env[0]).toMatchObject({ key: 'POSTGRES_CONNECTION_STRING', value: '', required: true, ref: undefined });
    expect(validateDraft(d).env).toMatch(/POSTGRES_CONNECTION_STRING/);
    d.env[0]!.value = 'postgres://x';
    expect(validateDraft(d).env).toBeUndefined();
  });
  it('N7: the status pill is short when the error is printed below', () => {
    const s = { status: 'error', enabled: true, error: 'HTTP 401: nope\nmore', toolCount: 0 };
    expect(serverStatus(s).label).toBe('Error: HTTP 401: nope');
    expect(serverStatus(s, { errorShownBelow: true }).label).toBe('Error');
    expect(serverStatus({ ...s, error: undefined }, { errorShownBelow: true }).label).toBe('Error: could not connect');
  });
  it('N10: the High hint names the chat to switch, or says Reconnect when it is already High', () => {
    const s = { status: 'running', warning: 'Project server could not start: needs Autonomy High. Using global server; Reconnect retries the project entry.' };
    expect(autonomyHint(s, { title: 'zm', level: 'medium' })).toEqual({ note: 'Project server could not start: needs Autonomy High. Using global server;', hint: 'Switch the chat “zm” to High autonomy, then Reconnect.' });
    expect(autonomyHint(s, { title: 'zm', level: 'high' })!.hint).toBe('The chat “zm” is on High autonomy: press Reconnect.');
    expect(autonomyHint({ status: 'running', warning: 'Using global server.' })).toBeUndefined();
    expect(autonomyHint({ status: 'needs-autonomy-high' })!.hint).toMatch(/switch it to High autonomy/);
    expect(autonomyHint({ status: 'needs-autonomy-high' }, undefined, false)!.hint).toBe('Set the default autonomy to High under Settings › Autonomy, then Reconnect.');
  });
});
