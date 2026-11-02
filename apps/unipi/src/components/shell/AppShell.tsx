/**
 * App shell (app spec §3.7, WP-A3): the router root.
 *
 * Desktop (`html[data-shell=desktop]`): sidebar (264 px, Cmd/Ctrl+B) with
 * the rail (Z-1) | main area. The main area is a rail page (Board, Cron,
 * extension pages, Settings) or the workspace — conversation frame + side
 * pane + bottom panel (Workspace.tsx). Shortcuts: Cmd/Ctrl+B sidebar,
 * Cmd/Ctrl+J bottom panel, Cmd/Ctrl+Alt+B side pane, Cmd/Ctrl+Shift+F full
 * view, Cmd/Ctrl+Shift+T reopen closed tab, Cmd/Ctrl+K palette, Cmd/Ctrl+/
 * shortcuts. The old screens model (NAV, Ctrl+1…6, list | detail panes) is
 * gone.
 *
 * Phone: the unipi shell as before (Z-115): bottom tab bar, full-screen
 * layers; the conversation route is a full-screen layer. WP-A29 moved the
 * frame to `PhoneShell.tsx` (tab bar `TabBar.tsx`, ⋯ "Panes" group
 * `PanesMenu.tsx`).
 */
import { createEffect, onCleanup, onMount, Show, type ParentProps } from 'solid-js';
import { useLocation, useNavigate } from '@solidjs/router';
import { Columns3, Keyboard, PanelBottom, PanelLeft, PanelRight, Maximize2, Settings as SettingsIcon, SquarePen, Clock, History } from 'lucide-solid';
import { RelaySettings } from '../RelaySettings';
import { HostsProvider, useHostsOptional } from '../../hosts/context';
import { SettingsProvider, useSettings } from '../../settings/context';
import { Pairing } from '../Pairing';
import { FloatingBell } from '../FloatingBell';
import { NeedsYouFeed } from '../NeedsYouBell';
import { ReportSheet } from '../ReportSheet';
import { Toasts } from '../Toasts';
import { TooltipHost } from '../Tooltip';
import { NotifyInitRunner } from '../NotifyInitRunner';
import { IncomingPushRunner } from '../IncomingPush';
import { SharingRequestsRunner } from '../SharingSettings';
import { CloseAskRunner, QuitRunner } from '../LifecycleUi';
import { PairPhoneDialog } from '../PairPhone';
import { DesktopUpdatePrompt } from '../UpdateCard';
import { PhoneShell } from './PhoneShell';
import { LinkAnchorMenu } from '../LinkAnchorMenu';
import { MermaidViewer } from '../chat/MermaidViewer';
import { StatusPaneLayer } from '../status/StatusPane';
import { Previewer } from '../media/Previewer';
import Editor from '../../screens/Editor';
import { InstallCheck } from '../../lib/about';
import { inTextField } from '../../lib/desktop';
import { bindingForEvent, installShortcutsPersistence, scopeOf, type BindingHit } from '../../lib/shortcuts/registry';
import { hasForegroundSubagent, SUBAGENTS_SHORTCUT } from '../../lib/shortcuts/subagents';
import { cycleSessionModel } from '../model/cycleModel';
import { openFind } from '../../lib/find';
import { focusComposer } from '../../lib/timeline/actions';
import { openSessionModal } from '../session/SessionModal';
import { hubs } from '../../lib/session/hub';
import { sessions } from '../../lib/session/store';
import { sessionGroups } from '../../lib/session/selectors';
import { getEditorTarget } from '../../lib/editorTarget';
import { gitParams, gitTarget, setGitDesktopOpener } from '../../lib/git';
import { chatVisible } from '../../lib/session/open';
import { boardUi } from '../../lib/board/ui';
import { can } from '../../lib/platform';
import { layout } from '../../lib/layout';
import { panes } from '../../lib/panes';
import { modKey, paletteOpen, registerCommands, setPaletteOpen } from '../../lib/shell';
import { CommandCenter } from '../command/CommandCenter';
import { BuiltinPanes } from './builtins';
import { DesktopNative } from './DesktopNative';
import { SessionModalLayer } from '../session/SessionModal';
import { SessionActions } from '../session/SessionActions';
import { ModelPickerLayer } from '../model/ChatModelPicker';
import { openModelPicker } from '../model/target';
import { cycleThinkingLevel } from '../../lib/model/thinking';
import { composerDraft, setComposerText } from '../../lib/composer/drafts';
import { cycleStarter } from '../../lib/status/modeStarter';
import { setShortcutsOpen, ShortcutsDialog, shortcutsOpen } from './ShortcutsDialog';
import { BuiltinActions } from '../command/BuiltinActions';
import { ResizeHandle } from './ResizeHandle';
import { Sidebar, useNewChatHost } from './Sidebar';
import { currentWorkspace } from './Workspace';
import { SIDEBAR_DEFAULT } from '../../lib/layout';
import { installExtensionPageFeed } from '../../ui/slots/pages';
import { ReservedRoutingLayer } from '../../ui/slots/ReservedHost';
import { reservedGate } from '../../ui/slots/reserved';

