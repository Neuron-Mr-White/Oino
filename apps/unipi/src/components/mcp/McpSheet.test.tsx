// @vitest-environment jsdom
/**
 * The Add / Edit sheet (OIN-101 WI-3, SPEC §2): the Form tab, Paste JSON in all three shapes previewed in the form
 * before saving, secret handling (default-on toggle, •••• + Replace, the entry never carries a value), validation,
 * and the status shown after Save. The phone shape: a full-height sheet (`data-size=full`).
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@solidjs/testing-library';
import { createSignal } from 'solid-js';
import { McpSheet, type McpSheetProps } from './McpSheet';
import type { McpServer } from './model';

type Save = McpSheetProps['onSave'];
const PROJECT = { cwd: '/home/you/projects/zm', writable: true };

function open(over: Partial<McpSheetProps> = {}, save: Save = async () => ({ ok: true })) {
  const onSave = vi.fn(save);
  const onClose = vi.fn();
  const [servers, setServers] = createSignal<McpServer[]>([]);
  // One spread (Solid's mergeProps skips an explicit `undefined` override, and a test needs "no project").
  const props: McpSheetProps = { mode: { kind: 'add' }, project: PROJECT, taken: () => [], servers, onSave, onClose, ...over };
  render(() => <McpSheet {...props} />);
  return { onSave, onClose, setServers };
}
const type = (id: string, value: string) => fireEvent.input(screen.getByTestId(id), { target: { value } });
const saveBtn = () => screen.getByTestId('mcp-sheet-save');

afterEach(() => cleanup());

describe('Form tab', () => {
  it('is a full-height sheet with Form and Paste JSON tabs; Add defaults to the project scope', async () => {
    open();
    const sheet = await screen.findByTestId('mcp-sheet');
    expect(sheet.getAttribute('data-size')).toBe('full');
    expect(screen.getByTestId('mcp-tab-form').getAttribute('aria-selected')).toBe('true');
    expect(screen.getByTestId('mcp-tab-paste')).toBeTruthy();
    expect(screen.getByTestId('mcp-scope-project').getAttribute('aria-checked')).toBe('true');
    expect(screen.getByTestId('mcp-scope-project').textContent).toBe('Project: zm');
  });

  it('adds Linear: HTTP, Authorization is Secret by default and only a ${NAME} reference is sent', async () => {
    const { onSave } = open();
    await screen.findByTestId('mcp-sheet');
    type('mcp-name', 'linear');
    type('mcp-url', 'https://mcp.linear.app/mcp');
    fireEvent.click(screen.getByTestId('mcp-add-row'));
    type('mcp-kv-key', 'Authorization');
    const secret = screen.getByTestId('mcp-kv-secret');
    expect(secret.getAttribute('aria-checked')).toBe('true');
    const value = screen.getByTestId('mcp-kv-value') as HTMLInputElement;
    expect(value.type).toBe('password');
    fireEvent.input(value, { target: { value: 'Bearer lin_api_X' } });
    fireEvent.click(saveBtn());
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    const [scope, entries, secrets] = onSave.mock.calls[0]!;
    expect(scope).toBe('project');
    expect(entries).toEqual({ linear: { url: 'https://mcp.linear.app/mcp', headers: { Authorization: '${LINEAR_AUTHORIZATION}' } } });
    expect(secrets).toEqual({ LINEAR_AUTHORIZATION: 'Bearer lin_api_X' });
    expect(JSON.stringify(entries)).not.toContain('lin_api_X');
  });

  it('a non-secret header key starts with Secret off; turning it on moves the value to the store', async () => {
    const { onSave } = open({ project: undefined });
    await screen.findByTestId('mcp-sheet');
    type('mcp-name', 'svc');
    type('mcp-url', 'https://x.dev/mcp');
    fireEvent.click(screen.getByTestId('mcp-add-row'));
    type('mcp-kv-key', 'X-Team');
    expect(screen.getByTestId('mcp-kv-secret').getAttribute('aria-checked')).toBe('false');
    type('mcp-kv-value', 'MON');
    fireEvent.click(screen.getByTestId('mcp-kv-secret'));
    fireEvent.click(saveBtn());
    await waitFor(() => expect(onSave).toHaveBeenCalled());
    expect(onSave.mock.calls[0]![0]).toBe('global');
    expect(onSave.mock.calls[0]![2]).toEqual({ SVC_X_TEAM: 'MON' });
  });

  it('Command type: command + args + env rows', async () => {
    const { onSave } = open({ project: undefined });
    await screen.findByTestId('mcp-sheet');
    type('mcp-name', 'fs');
    fireEvent.click(screen.getByTestId('mcp-type-command'));
    type('mcp-command', 'npx');
    type('mcp-args', '-y @modelcontextprotocol/server-filesystem "/my dir"');
    fireEvent.click(screen.getByTestId('mcp-add-row'));
    type('mcp-kv-key', 'REGION');
    type('mcp-kv-value', 'us');
    fireEvent.click(saveBtn());
    await waitFor(() => expect(onSave).toHaveBeenCalled());
    expect(onSave.mock.calls[0]![1]).toEqual({ fs: { command: 'npx', args: ['-y', '@modelcontextprotocol/server-filesystem', '/my dir'], env: { REGION: 'us' } } });
  });

  it('validates before saving: nothing is sent, the problems are shown', async () => {
    const { onSave } = open({ taken: () => ['linear'] });
    await screen.findByTestId('mcp-sheet');
    fireEvent.click(saveBtn());
    expect(onSave).not.toHaveBeenCalled();
    expect(screen.getAllByRole('alert').map((a) => a.textContent).join(' ')).toMatch(/name/i);
    type('mcp-name', 'linear');
    type('mcp-url', 'nope');
    expect(screen.getAllByRole('alert').map((a) => a.textContent).join(' ')).toMatch(/already exists/);
    expect(screen.getAllByRole('alert').map((a) => a.textContent).join(' ')).toMatch(/http/);
  });

  it('the project scope is off (with the reason) when the folder is not trusted', async () => {
    open({ project: { cwd: PROJECT.cwd, writable: false, why: 'Trust this project first.' } });
    await screen.findByTestId('mcp-sheet');
    const project = screen.getByTestId('mcp-scope-project') as HTMLButtonElement;
    expect(project.disabled).toBe(true);
    expect(screen.getByTestId('mcp-scope-global').getAttribute('aria-checked')).toBe('true');
  });

  it('shows the host’s refusal and keeps the sheet open', async () => {
    const { onClose } = open({ project: undefined }, async () => ({ ok: false, message: 'Cannot write ~/.oino/mcp.json: read-only file system' }));
    await screen.findByTestId('mcp-sheet');
    type('mcp-name', 'x');
    type('mcp-url', 'https://x.dev');
    fireEvent.click(saveBtn());
    expect((await screen.findByTestId('mcp-sheet-failure')).textContent).toContain('read-only file system');
    expect(onClose).not.toHaveBeenCalled();
  });

  it('after Save it shows each server’s resulting status (connecting → connected / the error), then Done closes', async () => {
    const { onClose, setServers } = open({ project: undefined });
    await screen.findByTestId('mcp-sheet');
    type('mcp-name', 'linear');
    type('mcp-url', 'https://mcp.linear.app/mcp');
    fireEvent.click(saveBtn());
    const saved = await screen.findByTestId('mcp-sheet-saved');
    expect(within(saved).getByTestId('mcp-saved-linear').textContent).toContain('Connecting…');
    const row = { name: 'linear', source: 'global', badge: 'global', readOnly: false, enabled: true, transport: 'http', status: 'error', error: 'LINEAR_AUTHORIZATION is not set — add it in Settings › MCP', toolCount: 0, tools: [] } as McpServer;
    setServers([row]);
    await waitFor(() => expect(screen.getByTestId('mcp-saved-linear').textContent).toContain('LINEAR_AUTHORIZATION is not set'));
    // N7: the error text is printed once (the pill only says "Error").
    expect(screen.getByTestId('mcp-saved-linear').textContent!.match(/LINEAR_AUTHORIZATION is not set/g)).toHaveLength(1);
    setServers([{ ...row, status: 'running', error: undefined, toolCount: 12 }]);
    await waitFor(() => expect(screen.getByTestId('mcp-saved-linear').textContent).toContain('Connected · 12 tools'));
    fireEvent.click(screen.getByTestId('mcp-sheet-done'));
    expect(onClose).toHaveBeenCalled();
  });

  it('shows the host’s secret notice after Save', async () => {
    open({ project: undefined }, async () => ({ ok: true, notice: 'Moved 1 secret-looking value to this PC’s secret store.' }));
    await screen.findByTestId('mcp-sheet');
    type('mcp-name', 'x');
    type('mcp-url', 'https://x.dev');
    fireEvent.click(saveBtn());
    expect((await screen.findByTestId('mcp-sheet-notice')).textContent).toContain('secret store');
  });
});

describe('Paste JSON', () => {
  const linear = { type: 'http', url: 'https://mcp.linear.app/mcp', headers: { Authorization: 'Bearer lin_x' } };
  const paste = async (value: unknown) => {
    fireEvent.click(screen.getByTestId('mcp-tab-paste'));
    fireEvent.input(screen.getByTestId('mcp-paste'), { target: { value: typeof value === 'string' ? value : JSON.stringify(value) } });
  };

  it('one entry: previewed in the form (nothing saved yet); the name is typed in the form', async () => {
    const { onSave } = open({ project: undefined });
    await screen.findByTestId('mcp-sheet');
    await paste(linear);
    expect(screen.getByTestId('mcp-paste-found').textContent).toContain('Found 1 server');
    fireEvent.click(screen.getByTestId('mcp-paste-preview'));
    expect(onSave).not.toHaveBeenCalled();
    expect(screen.getByTestId('mcp-imported')).toBeTruthy();
    expect((screen.getByTestId('mcp-url') as HTMLInputElement).value).toBe('https://mcp.linear.app/mcp');
    expect((screen.getByTestId('mcp-kv-key') as HTMLInputElement).value).toBe('Authorization');
    expect((screen.getByTestId('mcp-kv-value') as HTMLInputElement).type).toBe('password');
    type('mcp-name', 'linear');
    fireEvent.click(saveBtn());
    await waitFor(() => expect(onSave).toHaveBeenCalled());
    expect(onSave.mock.calls[0]![1]).toEqual({ linear: { url: 'https://mcp.linear.app/mcp', headers: { Authorization: '${LINEAR_AUTHORIZATION}' } } });
    expect(onSave.mock.calls[0]![2]).toEqual({ LINEAR_AUTHORIZATION: 'Bearer lin_x' });
  });

  it('{ name: entry }: the name comes with it', async () => {
    open({ project: undefined });
    await screen.findByTestId('mcp-sheet');
    await paste({ linear });
    fireEvent.click(screen.getByTestId('mcp-paste-preview'));
    expect((screen.getByTestId('mcp-name') as HTMLInputElement).value).toBe('linear');
  });

  it('a full { mcpServers } from Claude Code / Cursor: every server is staged, each checked in the form, all saved together', async () => {
    const { onSave } = open({ project: undefined });
    await screen.findByTestId('mcp-sheet');
    await paste({ mcpServers: { linear, fs: { command: 'npx', args: ['-y', 'x'], env: { API_KEY: 'k' } } } });
    expect(screen.getByTestId('mcp-paste-found').textContent).toContain('Found 2 servers');
    fireEvent.click(screen.getByTestId('mcp-paste-preview'));
    expect(screen.getByTestId('mcp-draft-0').textContent).toBe('linear');
    fireEvent.click(screen.getByTestId('mcp-draft-1'));
    expect((screen.getByTestId('mcp-command') as HTMLInputElement).value).toBe('npx');
    expect(screen.getByTestId('mcp-kv-secret').getAttribute('aria-checked')).toBe('true');
    expect(saveBtn().textContent).toBe('Save 2 servers');
    fireEvent.click(saveBtn());
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    expect(Object.keys(onSave.mock.calls[0]![1])).toEqual(['linear', 'fs']);
    expect(onSave.mock.calls[0]![2]).toEqual({ LINEAR_AUTHORIZATION: 'Bearer lin_x', FS_API_KEY: 'k' });
  });

  it('says what is wrong with broken JSON and does not preview', async () => {
    open({ project: undefined });
    await screen.findByTestId('mcp-sheet');
    await paste('{\n  "url": "https://x.dev",\n  oops\n}');
    expect(screen.getByTestId('mcp-paste-error').textContent).toMatch(/not valid JSON/);
    expect((screen.getByTestId('mcp-paste-preview') as HTMLButtonElement).disabled).toBe(true);
  });
});

describe('Edit / Override', () => {
  const stored = (over: Partial<McpServer> = {}): McpServer => ({
    name: 'linear', source: 'project', badge: 'project', sourceFile: '/p/.oino/mcp.json', readOnly: false, enabled: true, transport: 'http', status: 'running', toolCount: 1, tools: [],
    entry: { url: 'https://mcp.linear.app/mcp', headers: { Authorization: '${LINEAR_AUTHORIZATION}', 'X-Team': 'MON' }, enabled: true },
    ...over,
  });

  it('a stored secret shows •••• + Replace — never a value; saving without touching it keeps the reference and sends no secret', async () => {
    const { onSave } = open({ mode: { kind: 'edit', server: stored() } });
    await screen.findByTestId('mcp-sheet');
    expect((screen.getByTestId('mcp-name') as HTMLInputElement).readOnly).toBe(true);
    const masked = screen.getByTestId('mcp-kv-masked');
    expect(masked.textContent).toContain('••••');
    expect(masked.textContent).not.toContain('LINEAR');
    expect(screen.getByTestId('mcp-kv-replace')).toBeTruthy();
    // No Paste tab when editing.
    expect(screen.queryByTestId('mcp-tab-paste')).toBeNull();
    fireEvent.click(saveBtn());
    await waitFor(() => expect(onSave).toHaveBeenCalled());
    expect(onSave.mock.calls[0]![1]).toEqual({ linear: { url: 'https://mcp.linear.app/mcp', headers: { Authorization: '${LINEAR_AUTHORIZATION}', 'X-Team': 'MON' }, enabled: true } });
    expect(onSave.mock.calls[0]![2]).toEqual({});
  });

  it('Replace opens an empty secret field; the new value goes to the store, the file keeps the reference', async () => {
    const { onSave } = open({ mode: { kind: 'edit', server: stored() } });
    await screen.findByTestId('mcp-sheet');
    fireEvent.click(screen.getByTestId('mcp-kv-replace'));
    const field = screen.getAllByTestId('mcp-kv-value')[0] as HTMLInputElement;
    expect(field.type).toBe('password');
    expect(field.value).toBe('');
    fireEvent.input(field, { target: { value: 'Bearer fresh' } });
    fireEvent.click(saveBtn());
    await waitFor(() => expect(onSave).toHaveBeenCalled());
    expect((onSave.mock.calls[0]![1] as Record<string, { headers: Record<string, string> }>).linear!.headers.Authorization).toBe('${LINEAR_AUTHORIZATION}');
    expect(onSave.mock.calls[0]![2]).toEqual({ LINEAR_AUTHORIZATION: 'Bearer fresh' });
  });

  it('Override in Oino: the read-only entry is copied into the project scope, the name locked', async () => {
    const { onSave } = open({ mode: { kind: 'override', server: stored({ badge: 'project', readOnly: true, sourceFile: '/p/.mcp.json' }) } });
    await screen.findByTestId('mcp-sheet');
    expect((screen.getByTestId('mcp-name') as HTMLInputElement).readOnly).toBe(true);
    expect(screen.getByTestId('mcp-scope-project').getAttribute('aria-checked')).toBe('true');
    fireEvent.click(saveBtn());
    await waitFor(() => expect(onSave).toHaveBeenCalled());
    expect(onSave.mock.calls[0]![0]).toBe('project');
  });

  it('editing a global server from a project page keeps it in the global scope', async () => {
    const { onSave } = open({ mode: { kind: 'edit', server: stored({ badge: 'global', source: 'global' }) } });
    await screen.findByTestId('mcp-sheet');
    expect(screen.getByTestId('mcp-scope-global').getAttribute('aria-checked')).toBe('true');
    expect((screen.getByTestId('mcp-scope-project') as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(saveBtn());
    await waitFor(() => expect(onSave.mock.calls[0]![0]).toBe('global'));
  });

  it('a host that did not send the entry says so instead of showing fields that look saved', async () => {
    open({ mode: { kind: 'edit', server: stored({ entry: undefined }) } });
    expect((await screen.findByTestId('mcp-no-entry')).textContent).toMatch(/did not send/);
  });
});


it('OIN-101 D4 catalog search stays in the Add sheet and selects the clicked unique entry', async () => {
  const onCatalog = vi.fn().mockResolvedValue([
    { id: 'one/repo', name: 'first', label: 'First', description: 'One', entry: { command: 'first-command' } },
    { id: 'two/repo', name: 'second', label: 'Second', description: 'Two', entry: { command: 'second-command' } },
  ]);
  open({ onCatalog });
  await screen.findByTestId('mcp-sheet');
  fireEvent.click(screen.getByTestId('mcp-browse-catalog'));
  fireEvent.input(screen.getByTestId('mcp-catalog-search'), { target: { value: 'repo' } });
  await waitFor(() => expect(onCatalog).toHaveBeenLastCalledWith('repo'));
  fireEvent.click(await screen.findByText('Second'));
  expect((screen.getByTestId('mcp-name') as HTMLInputElement).value).toBe('second');
  expect((screen.getByTestId('mcp-command') as HTMLInputElement).value).toBe('second-command');
});
it('OIN-101 D13 cannot save directly from Paste JSON', async () => {
  open({});
  await screen.findByTestId('mcp-sheet');
  fireEvent.click(screen.getByTestId('mcp-tab-paste'));
  expect((saveBtn() as HTMLButtonElement).disabled).toBe(true);
});

describe('OIN-101 UI2: catalog (N3) and placeholders (N4)', () => {
  const pg = { id: 'pg', name: 'postgresql', label: 'PostgreSQL', description: 'PostgreSQL database access', entry: { command: 'npx', args: ['-y', '@modelcontextprotocol/server-postgres'], env: { POSTGRES_CONNECTION_STRING: '${POSTGRES_CONNECTION_STRING}' } } };

  it('N3: Browse catalog is a secondary button beside the tabs; a whole catalog row is one button', async () => {
    const onCatalog = vi.fn().mockResolvedValue([pg]);
    open({ onCatalog });
    await screen.findByTestId('mcp-sheet');
    const browse = screen.getByTestId('mcp-browse-catalog');
    expect(browse.classList.contains('btn-secondary')).toBe(true);
    expect(browse.closest('.mcps-tabrow')).toBeTruthy();
    fireEvent.click(browse);
    const row = await screen.findByText('PostgreSQL database access');
    // Clicking the description (not the name) picks the entry.
    fireEvent.click(row);
    expect((screen.getByTestId('mcp-name') as HTMLInputElement).value).toBe('postgresql');
  });

  it('N4: a catalog ${VAR} placeholder is an empty required value, never "•••• Replace"; Save is blocked until it is filled', async () => {
    const onCatalog = vi.fn().mockResolvedValue([pg]);
    const { onSave } = open({ onCatalog, project: undefined });
    await screen.findByTestId('mcp-sheet');
    fireEvent.click(screen.getByTestId('mcp-browse-catalog'));
    fireEvent.click(await screen.findByText('PostgreSQL'));
    expect(screen.queryByTestId('mcp-kv-masked')).toBeNull();
    expect(screen.queryByTestId('mcp-kv-replace')).toBeNull();
    const value = screen.getByTestId('mcp-kv-value') as HTMLInputElement;
    expect(value.value).toBe('');
    expect(value.placeholder).toBe('Required');
    expect(screen.getByTestId('mcp-kv-required').textContent).toContain('POSTGRES_CONNECTION_STRING');
    expect((saveBtn() as HTMLButtonElement).disabled).toBe(true);
    fireEvent.input(value, { target: { value: 'postgres://u:p@h/db' } });
    expect(screen.queryByTestId('mcp-kv-required')).toBeNull();
    expect((saveBtn() as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(saveBtn());
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    expect(onSave.mock.calls[0]![2]).toEqual({ POSTGRESQL_POSTGRES_CONNECTION_STRING: 'postgres://u:p@h/db' });
  });

  it('N4: a stored secret of an existing server still shows •••• Replace', async () => {
    const stored = { name: 'x', source: 'global', badge: 'global', readOnly: false, enabled: true, transport: 'stdio', status: 'running', toolCount: 0, tools: [], entry: { command: 'npx', env: { X_TOKEN: '${X_TOKEN}' } } } as McpServer;
    open({ mode: { kind: 'edit', server: stored }, project: undefined });
    expect(await screen.findByTestId('mcp-kv-masked')).toBeTruthy();
    expect((saveBtn() as HTMLButtonElement).disabled).toBe(false);
  });
});
