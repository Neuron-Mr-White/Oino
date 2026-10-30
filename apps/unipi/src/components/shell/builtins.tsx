/**
 * The built-in screens' palette commands (UNI-117). Kept here, outside the
 * screen files, because their state lives in global singletons (board ui
 * store, chat layer). The shell's own commands (Go to, View) are in
 * AppShell (WP-A3).
 */
import { onCleanup } from 'solid-js';
import { useLocation } from '@solidjs/router';
import { MessagesSquare, Plus, Search, SlidersHorizontal, LayoutGrid, List, Monitor, FolderOpen, RefreshCw, X, Server, LayoutDashboard, Keyboard, Network, QrCode } from 'lucide-solid';
import { chatVisible, hideChat } from '../../lib/session/open';
import { boardUi } from '../../lib/board/ui';
import { registerCommands } from '../../lib/shell';
import { can, isTauri } from '../../lib/platform';
import { setAddHostOpen } from '../Pairing';
import { openPairPhone } from '../../lib/pairPhone';
import { currentWorkspace } from './Workspace';
import { openStatusSection } from '../status/StatusPane';

export function BuiltinPanes() {
  const location = useLocation();
  const ui = boardUi();
  const onBoard = () => location.pathname === '/board';
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
      { id: 'chat.progress', title: 'Show long-horizon progress', group: 'Chat', keywords: 'goal ralph swarm graph progress status pane', icon: () => <Network size={16} />, when: () => chatVisible(), run: () => {
        // §3.10: the Progress sheet is gone — the LH section of the status pane owns it.
        const ws = currentWorkspace();
        if (!ws) return;
        openStatusSection({ hostId: ws.scope.hostId, sessionId: ws.view.sessionId, path: ws.scope.projectKey }, 'lh');
      } },
      { id: 'chat.close', title: 'Back to the chat list', group: 'Chat', keywords: 'close leave', icon: () => <X size={16} />, keys: 'Esc', when: () => chatVisible(), run: () => hideChat() },
      { id: 'hosts.add', title: 'Add a PC (pair)', group: 'PCs', keywords: 'pair computer host add a host', icon: () => <Server size={16} />, when: () => can('hosts'), run: () => setAddHostOpen(true) },
      { id: 'hosts.pairPhone', title: 'Pair a phone', group: 'PCs', keywords: 'qr code phone pair this pc computer scan', icon: () => <QrCode size={16} />, when: () => can('localSharing'), run: () => openPairPhone() },
      { id: 'app.reload', title: 'Reload the app', group: 'App', keywords: 'refresh', icon: () => <RefreshCw size={16} />, run: () => window.location.reload() },
    ]),
  );
  return null;
}