// Z-100: every Git entry point opens the one `git` side-pane tab; the phone
// keeps the full-screen layer (§3.9). Installed here so `lib/git` stays free
// of component imports.
setGitDesktopOpener((target) => {
  const ws = currentWorkspace();
  if (!ws) return false;
  panes().open(ws.scope, 'side', ws.view, { kind: 'git', params: gitParams(target) });
  layout().toggleSide(true);
  return true;
});

/** Keyed editor: a new target (another file picked in the Files list) remounts it. */
const NO_EDITOR_TARGET = {};

/** The conversation route (`/s/:host/:session`), if one is shown. */
export function currentSessionRoute(): { hostId: string; sessionId: string } | undefined {
  const m = /^#\/s\/([^/]+)\/([^/?#]+)/.exec(location.hash);
  return m ? { hostId: decodeURIComponent(m[1]!), sessionId: decodeURIComponent(m[2]!) } : undefined;
}

/** The sidebar's conversation order across hosts (Z-8/Z-9): This computer
 *  first, then the paired PCs; per host pinned → projects (running first). */
export function orderedConversations(hosts: ReadonlyArray<{ id: string; transport?: string }>): Array<{ hostId: string; sessionId: string }> {
  const list = [...hosts].sort((a, b) => Number(b.transport === 'local') - Number(a.transport === 'local'));
  const out: Array<{ hostId: string; sessionId: string }> = [];
  for (const host of list) {
    const hub = hubs().peek(host.id);
    if (!hub) continue;
    const groups = sessionGroups(hub.state.cards, hub.state.meta);
    for (const card of [...groups.pinned, ...groups.projects.flatMap((g) => g.cards)]) {
      out.push({ hostId: host.id, sessionId: card.session_id });
    }
  }
  return out;
}

type BindingCtx = { navigate: (to: string) => void; newChatHost: () => string; hosts: ReadonlyArray<{ id: string; transport?: string }> };

