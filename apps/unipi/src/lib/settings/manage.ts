/**
 * Settings › Agent capabilities "Manage…" actions (WP-A25, G-8..G-11).
 *
 * Each capability page (MCP, Memory, Skills, Subagents) shows the extension's
 * settings form (U11) plus the extension's own **panels**: the page's buttons
 * run the extension command in an open chat, which opens its UI v2 panel there
 * (U10 renders it) — one surface per feature, no second list in Settings.
 * The same actions are command-center entries (`components/command/
 * BuiltinActions.tsx`), so `/mcp-settings` is reachable with no chat view.
 *
 * R8: a panel needs a chat. With none open, the buttons say "Open a chat to
 * manage" and a one-click "New chat here" starts one (the settings forms keep
 * working without a chat).
 */
import { sessions } from '../session/store';
import { currentSessionRoute } from '../../components/shell/AppShell';
import { openSettingsAt } from './open';

/** One Manage action: a page, the extension command it runs, and its palette wording. */
export type ManageAction = {
  id: string;
  /** Settings page that lists it (`settings/<page>`). */
  page: 'mcp' | 'memory' | 'skills' | 'subagents';
  /** Button label on the page. */
  label: string;
  /** Description under the button (what panel opens). */
  description: string;
  /** The extension command, without the slash. */
  command: string;
  /** Command-center wording (the same action from the palette). */
  title: string;
  keywords?: string;
  /** Group in the command center. */
  group: string;
};

/**
 * The harness actions behind the capability pages. Command names are the
 * extensions' own (`/mcp-status`, `/memory`, `/skills-editor`, `/agents`, …),
 * so an extension update changes its panels without an app release.
 */
export const MANAGE_ACTIONS: ManageAction[] = [
  {
    id: 'mcp.status',
    page: 'mcp',
    label: 'Server status',
    description: 'Every configured server, its tools and its connection',
    command: 'mcp-status',
    title: 'MCP: server status',
    keywords: 'mcp status servers tools connection',
    group: 'Agent',
  },
  {
    id: 'mcp.servers',
    page: 'mcp',
    label: 'Manage servers',
    description: 'List, enable, edit or remove a server',
    command: 'mcp-settings',
    title: 'MCP: manage servers',
    keywords: 'mcp servers add remove enable edit json',
    group: 'Agent',
  },
  {
    id: 'mcp.catalog',
    page: 'mcp',
    label: 'Add from the catalog',
    description: 'Browse the server catalog and add one',
    command: 'mcp-add',
    title: 'MCP: add a server from the catalog',
    keywords: 'mcp catalog add install',
    group: 'Agent',
  },
  {
    id: 'mcp.sync',
    page: 'mcp',
    label: 'Sync the catalog',
    description: 'Refresh the catalog from GitHub',
    command: 'mcp-sync',
    title: 'MCP: sync the catalog',
    keywords: 'mcp catalog sync refresh',
    group: 'Agent',
  },
  {
    id: 'mcp.reload',
    page: 'mcp',
    label: 'Reload servers',
    description: 'Reconnect every server and re-register its tools',
    command: 'mcp-reload',
    title: 'MCP: reload servers',
    keywords: 'mcp reload reconnect tools',
    group: 'Agent',
  },
  {
    id: 'memory.browse',
    page: 'memory',
    label: 'Browse memories',
    description: 'List, search and forget what this project remembers',
    command: 'memory',
    title: 'Memory: browse memories',
    keywords: 'memory browse list search forget remember',
    group: 'Agent',
  },
  {
    id: 'memory.consolidate',
    page: 'memory',
    label: 'Consolidate this session',
    description: 'Let the agent write what this chat is worth remembering',
    command: 'memory-consolidate',
    title: 'Memory: consolidate this session',
    keywords: 'memory consolidate save summarize',
    group: 'Agent',
  },
  {
    id: 'memory.global',
    page: 'memory',
    label: 'All projects',
    description: 'Every memory across every project',
    command: 'global-memory-list',
    title: 'Memory: list every project',
    keywords: 'memory global all projects list',
    group: 'Agent',
  },
  {
    id: 'skills.hub',
    page: 'skills',
    label: 'Skills hub',
    description: 'Exposure, pins and the vault (the extension’s own panel)',
    command: 'skills',
    title: 'Skills: open the hub',
    keywords: 'skills exposure pins vault hub',
    group: 'Agent',
  },
  {
    id: 'skills.editor',
    page: 'skills',
    label: 'Skill editor',
    description: 'Enabled / Must show, per skill',
    command: 'skills-editor',
    title: 'Skills: edit enabled and pinned skills',
    keywords: 'skills editor enabled must show pins',
    group: 'Agent',
  },
  {
    id: 'agents.profiles',
    page: 'subagents',
    label: 'Manage profiles',
    description: 'List, create, edit, copy or delete agent profiles',
    command: 'agents',
    title: 'Subagents: manage profiles',
    keywords: 'agents subagents profiles create edit delete',
    group: 'Agent',
  },
];

