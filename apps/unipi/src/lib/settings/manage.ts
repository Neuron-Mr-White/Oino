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
 * R8 / P1-07: a panel needs a chat. With none open, `runManageActionAnywhere`
 * opens this PC's most recent chat (or starts an empty one when there is
 * none), waits for its commands and runs the action there — the button always
 * works; the settings forms keep working without a chat at all.
 */
import { sessions } from '../session/store';
import { hubs } from '../session/hub';
import { newSession } from '../session/newSession';
import { noteCommandRun } from '../reports';
import { currentSessionRoute } from '../../components/shell/AppShell';
import { openSettingsAt } from './open';

/** One Manage action: a page, the extension command it runs, and its palette wording. */
export type ManageAction = {
  id: string;
  /** Settings page that lists it (`settings/<page>`). */
  page: 'mcp' | 'memory' | 'skills' | 'subagents' | 'about';
  /** Button label on the page. */
  label: string;
  /** Description under the button (what panel opens). */
  description: string;
  /** The extension command (and its arguments), without the slash. */
  command: string;
  /** Command-center wording (the same action from the palette). */
  title: string;
  keywords?: string;
  /** Group in the command center. */
  group: string;
  /** A hidden extension command (not in the chat's command list): run it without the listed-check. */
  hidden?: boolean;
  /** The extension that owns the command (default: derived from it, `lib/settings/run.ts`). */
  extension?: string;
  /** Settings opens this settings namespace's form in a sheet instead of running the command. */
  form?: string;
};

/** P1-08: the updater extension's changelog browser (About › What's new → "Browse changelog"). */
export const CHANGELOG_ACTION: ManageAction = {
  id: 'updater.changelog',
  page: 'about',
  label: 'Browse changelog',
  description: 'Every release, newest first',
  command: 'changelog',
  title: 'Browse the changelog',
  group: 'App',
  hidden: true,
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
  // P1-07: Dream (background reflection over past sessions → memory lessons and
  // proposals) has no idle tray tab; its actions live with Memory.
  {
    id: 'dream.status',
    page: 'memory',
    label: 'Dream status',
    description: 'Last dream, whether one is due, proposals waiting',
    command: 'dream status',
    title: 'Dream: status',
    keywords: 'dream status reflection consolidation proposals',
    group: 'Agent',
  },
  {
    id: 'dream.report',
    page: 'memory',
    label: 'Latest dream report',
    description: 'What the last dream learned and proposed',
    command: 'dream report',
    title: 'Dream: latest report',
    keywords: 'dream report past dreams proposals lessons',
    group: 'Agent',
  },
  {
    id: 'dream.run',
    page: 'memory',
    label: 'Dream now',
    description: 'Reflect over recent sessions in the background',
    command: 'dream run',
    title: 'Dream: run now',
    keywords: 'dream run now reflect consolidate background',
    group: 'Agent',
  },
  {
    id: 'skills.hub',
    page: 'skills',
    label: 'Skills hub',
    description: 'Exposure, pins and the vault',
    command: 'skills',
    form: 'skills',
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

/** Sends the action's command; a multi-line reply opens the report panel titled with it (P1-02). */
function sendAction(handle: { key: string; send: (text: string, opts: { mode: 'prompt' }) => unknown }, action: ManageAction): void {
  noteCommandRun(handle.key, action.command);
  handle.send(`/${action.command}`, { mode: 'prompt' });
}

/** The chat's own command list carries `name` (a leading slash is tolerated: fixtures use it). */
export function chatHasCommand(handle: { state: { commands: ReadonlyArray<{ name: string }> } }, command: string, hidden = false): boolean {
  if (hidden) return true;
  // `dream run` → the `dream` command with an argument.
  const name = command.split(' ')[0]!;
  return handle.state.commands.some((c) => c.name.replace(/^\//, '') === name);
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
      if (handle.state.commands.length > 0 && !chatHasCommand(handle, action.command, action.hidden)) return 'missing';
      sendAction(handle, action);
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
  if (handle.state.commands.length > 0 && !chatHasCommand(handle, action.command, action.hidden)) return 'missing';
  sendAction(handle, action);
  deps.navigate(chatHref(chat.hostId, chat.sessionId));
  return 'sent';
}

/** The most recently active, non-cron session on `hostId` (from the hub's cards), if any. */
export function latestChatOn(hostId: string): string | undefined {
  const cards = Object.values(hubs().ensure(hostId).state.cards).filter(
    (card) => card.source !== 'cron' && (card as { archived?: unknown }).archived !== true,
  );
  cards.sort((a, b) => (b.last_activity ?? b.updated_at ?? 0) - (a.last_activity ?? a.updated_at ?? 0));
  return cards[0]?.session_id;
}

/** Resolves once the chat lists its commands (the extension panels are commands), or soon after
 *  it is live without any (an old host), or after `ms`. */
function commandsReady(handle: { state: { commands: ReadonlyArray<unknown>; conn?: string } }, ms = 6000): Promise<void> {
  return new Promise((resolve) => {
    const start = Date.now();
    let liveAt: number | undefined;
    const tick = () => {
      const now = Date.now();
      if (handle.state.conn === 'live') liveAt ??= now;
      if (handle.state.commands.length > 0 || now - start > ms || (liveAt !== undefined && now - liveAt > 1200)) resolve();
      else setTimeout(tick, 80);
    };
    tick();
  });
}

/**
 * P1-07: a Manage action from anywhere (Settings, the command center) with no
 * chat needed up front: with none open it opens this PC's latest chat — or
 * starts an empty one — and runs the action there once its commands load.
 */
export async function runManageActionAnywhere(
  action: ManageAction,
  deps: { navigate: (href: string) => void; hostId: string; start?: typeof newSession },
): Promise<ManageOutcome> {
  const first = runManageAction(action, deps);
  if (first !== 'no-chat') return first;
  const hub = hubs().ensure(deps.hostId);
  if (!hub.state.loaded) await hub.reload().catch(() => undefined);
  let sessionId = latestChatOn(deps.hostId);
  if (!sessionId) {
    const created = await (deps.start ?? newSession)(deps.hostId);
    if (!created.ok) return 'no-chat';
    sessionId = created.value.session;
  }
  deps.navigate(chatHref(deps.hostId, sessionId));
  const handle = sessions().ensure(deps.hostId, sessionId);
  await commandsReady(handle);
  if (handle.state.commands.length > 0 && !chatHasCommand(handle, action.command, action.hidden)) return 'missing';
  sendAction(handle, action);
  return 'sent';
}

/** The R8 wording every chat-bound button shows with no chat open. */
export const NEEDS_CHAT = 'Open a chat to manage';

/** A Manage action whose extension is off (or not installed on this host). */
export const MISSING_EXTENSION = 'That extension is off — turn it on in Settings › Extensions';
