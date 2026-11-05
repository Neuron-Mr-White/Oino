/**
 * The composer's plug (owner 2026-11-05): the GUI home of everything the terminal-era
 * slash commands used to reach (`/agents`, `/bg-tasks`, `/info`, `/compact-stats`,
 * `/fusion-stats`, `/doctor`, `/visualize-progress`, `/move`, `/memory`, `/notify-event`,
 * `/cron`, `/skills`, `/mcp-status`, `/readme`, `/changelog`, `/bug`…). Those commands no
 * longer clutter the `/` menu (they still run when typed); each row here opens the app
 * surface, or — where the answer is a report — runs the command in this chat.
 */
import { createSignal, Show, type Component } from 'solid-js';
import { Activity, Bot, BookOpen, Brain, Bug, CalendarClock, ClipboardCheck, FolderInput, Gauge, Info, Layers, Network, Plug, Bell, Sparkles, Stethoscope, Newspaper } from 'lucide-solid';
import { AnchoredMenu, MenuLabel, MenuRow, MenuSeparator } from '../board/AnchoredMenu';
import type { SessionHandle } from '../../lib/session/store';
import { openSessionModal } from '../session/modalTarget';
import { openStatusSection } from '../status/StatusPane';
import { openMoveWorkspaceDialog } from '../session/MoveWorkspaceDialog';
import { openMemoryBrowserFor } from '../memory/MemoryBrowser';
import { openSettingsAt } from '../../lib/settings/open';
import { ABOUT_ANCHORS } from '../../screens/settings/anchors';
import { openSkillsManager } from '../../screens/settings/SkillsPage';
import { appNavigate } from '../../lib/nav';
import { noteCommandRun } from '../../lib/reports';
import { chatHasCommand } from '../../lib/settings/manage';

export type PlugItem = { id: string; label: string; icon: Component<{ size?: number }>; group: 'chat' | 'reports' | 'app'; command?: string };

/** The rows (pure list; `run` decides what each does). */
export const PLUG_ITEMS: PlugItem[] = [
  { id: 'running', label: 'Running tasks & agents', icon: Activity, group: 'chat' },
  { id: 'info', label: 'Session info & cost', icon: Info, group: 'chat' },
  { id: 'progress', label: 'Progress graph', icon: Network, group: 'chat' },
  { id: 'move', label: 'Move to a workspace…', icon: FolderInput, group: 'chat' },
  { id: 'compact-stats', label: 'Compaction stats', icon: Layers, group: 'reports', command: 'compact-stats' },
  { id: 'compact-doctor', label: 'Compaction doctor', icon: ClipboardCheck, group: 'reports', command: 'compact-doctor' },
  { id: 'fusion-stats', label: 'Fusion savings', icon: Gauge, group: 'reports', command: 'fusion-stats' },
  { id: 'doctor', label: 'Health check', icon: Stethoscope, group: 'reports', command: 'doctor' },
  { id: 'readme', label: 'Readme', icon: BookOpen, group: 'reports', command: 'readme' },
  { id: 'memory', label: 'Memories', icon: Brain, group: 'app' },
  { id: 'skills', label: 'Skills', icon: Sparkles, group: 'app' },
  { id: 'agents', label: 'Agent profiles', icon: Bot, group: 'app' },
  { id: 'mcp', label: 'MCP servers', icon: Plug, group: 'app' },
  { id: 'cron', label: 'Cron', icon: CalendarClock, group: 'app' },
  { id: 'notify', label: 'Notification events', icon: Bell, group: 'app' },
  { id: 'changelog', label: "What's new", icon: Newspaper, group: 'app' },
  { id: 'bug', label: 'Report a bug', icon: Bug, group: 'app' },
];

export function PlugMenu(props: { session: SessionHandle; cwd?: string; disabled?: boolean; /** New-chat draft: only the app surfaces (no chat yet). */ draft?: boolean }) {
  const [anchor, setAnchor] = createSignal<DOMRect | undefined>();
  const chat = () => ({ hostId: props.session.hostId, sessionId: props.session.sessionId() });
  const run = (item: PlugItem) => {
    setAnchor(undefined);
    const c = chat();
    switch (item.id) {
      case 'running':
        return openSessionModal({ ...c, tab: 'running' });
      case 'info':
        return openSessionModal({ ...c, tab: 'info' });
      case 'progress':
        return openStatusSection({ hostId: c.hostId, sessionId: c.sessionId, path: props.cwd || '~' }, 'lh');
      case 'move':
        return openMoveWorkspaceDialog({ ...c, cwd: props.cwd });
      case 'memory':
        return openMemoryBrowserFor('project', c.hostId);
      case 'skills':
        return openSkillsManager({ hostId: c.hostId, cwd: props.cwd });
      case 'agents':
        return openSettingsAt('subagents', 'sec-subagents');
      case 'mcp':
        return openSettingsAt('mcp', 'sec-mcp');
      case 'cron':
        return appNavigate('/cron');
      case 'notify':
        return openSettingsAt('notifications');
      case 'changelog':
        return openSettingsAt('about', ABOUT_ANCHORS['whats-new']);
      case 'bug':
        return openSettingsAt('about', ABOUT_ANCHORS['report-bug']);
    }
    if (item.command) {
      noteCommandRun(props.session.key, item.command);
      props.session.send(`/${item.command}`, { mode: 'prompt' });
    }
  };
  const available = (item: PlugItem) => !item.command || props.session.state.commands.length === 0 || chatHasCommand(props.session, item.command);
  const group = (g: PlugItem['group']) => (props.draft && g !== 'app' ? [] : PLUG_ITEMS.filter((i) => i.group === g && available(i)));
  return (
    <>
      <button
        type="button"
        class="tb-chip tb-chip-plug"
        aria-label="Tools and panels"
        aria-haspopup="menu"
        aria-expanded={!!anchor()}
        title="Tools and panels"
        disabled={props.disabled}
        onClick={(e) => (anchor() ? setAnchor(undefined) : setAnchor((e.currentTarget as HTMLElement).getBoundingClientRect()))}
        data-testid="composer-plug"
      >
        <Plug size={15} class="tb-ic" />
      </button>
      <Show when={anchor()}>
        {(rect) => (
          <AnchoredMenu anchor={rect()} width={252} label="Tools and panels" title="Tools and panels" onDismiss={() => setAnchor(undefined)}>
            <Show when={group('chat').length}>
              <MenuLabel>This chat</MenuLabel>
              {group('chat').map((i) => <MenuRow icon={<i.icon size={15} />} label={i.label} testId={`plug-${i.id}`} onSelect={() => run(i)} />)}
            </Show>
            <Show when={group('reports').length}>
              <MenuSeparator />
              <MenuLabel>Reports</MenuLabel>
              {group('reports').map((i) => <MenuRow icon={<i.icon size={15} />} label={i.label} testId={`plug-${i.id}`} onSelect={() => run(i)} />)}
            </Show>
            <Show when={!props.draft}>
              <MenuSeparator />
            </Show>
            <MenuLabel>App</MenuLabel>
            {group('app').map((i) => <MenuRow icon={<i.icon size={15} />} label={i.label} testId={`plug-${i.id}`} onSelect={() => run(i)} />)}
          </AnchoredMenu>
        )}
      </Show>
    </>
  );
}