/** The Actions of one capability page, in table order. */
export function manageActionsFor(page: ManageAction['page']): ManageAction[] {
  return MANAGE_ACTIONS.filter((action) => action.page === page);
}

/** The chat a Manage action runs in: the shown session, else the most recently used open one. */
export function manageChat(): { hostId: string; sessionId: string } | undefined {
  const shown = currentSessionRoute();
  if (shown && sessions().peek(shown.hostId, shown.sessionId)) return shown;
  // `keys()` is least-recently-used first: the last key is the chat the user
  // was in before Settings.
  const last = sessions().keys().at(-1);
  if (!last) return shown;
  const slash = last.indexOf('/');
  return { hostId: last.slice(0, slash), sessionId: last.slice(slash + 1) };
}

/** `/s/:host/:session` for a chat. */
export function chatHref(hostId: string, sessionId: string): string {
  return `/s/${encodeURIComponent(hostId)}/${encodeURIComponent(sessionId)}`;
}

export type ManageOutcome = 'sent' | 'no-chat' | 'missing';

/** The chat's own command list carries `name` (a leading slash is tolerated: fixtures use it). */
export function chatHasCommand(handle: { state: { commands: ReadonlyArray<{ name: string }> } }, command: string): boolean {
  return handle.state.commands.some((c) => c.name.replace(/^\//, '') === command);
}

/**
 * Runs a Manage action: sends `/<command>` through the chat's stream (an
 * extension command never becomes a prompt — the composer's rule) and shows
 * the chat so the panel the extension opens is visible.
 */
export function runManageAction(action: ManageAction, deps: { navigate: (href: string) => void }): ManageOutcome {
  // `/skills` fires `openSettings{namespace:"skills"}` (the Skills page itself),
  // never a chat-resident overlay: run it in place when a chat is open, and open
  // Settings › Skills directly when none is.
  if (action.id === 'skills.hub') {
    const chat = manageChat();
    const handle = chat ? sessions().peek(chat.hostId, chat.sessionId) : undefined;
    if (handle) {
      if (handle.state.commands.length > 0 && !chatHasCommand(handle, action.command)) return 'missing';
      handle.send(`/${action.command}`, { mode: 'prompt' });
      return 'sent';
    }
    openSettingsAt('skills', 'sec-skills');
    return 'sent';
  }
  const chat = manageChat();
  if (!chat) return 'no-chat';
  const handle = sessions().peek(chat.hostId, chat.sessionId);
  if (!handle) return 'no-chat';
  // A disabled extension has no command: say so instead of sending prompt text.
  if (handle.state.commands.length > 0 && !chatHasCommand(handle, action.command)) return 'missing';
  handle.send(`/${action.command}`, { mode: 'prompt' });
  deps.navigate(chatHref(chat.hostId, chat.sessionId));
  return 'sent';
}

/** The R8 wording every chat-bound button shows with no chat open. */
export const NEEDS_CHAT = 'Open a chat to manage';

/** A Manage action whose extension is off (or not installed on this host). */
export const MISSING_EXTENSION = 'That extension is off — turn it on in Settings › Extensions';
