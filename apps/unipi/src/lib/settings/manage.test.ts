/**
 * WP-A25: the capability pages' "Manage…" actions — which chat they run in,
 * what they send, and the two R8 outcomes (no chat / extension off).
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createMockHost, type MockWorld } from '../../dev/mock/host';
import { capMcpNoChat } from '../../dev/mock/scenarios/capabilities';
import { setHostClientFactory } from '../host/factory';
import { resetSessions, sessions } from '../session/store';
import { CONSOLIDATE_COMMAND, MANAGE_ACTIONS, MISSING_EXTENSION, NEEDS_CHAT, chatCanRun, chatHasCommand, manageActionsFor, manageChat, runInChat, runManageAction } from './manage';

/**
 * Opens the scenario's chat in the registry (live, so its command list is the
 * host's). `commands` replaces the session's list — a host the extension is off
 * on answers without its command.
 */
async function openChat(commands?: string[]): Promise<MockWorld> {
  const scenario = commands
    ? {
        ...capMcpNoChat,
        hosts: capMcpNoChat.hosts.map((host) => ({
          ...host,
          sessions: Object.fromEntries(
            Object.entries(host.sessions ?? {}).map(([id, session]) => [
              id,
              { ...session, snapshot: { ...session.snapshot, commands: commands.map((name) => ({ name, source: 'extension' })) } },
            ]),
          ),
        })),
      }
    : capMcpNoChat;
  const world = createMockHost(scenario);
  setHostClientFactory(world.client);
  const handle = sessions().ensure('local', 'cap001');
  const deadline = Date.now() + 3000;
  while (handle.state.conn !== 'live' && Date.now() < deadline) await new Promise((r) => setTimeout(r, 10));
  return world;
}

afterEach(() => {
  resetSessions();
  setHostClientFactory(undefined);
  vi.restoreAllMocks();
});

describe('the Manage table', () => {
  it('names the extension commands the four pages run', () => {
    expect(manageActionsFor('mcp').map((a) => a.command)).toEqual(['mcp-status', 'mcp-settings', 'mcp-add', 'mcp-sync', 'mcp-reload']);
    // memory-consolidate prompts a chat: it is the chat's (⋯ menu, /memory-consolidate), not a Settings row.
    expect(manageActionsFor('memory').map((a) => a.command)).toEqual(['memory', 'global-memory-list', 'dream status', 'dream report', 'dream run']);
    expect(manageActionsFor('chat').map((a) => a.command)).toEqual(['memory-consolidate']);
    expect(manageActionsFor('skills').map((a) => a.command)).toEqual([]);
    expect(manageActionsFor('subagents').map((a) => a.command)).toEqual(['agents']);
  });

  it('every action belongs to one of the four pages and has a unique id', () => {
    const ids = MANAGE_ACTIONS.map((a) => a.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(MANAGE_ACTIONS.every((a) => ['mcp', 'memory', 'skills', 'subagents', 'chat'].includes(a.page))).toBe(true);
  });

  it('chatHasCommand tolerates a leading slash (fixtures carry one)', () => {
    expect(chatHasCommand({ state: { commands: [{ name: '/memory' }] } }, 'memory')).toBe(true);
    expect(chatHasCommand({ state: { commands: [{ name: 'memory' }] } }, 'memory')).toBe(true);
    // P1-07: a command with arguments (`dream run`) needs only its command.
    expect(chatHasCommand({ state: { commands: [{ name: 'dream' }] } }, 'dream run')).toBe(true);
    expect(chatHasCommand({ state: { commands: [{ name: 'mcp' }] } }, 'memory')).toBe(false);
  });
});

describe('runManageAction', () => {
  it('runs the command in the most recently used open chat and shows it', async () => {
    const world = await openChat();
    const navigate = vi.fn();
    expect(runManageAction(manageActionsFor('mcp')[1]!, { navigate })).toBe('sent');
    expect(navigate).toHaveBeenCalledWith('/s/local/cap001');
    expect(world.log.some((entry) => entry.kind === 'cmd' && entry.op === 'prompt' && (entry.args as { message?: string }).message === '/mcp-settings')).toBe(true);
  });

  it('no open chat ⇒ “no-chat” (the page offers New chat here, R8)', () => {
    setHostClientFactory(createMockHost(capMcpNoChat).client);
    expect(manageChat()).toBeUndefined();
    expect(runManageAction(manageActionsFor('mcp')[0]!, { navigate: vi.fn() })).toBe('no-chat');
  });

  it('a disabled extension (no such command) says so instead of sending prompt text', async () => {
    await openChat(['model']);
    expect(runManageAction(manageActionsFor('mcp')[0]!, { navigate: vi.fn() })).toBe('missing');
    expect(MISSING_EXTENSION).toMatch(/Settings › Extensions/);
  });

  it('the R8 wording is the one the pages render', () => {
    expect(NEEDS_CHAT).toBe('Open a chat to manage');
  });
});

describe('the chat ⋯ Consolidate memories row (owner 2026-11-03)', () => {
  it('sends /memory-consolidate into that chat, only where the chat lists it', async () => {
    const world = await openChat(['memory', 'memory-consolidate']);
    expect(chatCanRun('local', 'cap001', CONSOLIDATE_COMMAND)).toBe(true);
    expect(runInChat('local', 'cap001', CONSOLIDATE_COMMAND)).toBe(true);
    expect(world.log.some((entry) => entry.kind === 'cmd' && entry.op === 'prompt' && (entry.args as { message?: string }).message === '/memory-consolidate')).toBe(true);
    resetSessions();
    await openChat(['model']);
    expect(chatCanRun('local', 'cap001', CONSOLIDATE_COMMAND)).toBe(false);
    expect(runInChat('local', 'cap001', CONSOLIDATE_COMMAND)).toBe(false);
  });
});
