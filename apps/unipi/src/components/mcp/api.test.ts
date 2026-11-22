import { beforeEach, expect, it, vi } from 'vitest';
const { cmd, chat, request, peek, keys, cards } = vi.hoisted(() => ({ cmd: vi.fn(), chat: vi.fn(), request: vi.fn(), peek: vi.fn(), keys: vi.fn(), cards: { value: {} as Record<string, unknown> } }));
vi.mock('../../lib/session/hub', () => ({ hubs: () => ({ ensure: () => ({ state: { cards: cards.value } }) }) }));
vi.mock('../../lib/host/factory', () => ({ setHostClientFactory: () => {}, hostClient: () => ({ request }) }));
vi.mock('../../lib/session/store', () => ({ sessions: () => ({ keys, peek }) }));
vi.mock('../../lib/host/localId', () => ({ resolveHostId: (id: string) => id }));
vi.mock('../../lib/settings/run', () => ({ settingsCommandChat: chat }));
import { loadMcp, mcpAction, mcpAutonomyContext, projectChat } from './api';
import { rememberAutonomy, resetRememberedAutonomy } from '../../lib/session/chatAutonomy';
beforeEach(() => { cmd.mockReset(); chat.mockReset(); request.mockReset(); request.mockResolvedValue({ value: 'medium' }); keys.mockReturnValue([]); peek.mockReset(); cards.value = {}; resetRememberedAutonomy(); chat.mockResolvedValue({ cmd }); });
it('uses host saved/undecided trust rather than extension effective trust', async () => {
  request.mockResolvedValueOnce({ servers: [], files: [{ scope: 'project', path: '/p/.mcp.json' }], trust: { state: 'untrusted', path: '/p/' } });
  request.mockResolvedValueOnce({ state: 'undecided', path: '/p', protected: ['.mcp.json'] });
  expect((await loadMcp({ hostId: 'h', cwd: '/p' })).trust).toEqual({ state: 'undecided', path: '/p', protected: ['.mcp.json'] });
  expect(chat).not.toHaveBeenCalled();
  expect(request).toHaveBeenCalledWith('host_cmd', { cmd: { kind: 'mcp_servers', cwd: '/p' } });
});
it('marks project reload explicitly so host does not fan it out globally', async () => {
  cmd.mockResolvedValue({ ok: true });
  await mcpAction({ hostId: 'h', cwd: '/p' }, 'reload');
  expect(cmd).toHaveBeenLastCalledWith({ type: '_oino/service_call', name: 'mcp.server-action', payload: { cwd: '/p', action: 'reload', scope: 'project' } });
  await mcpAction({ hostId: 'h' }, 'reload');
  expect(cmd).toHaveBeenLastCalledWith({ type: '_oino/service_call', name: 'mcp.server-action', payload: { action: 'reload', scope: 'global' } });
});

it('Reconnect uses the open project chat, including its changed autonomy', async () => {
  const liveCmd = vi.fn().mockResolvedValue({ ok: true });
  keys.mockReturnValue(['h/chat']);
  peek.mockReturnValue({ state: { conn: 'live', info: { cwd: '/p', autonomy: 'high' } }, cmd: liveCmd });
  await mcpAction({ hostId: 'h', cwd: '/p' }, 'reconnect', 'refero');
  expect(chat).not.toHaveBeenCalled();
  expect(liveCmd).toHaveBeenCalledWith({ type: '_oino/service_call', name: 'mcp.server-action', payload: { cwd: '/p', scope: 'project', action: 'reconnect', name: 'refero' } });
});
it('a scratch action re-reads default autonomy on each reconnect', async () => {
  request.mockResolvedValue({ value: 'high' });
  await mcpAction({ hostId: 'h', cwd: '/p' }, 'reconnect', 'refero');
  expect(cmd).toHaveBeenCalledWith({ type: 'set_autonomy', level: 'high' });
});

const liveChat = (over: Record<string, unknown>, c = vi.fn().mockResolvedValue({ ok: true })) => ({ state: { conn: 'live', name: 'live chat', info: { cwd: '/p', autonomy: 'medium', ...over } }, cmd: c });

it('N2: the most recently used live chat of the project decides, by its title', () => {
  keys.mockReturnValue(['h/old', 'h/new']);
  peek.mockImplementation((_h: string, id: string) => (id === 'new' ? liveChat({}) : liveChat({})));
  expect(projectChat('h', '/p')).toMatchObject({ level: 'medium', title: 'live chat' });
  expect(mcpAutonomyContext('h', '/p')).toBe('Autonomy medium · from chat “live chat”');
});

it('N2: never lower than any open chat: a High chat the registry already evicted still wins over a Medium live chat', async () => {
  keys.mockReturnValue(['h/live']);
  peek.mockReturnValue(liveChat({ autonomy: 'medium' }));
  cards.value = { evicted: { session_id: 'evicted', cwd: '/p', name: 'zm chat', run_state: 'idle', last_activity: 9 } };
  rememberAutonomy('h', 'evicted', 'high');
  expect(projectChat('h', '/p')).toMatchObject({ level: 'high', title: 'zm chat' });
  // The Settings action runs on the scratch chat AT that level, not at the Settings default.
  request.mockResolvedValue({ value: 'low' });
  await mcpAction({ hostId: 'h', cwd: '/p' }, 'reload');
  expect(cmd).toHaveBeenCalledWith({ type: 'set_autonomy', level: 'high' });
  expect(cmd).not.toHaveBeenCalledWith({ type: 'set_autonomy', level: 'low' });
});

it('N2: a chat whose card is gone (closed) no longer counts', () => {
  rememberAutonomy('h', 'gone', 'high');
  expect(projectChat('h', '/p')).toBeUndefined();
  expect(mcpAutonomyContext('h', '/p', 'medium')).toBe('Autonomy medium · Settings default (no open chat in this project)');
});

it('N2: a chat of another project does not count', () => {
  cards.value = { other: { session_id: 'other', cwd: '/q', name: 'other', run_state: 'idle' } };
  rememberAutonomy('h', 'other', 'high');
  expect(projectChat('h', '/p')).toBeUndefined();
});

it('N5/N6: no attribution on Global; an untitled chat is never shown by its id', () => {
  expect(mcpAutonomyContext('h', undefined)).toBeUndefined();
  cards.value = { '65a35935-92d5-59e3-a8c8-6908ccfef16a': { session_id: '65a35935-92d5-59e3-a8c8-6908ccfef16a', cwd: '/p', name: null, run_state: 'idle' } };
  rememberAutonomy('h', '65a35935-92d5-59e3-a8c8-6908ccfef16a', 'medium');
  expect(mcpAutonomyContext('h', '/p')).toBe('Autonomy medium · from chat “Untitled chat”');
});