/** Runs a resolved binding (WP-A18: the shortcut registry drives the shell). */
export function runBinding(hit: BindingHit, ctx: BindingCtx): boolean {
  const cur = currentSessionRoute();
  if (hit.kind === 'ext') {
    // Ruled (§4): Shift+↓ never reaches `unipi.tray.open` — the app opens Session · Running.
    if (hit.intercepted) return runBinding({ kind: 'app', id: hit.interceptedBy ?? 'chat.running' }, ctx);
    if (!cur) return false;
    const handle = sessions().peek(cur.hostId, cur.sessionId);
    if (!handle) return false;
    void handle.cmd({ type: '_oino/invoke_shortcut', shortcutId: hit.id }).catch((error: unknown) => console.warn('[shortcut] invoke failed', hit.id, error));
    return true;
  }
  if (scopeOf(hit.id) === 'conversation' && !cur) return false;
  const L = layout();
  switch (hit.id) {
    case 'shell.palette':
      setPaletteOpen(!paletteOpen());
      return true;
    case 'shell.files':
      setPaletteOpen(true, 'files');
      return true;
    case 'shell.shortcuts':
      if (!L.desktop()) return false;
      setPaletteOpen(false);
      setShortcutsOpen(!shortcutsOpen());
      return true;
    case 'shell.settings':
      ctx.navigate('/settings');
      return true;
    case 'chat.new':
      ctx.navigate(`/new/${encodeURIComponent(ctx.newChatHost())}`);
      return true;
    case 'find.open':
      openFind();
      return true;
    case 'chat.focus':
      focusComposer(`${cur!.hostId}/${cur!.sessionId}`);
      return true;
    case 'chat.running':
      openSessionModal({ hostId: cur!.hostId, sessionId: cur!.sessionId, tab: 'running' });
      return true;
    // WP-A14 (Z-69..Z-74): the model sheet, the thinking cycle and the
    // working-mode starter cycle all act on the shown session.
    case 'model.open':
      openModelPicker({ hostId: cur!.hostId, sessionId: cur!.sessionId });
      return true;
    case 'model.cycle': {
      const handle = sessions().peek(cur!.hostId, cur!.sessionId);
      if (!handle) return false;
      void cycleSessionModel(handle);
      return true;
    }
    case 'thinking.cycle': {
      const handle = sessions().peek(cur!.hostId, cur!.sessionId);
      if (!handle) return false;
      void cycleThinkingLevel(handle).catch(() => undefined);
      return true;
    }
    case 'mode.cycle': {
      const handle = sessions().peek(cur!.hostId, cur!.sessionId);
      if (!handle) return false;
      setComposerText(handle.key, cycleStarter(composerDraft(handle.key).text).text);
      focusComposer(handle.key);
      return true;
    }
    case 'chat.prev':
    case 'chat.next': {
      const rows = orderedConversations(ctx.hosts);
      const at = rows.findIndex((r) => r.hostId === cur?.hostId && r.sessionId === cur?.sessionId);
      if (at < 0 || rows.length < 2) return false;
      const next = rows[(at + (hit.id === 'chat.next' ? 1 : -1) + rows.length) % rows.length]!;
      ctx.navigate(`/s/${encodeURIComponent(next.hostId)}/${encodeURIComponent(next.sessionId)}`);
      return true;
    }
    case 'shell.sidebar': {
      // P0-10: a foreground subagent in the shown session goes to the background first.
      const handle = cur ? sessions().peek(cur.hostId, cur.sessionId) : undefined;
      if (handle && hasForegroundSubagent(handle.state)) {
        void handle.cmd({ type: '_oino/invoke_shortcut', shortcutId: SUBAGENTS_SHORTCUT }).catch((error: unknown) => console.warn('[shortcut] invoke failed', SUBAGENTS_SHORTCUT, error));
        return true;
      }
      L.toggleSidebar();
      return true;
    }
    case 'shell.bottom':
      L.toggleBottom();
      return true;
    case 'shell.side':
      L.toggleSide();
      return true;
    case 'shell.fullview':
      L.toggleFullView();
      return true;
    case 'shell.reopen': {
      const ws = currentWorkspace();
      if (!ws) return false;
      if (panes().reopen(ws.scope, 'side', ws.view)) L.toggleSide(true);
      return true;
    }
    default:
      return false;
  }
}

