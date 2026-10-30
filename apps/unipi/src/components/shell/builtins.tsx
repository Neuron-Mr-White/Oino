/**
 * The built-in screens' pane specs and palette commands (UNI-117). Kept
 * here, outside the screen files, because the screens' detail views are
 * global singletons (chat layer, board ui store, editor route): the shell
 * can read their open state without touching the screens. A new screen
 * declares its own in its component instead (see lib/shell.ts).
 */
import { onCleanup } from 'solid-js';
import { useLocation, useNavigate } from '@solidjs/router';
import { MessagesSquare, Plus, Search, SlidersHorizontal, LayoutGrid, List, Monitor, FolderOpen, RefreshCw, X, Server, LayoutDashboard, Keyboard, Network, QrCode } from 'lucide-solid';
import { setProgressSheetOpen } from '../status/lhProgress';
import { chatVisible, hideChat } from '../../lib/session/open';
import { setNewChatOpen } from '../../lib/session/newSession';
import { boardUi } from '../../lib/board/ui';
import { registerCommands, registerPane, screenOf } from '../../lib/shell';
import { can, isTauri } from '../../lib/platform';
import { setAddHostOpen } from '../Pairing';
import { openPairPhone } from '../../lib/pairPhone';
import { PanePlaceholder } from './DesktopShell';

export function BuiltinPanes() {
  const location = useLocation();
  const on = (screen: string) => () => screenOf(location.pathname) === screen;
  const ui = boardUi();
  const onBoard = on('board');
  const navigate = useNavigate();
  onCleanup(
    registerPane({
      screen: 'chat',
      open: () => chatVisible(),
      placeholder: !can('chat') ? undefined : () => <PanePlaceholder icon={<MessagesSquare size={22} />} title="No chat open" hint="Pick a session on the left. It stays live while you browse the others." />,
    }),
  );
  onCleanup(registerPane({ screen: 'board', open: () => !!ui.openedTask() || !!ui.reader() }));
  onCleanup(registerPane({ screen: 'files', open: () => location.pathname === '/files/edit' }));

  const boardReady = () => onBoard() && can('board');
  onCleanup(
    registerCommands([
      { id: 'board.new', title: 'New task', group: 'Board', keywords: 'add create', icon: () => <Plus size={16} />, when: boardReady, run: () => ui.openNewTask({ lane: 'backlog' }) },
      { id: 'board.search', title: 'Search tasks', group: 'Board', keywords: 'find', icon: () => <Search size={16} />, when: boardReady, run: () => ui.setSheet('search') },
      { id: 'board.filter', title: 'Filter tasks', group: 'Board', keywords: 'priority label', icon: () => <SlidersHorizontal size={16} />, when: boardReady, run: () => ui.setSheet('filter') },
      { id: 'board.display', title: 'Display options', group: 'Board', keywords: 'lanes columns backlog', icon: () => <LayoutGrid size={16} />, when: boardReady, run: () => ui.setSheet('display') },
      { id: 'board.view.board', title: 'Show as lanes', group: 'Board', keywords: 'kanban columns view', icon: () => <LayoutGrid size={16} />, when: () => boardReady() && ui.view() !== 'board', run: () => ui.setView('board') },
      { id: 'board.view.list', title: 'Show as list', group: 'Board', keywords: 'view', icon: () => <List size={16} />, when: () => boardReady() && ui.view() !== 'list', run: () => ui.setView('list') },
      { id: 'board.overview', title: 'Overview of all projects', group: 'Board', keywords: 'dashboard inbox needs you agents', icon: () => <LayoutDashboard size={16} />, when: () => boardReady() && !ui.overview(), run: () => ui.setOverview(true) },
      { id: 'board.back', title: 'Back to the board', group: 'Board', keywords: 'lanes project', icon: () => <LayoutGrid size={16} />, when: () => boardReady() && ui.overview(), run: () => ui.setOverview(false) },
      { id: 'board.shortcuts', title: 'Keyboard shortcuts', group: 'Board', keywords: 'keys help', keys: '?', icon: () => <Keyboard size={16} />, when: boardReady, run: () => ui.setShortcutsOpen(true) },
      { id: 'board.pc', title: 'Switch PC', group: 'Board', keywords: 'host computer', icon: () => <Monitor size={16} />, when: () => boardReady() && isTauri(), run: () => ui.setSheet('pc') },
      { id: 'board.project', title: 'Switch project', group: 'Board', keywords: 'project', icon: () => <FolderOpen size={16} />, when: boardReady, run: () => ui.setSheet('project') },
      { id: 'board.closeTask', title: 'Close task', group: 'Board', icon: () => <X size={16} />, keys: 'Esc', when: () => onBoard() && !!ui.openedTask(), run: () => ui.closeTask() },
      { id: 'chat.progress', title: 'Show long-horizon progress', group: 'Chat', keywords: 'goal ralph swarm graph progress mermaid', icon: () => <Network size={16} />, when: () => chatVisible(), run: () => setProgressSheetOpen(true) },
      { id: 'chat.close', title: 'Back to the chat list', group: 'Chat', keywords: 'close leave', icon: () => <X size={16} />, keys: 'Esc', when: () => chatVisible(), run: () => hideChat() },
      { id: 'chat.new', title: 'New chat', group: 'Chat', keywords: 'start session folder project', icon: () => <Plus size={16} />, when: () => can('chat'), run: () => (navigate('/chat'), setNewChatOpen(true)) },
      { id: 'hosts.add', title: 'Add a PC (pair)', group: 'PCs', keywords: 'pair computer host add a host', icon: () => <Server size={16} />, when: () => can('hosts'), run: () => setAddHostOpen(true) },
      { id: 'hosts.pairPhone', title: 'Pair a phone', group: 'PCs', keywords: 'qr code phone pair this pc computer scan', icon: () => <QrCode size={16} />, when: () => can('localSharing'), run: () => openPairPhone() },
      { id: 'app.reload', title: 'Reload the app', group: 'App', keywords: 'refresh', icon: () => <RefreshCw size={16} />, run: () => window.location.reload() },
    ]),
  );
  return null;
}
