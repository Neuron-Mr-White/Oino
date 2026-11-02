/**
 * The Settings catalog (WP-A24, Z-113): the nav's groups and sections plus the
 * static row labels the global search indexes. Dynamic keys — host settings
 * (`config.json`), pi agent settings and extension namespaces — join at search
 * time from the host (`settings/hostCmd.ts`); this file is the app's own map.
 *
 * Groups (ZCode settings-sidebar-layout, in the unipi look):
 * Basics / Agent capabilities / Hosts & devices / Data / About.
 */
import type { Component } from 'solid-js';
import {
  Blocks, Bell, Bot, Brain, Cpu, FolderKanban, Globe, Info, KanbanSquare, Keyboard, MonitorSmartphone,
  Plug, Puzzle, Server, Settings2, Smartphone, Sparkles, TerminalSquare, Type, Unplug, Waypoints,
} from 'lucide-solid';
import { can, type Capability } from '../../lib/platform';

export type SettingsGroupId = 'basics' | 'agent' | 'hosts' | 'data' | 'about';

export type SettingsSectionId =
  | 'general' | 'appearance' | 'input' | 'shortcuts' | 'terminal' | 'editor' | 'board'
  | 'extensions' | 'agent' | 'mcp' | 'memory' | 'skills' | 'subagents' | 'providers'
  | 'hosts' | 'connection' | 'sharing' | 'notifications' | 'unicrab'
  | 'host'
  | 'ports'
  | 'about';

/** Phone Settings root groups (phone spec §3.9): this phone, the selected PC, PCs & connection, About. */
export type PhoneGroupId = 'phone' | 'pc' | 'pcs' | 'about';

export type SettingsRow = { key: string; label: string; description?: string };

export type SettingsSection = {
  id: SettingsSectionId;
  title: string;
  group: SettingsGroupId;
  icon: Component<{ size?: number | string }>;
  /** `can()` capability the whole section needs (web hides what needs the app). */
  cap?: Capability;
  /** Desktop-only sections (General/Lifetime needs the tray, Host config the local host). */
  desktopOnly?: boolean;
  /** Extra runtime gate (e.g. the Unicrab surfaces exist on phone/desktop only). */
  show?: () => boolean;
  /** Phone root (§3.9): which group lists it and its row label there; `false` = hidden on the phone (PH-20). */
  phone?: { group: PhoneGroupId; label?: string } | false;
  /** Only the phone lists it (Ports live in the desktop side pane, Z-104). */
  phoneOnly?: boolean;
  keywords: string[];
  rows: SettingsRow[];
};

export const SETTINGS_GROUPS: Array<{ id: SettingsGroupId; title: string }> = [
  { id: 'basics', title: 'Basics' },
  { id: 'agent', title: 'Agent capabilities' },
  { id: 'hosts', title: 'Hosts & devices' },
  { id: 'data', title: 'Data' },
  { id: 'about', title: 'About' },
];