function DesktopFrame(props: ParentProps) {
  const L = layout();
  const navigate = useNavigate();
  const newChatHost = useNewChatHost();
  const hostsCtx = useHostsOptional();
  onCleanup(
    registerCommands([
      { id: 'chat.new', title: 'New chat', group: 'Go to', keywords: 'start session folder project', icon: () => <SquarePen size={16} />, run: () => navigate(`/new/${encodeURIComponent(newChatHost())}`) },
      { id: 'go.board', title: 'Go to Board', group: 'Go to', keywords: 'kanban tasks', icon: () => <Columns3 size={16} />, when: () => reservedGate('unipi.kanboard/board'), run: () => navigate('/board') },
      { id: 'go.cron', title: 'Go to Cron', group: 'Go to', keywords: 'schedule jobs', icon: () => <Clock size={16} />, run: () => navigate('/cron') },
      { id: 'go.settings', title: 'Go to Settings', group: 'Go to', keywords: 'preferences', icon: () => <SettingsIcon size={16} />, keys: `${modKey()} ,`, run: () => navigate('/settings') },
      { id: 'view.sidebar', title: 'Toggle sidebar', group: 'View', keys: `${modKey()} B`, icon: () => <PanelLeft size={16} />, run: () => L.toggleSidebar() },
      { id: 'view.side', title: 'Toggle side pane', group: 'View', keywords: 'files git panel right', keys: `${modKey()} Alt B`, icon: () => <PanelRight size={16} />, run: () => runBinding({ kind: 'app', id: 'shell.side' }, { navigate, newChatHost, hosts: hostsCtx?.hosts() ?? [] }) },
      { id: 'view.bottom', title: 'Toggle terminal panel', group: 'View', keywords: 'bottom panel', keys: `${modKey()} J`, icon: () => <PanelBottom size={16} />, run: () => runBinding({ kind: 'app', id: 'shell.bottom' }, { navigate, newChatHost, hosts: hostsCtx?.hosts() ?? [] }) },
      { id: 'view.fullview', title: 'Toggle full view', group: 'View', keywords: 'side pane maximize', keys: `${modKey()} ⇧ F`, icon: () => <Maximize2 size={16} />, run: () => runBinding({ kind: 'app', id: 'shell.fullview' }, { navigate, newChatHost, hosts: hostsCtx?.hosts() ?? [] }) },
      { id: 'view.reopen', title: 'Reopen closed tab', group: 'View', keys: `${modKey()} ⇧ T`, icon: () => <History size={16} />, when: () => !!currentWorkspace(), run: () => runBinding({ kind: 'app', id: 'shell.reopen' }, { navigate, newChatHost, hosts: hostsCtx?.hosts() ?? [] }) },
      { id: 'app.shortcuts', title: 'Keyboard shortcuts', group: 'App', keywords: 'keys help hotkeys', keys: `${modKey()} /`, icon: () => <Keyboard size={16} />, run: () => runBinding({ kind: 'app', id: 'shell.shortcuts' }, { navigate, newChatHost, hosts: hostsCtx?.hosts() ?? [] }) },
    ]),
  );
  return (
    <div class="ashell" data-sidebar={L.sidebarOpen() ? 'open' : 'closed'} style={{ '--sb-w': `${L.sidebarWidth()}px` }} data-testid="app-shell">
      <Show when={L.sidebarOpen()}>
        <Sidebar />
        <ResizeHandle
          axis="x"
          dir={1}
          label="Resize sidebar"
          class="sb-handle"
          size={L.sidebarWidth()}
          min={220}
          max={420}
          onSize={(w) => L.setSidebarWidth(w)}
          onReset={() => L.setSidebarWidth(SIDEBAR_DEFAULT)}
          testId="sidebar-handle"
        />
      </Show>
      <div class="ashell-main">{props.children}</div>
    </div>
  );
}

/** html[data-shell] for the CSS (desktop rules are all scoped under it). */
function ShellAttr() {
  createEffect(() => {
    const L = layout();
    const root = document.documentElement;
    root.dataset.shell = L.desktop() ? 'desktop' : 'phone';
    // Fixed layers (board task page, git view, toasts) clear the sidebar.
    root.style.setProperty('--sidebar-w', L.desktop() && L.sidebarOpen() ? `${L.sidebarWidth()}px` : '0px');
  });
  onCleanup(() => {
    delete document.documentElement.dataset.shell;
    document.documentElement.style.removeProperty('--sidebar-w');
  });
  return null;
}

/** Z-94: the shortcut registry persists user chords in app settings (inside the provider). */
function ShortcutPersistence() {
  const [settings, update] = useSettings();
  createEffect(() => {
    const chords = settings.shortcuts.chords;
    installShortcutsPersistence({ read: () => chords, write: (next) => update('shortcuts.chords', next) });
  });
  return null;
}