export const SETTINGS_SECTIONS: SettingsSection[] = [
  {
    id: 'general',
    title: 'General',
    group: 'basics',
    icon: Settings2,
    desktopOnly: true,
    phone: false,
    keywords: ['lifetime', 'close', 'quit', 'tray', 'login', 'startup', 'autostart', 'window'],
    rows: [
      { key: 'closeAction', label: 'Close button', description: 'Ask, minimize to the tray, or quit' },
      { key: 'autostart', label: 'Start at login', description: 'Starts hidden in the tray' },
    ],
  },
  {
    id: 'appearance',
    title: 'Appearance',
    group: 'basics',
    icon: Type,
    phone: { group: 'phone' },
    keywords: ['font', 'size', 'text', 'code', 'wrap', 'line numbers', 'theme', 'ui'],
    rows: [
      { key: 'appearance.uiFontSize', label: 'UI font size', description: 'Text size across the app; icons and layout stay the same' },
      { key: 'appearance.codeFontSize', label: 'Code font size', description: 'Code blocks, the file editor and diffs' },
      { key: 'appearance.codeWrap', label: 'Wrap code lines', description: 'Wrap long code lines instead of scrolling sideways' },
      { key: 'appearance.codeLineNumbers', label: 'Code line numbers', description: 'Line numbers in the file editor' },
    ],
  },
  {
    id: 'input',
    title: 'Input',
    group: 'basics',
    icon: Keyboard,
    cap: 'terminal',
    phone: { group: 'phone', label: 'Input & keyboard' },
    keywords: ['keyboard', 'haptics', 'autocorrect', 'toolbar', 'keys', 'modifiers'],
    rows: [
      { key: 'input.haptics', label: 'Haptics' },
      { key: 'input.composerAutocorrect', label: 'Composer autocorrect' },
      { key: 'input.modifierIcons', label: 'Use icons for modifier keys', description: '^ ⇧ ↺ ⇥ ⌥ instead of Ctrl Shift Esc Tab Opt' },
      { key: 'input.toolbar', label: 'Toolbar', description: 'Which keys the phone toolbar shows' },
    ],
  },
  {
    id: 'shortcuts',
    title: 'Keyboard shortcuts',
    group: 'basics',
    icon: Keyboard,
    phone: false,
    keywords: ['keyboard', 'shortcuts', 'chords', 'rebind', 'hotkeys', 'keys', 'extensions'],
    rows: [
      { key: 'shortcuts.chords', label: 'Shell chords', description: 'Rebind any shell chord; extension chords that collide are rebound automatically' },
    ],
  },
  {
    id: 'terminal',
    title: 'Terminal',
    group: 'basics',
    icon: TerminalSquare,
    cap: 'terminal',
    phone: { group: 'phone' },
    keywords: ['terminal', 'font', 'cursor', 'scrollback', 'screen', 'session'],
    rows: [
      { key: 'terminal.fontSize', label: 'Font size' },
      { key: 'terminal.lineHeight', label: 'Line height' },
      { key: 'terminal.cursorStyle', label: 'Cursor style' },
      { key: 'terminal.cursorBlink', label: 'Cursor blink' },
      { key: 'terminal.scrollback', label: 'Scrollback' },
      { key: 'terminal.sizePolicy', label: 'Phone-size policy', description: 'Fit phone reflows the session; Keep PC size leaves it alone' },
      { key: 'terminal.keepScreenOn', label: 'Keep screen on' },
      { key: 'terminal.reopenLast', label: 'Reopen last session' },
    ],
  },
  {
    id: 'editor',
    title: 'Editor',
    group: 'basics',
    icon: MonitorSmartphone,
    cap: 'files',
    phone: false,
    keywords: ['editor', 'vim', 'keybindings'],
    rows: [{ key: 'editor.vim', label: 'Vim mode', description: 'vim keybindings in the file editor, with a NORMAL/INSERT/VISUAL indicator and :w to save' }],
  },
  {
    id: 'board',
    title: 'Board',
    group: 'basics',
    icon: FolderKanban,
    phone: { group: 'pc' },
    keywords: ['board', 'kanban', 'project', 'backlog'],
    rows: [
      { key: 'board.defaultProject', label: 'Default project' },
      { key: 'board.showBacklog', label: 'Always show backlog', description: 'Backlog also shows by itself whenever it has cards' },
    ],
  },
  {
    id: 'extensions',
    title: 'Extensions',
    group: 'agent',
    icon: Puzzle,
    cap: 'hosts',
    phone: { group: 'pc' },
    keywords: ['extensions', 'plugins', 'enable', 'disable', 'safe mode', 'crash', 'wasm'],
    rows: [
      { key: 'extensions.list', label: 'Installed extensions', description: 'Enable or disable extensions; crash and safe-mode notices' },
    ],
  },
  {
    id: 'agent',
    title: 'Agent',
    group: 'agent',
    icon: Waypoints,
    cap: 'hosts',
    phone: { group: 'pc' },
    keywords: ['oino', 'agent', 'settings', 'model', 'defaults', 'core'],
    rows: [
      { key: 'agent.settings', label: 'Agent settings', description: "The oino agent's settings.json (this PC)" },
    ],
  },
  {
    id: 'mcp',
    title: 'MCP',
    group: 'agent',
    icon: Plug,
    cap: 'hosts',
    phone: { group: 'pc' },
    keywords: ['mcp', 'model context protocol', 'servers', 'tools', 'catalog', 'json'],
    rows: [
      { key: 'mcp.servers', label: 'Servers', description: 'Every configured server; enable, edit its JSON or remove it' },
      { key: 'mcp.status', label: 'Server status', description: 'What each server offers and whether it connected' },
      { key: 'mcp.catalog', label: 'Catalog', description: 'Add a server from the catalog, sync it, reload every server' },
    ],
  },
  {
    id: 'memory',
    title: 'Memory',
    group: 'agent',
    icon: Brain,
    cap: 'hosts',
    phone: { group: 'pc' },
    keywords: ['memory', 'remember', 'recall', 'mempalace', 'browse', 'forget', 'consolidate', 'dream', 'reflection'],
    rows: [
      { key: 'memory.form', label: 'Recall and write', description: 'When the agent reads and writes memories, and the palace paths' },
      { key: 'memory.browse', label: 'Browse memories', description: 'List, search and forget what this project remembers' },
      { key: 'memory.consolidate', label: 'Consolidate this session', description: 'Let the agent write what this chat is worth remembering' },
      { key: 'dream.status', label: 'Dream', description: 'Status, the latest dream report, or dream now' },
    ],
  },
  {
    id: 'skills',
    title: 'Skills',
    group: 'agent',
    icon: Sparkles,
    cap: 'hosts',
    phone: { group: 'pc' },
    keywords: ['skills', 'exposure', 'judged', 'pins', 'vault', 'editor'],
    rows: [
      { key: 'skills.form', label: 'Exposure and pins', description: 'Judged / all / off, pinned skills and the vault' },
      { key: 'skills.hub', label: 'Skills hub', description: 'The extension’s own panel' },
      { key: 'skills.editor', label: 'Skill editor', description: 'Enabled / Must show, per skill' },
    ],
  },
  {
    id: 'subagents',
    title: 'Subagents',
    group: 'agent',
    icon: Bot,
    cap: 'hosts',
    phone: { group: 'pc' },
    keywords: ['subagents', 'agents', 'profiles', 'background', 'delegate', 'concurrency'],
    rows: [
      { key: 'subagents.form', label: 'Concurrency and defaults', description: 'How many run at once and which model they use' },
      { key: 'subagents.profiles', label: 'Agent profiles', description: 'List, create, edit, copy or delete custom agents' },
    ],
  },
  {
    id: 'providers',
    title: 'Providers',
    group: 'agent',
    icon: Plug,
    cap: 'hosts',
    phone: { group: 'pc' },
    keywords: ['providers', 'login', 'logout', 'sign in', 'oauth', 'api key', 'credentials', 'models'],
    rows: [
      { key: 'providers.list', label: 'Model providers', description: 'Sign in, save an API key or sign out' },
    ],
  },
  {
    id: 'hosts',
    title: 'Hosts',
    group: 'hosts',
    icon: Server,
    cap: 'hosts',
    phone: { group: 'pcs', label: 'PCs' },
    keywords: ['hosts', 'pair', 'phone', 'pc', 'this computer', 'unpair', 'iroh'],
    rows: [{ key: 'hosts.list', label: 'Paired hosts', description: 'This computer and paired phones/PCs' }],
  },
  {
    id: 'connection',
    title: 'Connection',
    group: 'hosts',
    icon: Unplug,
    cap: 'hosts',
    phone: { group: 'pcs' },
    keywords: ['connection', 'relay', 'n0', 'ssh', 'path'],
    rows: [
      { key: 'connection.relay', label: 'Relay', description: 'How this app reaches its hosts when not on the same network' },
      { key: 'connection.pathPreference', label: 'Path preference', description: 'Auto or prefer SSH' },
    ],
  },
  {
    id: 'sharing',
    title: 'Sharing',
    group: 'hosts',
    icon: Smartphone,
    cap: 'hosts',
    phone: { group: 'pcs' },
    keywords: ['sharing', 'features', 'files', 'ports', 'browser', 'screen'],
    rows: [{ key: 'sharing.features', label: 'Shared with phones', description: 'What paired devices may use on this PC' }],
  },
  {
    id: 'notifications',
    title: 'Notifications',
    group: 'hosts',
    icon: Bell,
    phone: { group: 'phone' },
    keywords: ['notifications', 'push', 'ntfy', 'needs you', 'desktop', 'phone', 'background', 'notify', 'gotify', 'telegram', 'events', 'silence', 'renotify'],
    rows: [
      { key: 'notify.desktop', label: 'Desktop notifications', description: 'A system notification when the agent needs you or finishes, while Unipi isn\u2019t in front' },
      { key: 'notify.enabled', label: 'Receive on this phone', description: 'Show the alerts your paired PCs push; each PC sends needs you and agent finished by default' },
      { key: 'notify.events', label: 'Which events notify', description: 'The PC’s event matrix, channels (ntfy, Gotify, Telegram) and quiet hours' },
      { key: 'notify.keepListening', label: 'Keep listening', description: 'Poll the topic on resume + while the app is open (fallback when no distributor is installed)' },
    ],
  },
  {
    id: 'unicrab',
    title: 'Unicrab',
    group: 'hosts',
    icon: KanbanSquare,
    show: () => can('hosts') || can('tray'),
    phone: { group: 'phone' },
    keywords: ['unicrab', 'crab', 'mascot', 'floating', 'overlay', 'window'],
    rows: [
      { key: 'unicrab.floating', label: 'Floating crab', description: 'A button you can drag to any edge; long-press for quick actions' },
      { key: 'unicrab.overlay', label: 'Over other apps' },
      { key: 'unicrab.window', label: 'Mascot window' },
    ],
  },
  {
    id: 'host',
    title: 'Host config',
    group: 'data',
    icon: Cpu,
    desktopOnly: true,
    phone: false,
    keywords: ['host', 'config', 'relay', 'forwarding', 'push', 'fs', 'scope', 'term', 'update', 'cron'],
    rows: [
      { key: 'host.relay', label: 'Host relay', description: 'The relay this PC\u2019s host advertises for pairing' },
      { key: 'host.relay_only', label: 'Relay only', description: 'Refuse direct connections' },
      { key: 'host.forwarding.enabled', label: 'Port forwarding' },
      { key: 'host.push.enabled', label: 'Push to phones' },
      { key: 'host.fs.allow', label: 'File access allow list' },
      { key: 'host.term.max', label: 'Max terminals' },
      { key: 'host.update.mode', label: 'Host updates', description: 'Prompt or install automatically' },
    ],
  },
  {
    id: 'ports',
    title: 'Ports & previews',
    group: 'hosts',
    icon: Globe,
    cap: 'hosts',
    phoneOnly: true,
    phone: { group: 'pc' },
    keywords: ['ports', 'preview', 'forward', 'web app', 'localhost', 'browser', 'tabs'],
    rows: [
      { key: 'ports.web', label: 'Web apps on your PC', description: 'Open or forward a port the PC is serving' },
      { key: 'ports.tabs', label: 'Browser tabs', description: 'Pages open in the in-app browser' },
    ],
  },
  {
    id: 'about',
    title: 'About',
    group: 'about',
    icon: Info,
    phone: { group: 'about' },
    keywords: ['about', 'version', 'update', 'licence', 'changelog', 'components', 'host', 'oino', 'engine', 'extensions', 'installed'],
    rows: [
      { key: 'about.update', label: 'Updates', description: 'One card: the app updater and the host\u2019s release' },
      { key: 'about.version', label: 'Version' },
      { key: 'about.components', label: 'Components', description: 'What this install ships: the app, the agent host, the oino engine and the extensions' },
      { key: 'about.licence', label: 'Licences' },
    ],
  },
];

export function sectionById(id: string | undefined): SettingsSection | undefined {
  return SETTINGS_SECTIONS.find((section) => section.id === id);
}