/** The router root: providers, the frame of this shell, and the app-wide layers. */
/** The registry's window listener (WP-A18): both shells, so a phone with a
 *  hardware keyboard gets the same Cmd/Ctrl chords (Z-94 is an app map). */
function ShellShortcuts() {
  const navigate = useNavigate();
  const newChatHost = useNewChatHost();
  const hostsCtx = useHostsOptional();
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      const hit = bindingForEvent(e);
      if (!hit) return;
      // Inside a terminal plain Ctrl+K / Ctrl+J / Ctrl+B belong to the shell program.
      if (!(e.altKey || e.shiftKey) && (e.target as HTMLElement | null)?.closest?.('.term-layer, .xterm')) return;
      // Chords without the primary modifier never fire inside a text field
      // (Shift+↓ must keep extending a selection in the composer).
      if (!(e.ctrlKey || e.metaKey) && inTextField(e.target)) return;
      if (e.isComposing) return;
      e.preventDefault();
      runBinding(hit, { navigate, newChatHost, hosts: hostsCtx?.hosts() ?? [] });
    };
    window.addEventListener('keydown', onKey);
    onCleanup(() => window.removeEventListener('keydown', onKey));
  });
  return null;
}

export function AppShell(props: ParentProps) {
  const location = useLocation();
  const navigate = useNavigate();
  // U10: extension `fullscreen` pages are registry entries of their session,
  // not of the conversation view — the rail feed sits in the shell so a page
  // keeps its nav item while the user is on Settings or a board (§6.2/§6.6).
  installExtensionPageFeed(() => location.pathname);
  // U12: reserved ids route to native components — owner gates (live), the page a
  // focused `openPanel` asks for, and closing a page whose owner went away (§8).
  const desktop = () => layout().desktop();
  const onEditor = () => location.pathname === '/files/edit';
  return (
    <SettingsProvider>
      <HostsProvider>
        <ShellShortcuts />
        <ReservedRoutingLayer />
        <ShortcutPersistence />
        <RelaySettings />
        <ShellAttr />
        <Show when={desktop()} fallback={<PhoneShell>{props.children}</PhoneShell>}>
          <DesktopFrame>{props.children}</DesktopFrame>
        </Show>
        <BuiltinPanes />
        <Show when={can('hosts') && (desktop() || (!onEditor() && !chatVisible() && !boardUi().openedTask() && !gitTarget() && !location.pathname.startsWith('/s/')))}>
          <FloatingBell />
        </Show>
        <Show when={!desktop()}>
          <Show when={onEditor() ? (getEditorTarget() ?? NO_EDITOR_TARGET) : undefined} keyed>
            {(_target) => <Editor />}
          </Show>
        </Show>
        <Pairing />
        <NeedsYouFeed />
        <NotifyInitRunner />
        <IncomingPushRunner />
        <SharingRequestsRunner />
        <QuitRunner />
        <CloseAskRunner />
        <InstallCheck />
        <PairPhoneDialog />
        <Show when={can('desktopUpdater')}>
          <DesktopUpdatePrompt />
        </Show>
        <MermaidViewer />
        <StatusPaneLayer />
        <Previewer />
        <LinkAnchorMenu />
        <CommandCenter />
        <Toasts />
        <ReportSheet />
        {/* Z-116: the one tooltip bubble for every icon-only control. */}
        <TooltipHost />
        <SessionModalLayer />
        <ModelPickerLayer />
        {/* WP-A25: the ⋯ Export/Duplicate rows, the capability palettes and the
            Import session… dialog (the harness reachability surfaces). */}
        <BuiltinActions />
        {/* WP-A5: the ⋯ Rename/Fork rows, the row context menu and the Rename
            dialog (Z-13/15/16/20). */}
        <SessionActions />
        {/* The shortcuts table is the desktop keyboard reference (its sheet does
            not paint in the phone capture shell; the phone has no chord UI). */}
        <Show when={desktop()}>
          <ShortcutsDialog />
        </Show>
        <DesktopNative />
      </HostsProvider>
    </SettingsProvider>
  );
}
