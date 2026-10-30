import { PageHeader } from './components/PageHeader';
import { RelaySettings } from './components/RelaySettings';
import { HostsProvider } from './hosts/context';
import { RecentsProvider } from './lib/recentsContext';
import { Pairing } from './components/Pairing';
import { render } from 'solid-js/web';
import { HashRouter, Route, A, useLocation, useNavigate } from '@solidjs/router';
import { Terminal, Columns3, Settings as SettingsIcon, MessagesSquare, Files as FilesIcon } from 'lucide-solid';
import { Show } from 'solid-js';
import type { Component, ParentProps } from 'solid-js';
import Sessions from './screens/Sessions';
import TerminalLayer from './screens/Terminal';
import Board from './screens/Board';
import Chat from './screens/Chat';
import Cron from './screens/Cron';
import Files from './screens/Files';
import Editor from './screens/Editor';
import { getEditorTarget } from './lib/editorTarget';
import Preview from './screens/Preview';
import { ChatLayer } from './components/chat/ChatView';
import { MermaidViewer } from './components/chat/MermaidViewer';
import { ReviewViewer } from './components/chat/ReviewViewer';
import { GitView } from './components/git/GitView';
import { gitTarget } from './lib/git';
import { Previewer } from './components/media/Previewer';
import { LinkAnchorMenu } from './components/LinkAnchorMenu';
import './components/media/media.css';
import Settings from './screens/Settings';
import { SetupWizard } from './screens/SetupWizard';
import { SetupGate } from './lib/setupGate';
import { SettingsProvider } from './settings/context';
import '@xterm/xterm/css/xterm.css';
import './style.css';
import { SettingsNotices } from './components/SettingsNotices';
import { dismissSplashWhenReady } from './lib/splash';
import { watchForUpdates } from './lib/update';
import { watchDesktopUpdates } from './lib/desktopUpdate';
import { DesktopUpdatePrompt } from './components/DesktopUpdate';
import { UpdateBanner } from './components/UpdateCard';
import { FloatingBell } from './components/FloatingBell';
import { NotifyInitRunner } from './components/NotifyInitRunner';
import { IncomingPushRunner } from './components/IncomingPush';
import { SharingRequestsRunner } from './components/SharingSettings';
import { QuitRunner } from './components/LifecycleUi';
import { PairPhoneDialog } from './components/PairPhone';
import { chatVisible } from './lib/chat/open';
// The task pane has its own header; the floating bell would cover its ⋯ (UNI-181).
import { boardUi } from './lib/board/ui';
import { layout } from './lib/layout';
import { can, isWeb, type Capability } from './lib/platform';
import { applyWebEntry } from './lib/webEntry';
import { Requires, UnavailableNotice } from './components/Requires';
import { DesktopChrome, Sidebar } from './components/shell/DesktopShell';
import { CommandPalette } from './components/shell/CommandPalette';
import { BuiltinPanes } from './components/shell/builtins';
import { DesktopNative } from './components/shell/DesktopNative';
import { ShortcutsDialog } from './components/shell/ShortcutsDialog';
import './components/shell/shell.css';
// Shared tokens + components (UNI-218): last, so it wins over the per-screen rules it unifies.
import './ui.css';
/** Keyed editor: a new target (another file picked in the desktop Files pane) remounts it. */
const NO_EDITOR_TARGET = {};
function Shell(props: ParentProps) {
  const location = useLocation();
  const title = () => location.pathname === '/settings' ? 'Settings' : location.pathname === '/board' ? 'Board' : location.pathname === '/chat' ? 'Chat' : location.pathname === '/files' ? 'Files' : 'Sessions';
  // /term and /files/edit show Home underneath their full-screen layer
  // (the knob drag-down reveals the terminal); the page below is inert
  // while either covers it. On the desktop shell (UNI-117) they sit in the
  // detail pane instead, so the list stays usable.
  const onTerm = () => location.pathname === '/term';
  const onEditor = () => location.pathname === '/files/edit';
  const navigate = useNavigate();
  const desktop = () => layout().desktop();
  return (
    <SettingsProvider><HostsProvider><RecentsProvider>
      <RelaySettings/>
      <div class="app-shell" inert={!desktop() && (onTerm() || onEditor()) ? true : undefined}>
        <Show when={desktop()}><Sidebar /></Show>
        <Show when={location.pathname !== "/" && location.pathname !== "/board" && location.pathname !== "/chat" && location.pathname !== "/files" && location.pathname !== "/preview" && location.pathname !== "/cron" && !onTerm() && !onEditor()}><PageHeader title={title()}/></Show>
        <main class="app-content"><Show when={location.pathname !== "/settings"}><UpdateBanner onOpen={() => navigate("/settings")} /></Show><SettingsNotices />{props.children}</main>
        <Show when={!desktop()}>
          <nav class="bottom-nav" aria-label="Main navigation"><A href="/" end><Terminal size={22} strokeWidth={1.5} /><span>Sessions</span></A><A href="/chat"><MessagesSquare size={22} strokeWidth={1.5} /><span>Chat</span></A><A href="/board"><Columns3 size={22} strokeWidth={1.5} /><span>Board</span></A><A href="/files"><FilesIcon size={22} strokeWidth={1.5} /><span>Files</span></A><A href="/settings"><SettingsIcon size={22} strokeWidth={1.5} /><span>Settings</span></A></nav>
        </Show>
      </div>
      <DesktopChrome />
      <BuiltinPanes />
      <Show when={can('hosts') && (desktop() || (!onTerm() && !onEditor() && !chatVisible() && !boardUi().openedTask() && !gitTarget()))}><FloatingBell /></Show>
      <ChatLayer /><TerminalLayer /><Show when={onEditor() ? (getEditorTarget() ?? NO_EDITOR_TARGET) : undefined} keyed>{(_target) => <Editor />}</Show><Pairing /><NotifyInitRunner /><IncomingPushRunner /><SharingRequestsRunner /><QuitRunner /><SetupGate /><PairPhoneDialog /><Show when={can('desktopUpdater')}><DesktopUpdatePrompt /></Show><MermaidViewer /><ReviewViewer /><GitView /><Previewer /><LinkAnchorMenu />
      <CommandPalette />
      <Show when={desktop()}><ShortcutsDialog /></Show>
      <DesktopNative />
    </RecentsProvider></HostsProvider></SettingsProvider>
  );
}
/** `/files` and `/files/edit` share one route: on the phone the editor is a full-screen layer and
 * the list isn't rendered under it (as before); on the desktop shell the list stays in its pane
 * next to the editor (UNI-117), same instance, so its folder and scroll survive. */
function FilesRoute() {
  const location = useLocation();
  return <Show when={location.pathname === '/files' || layout().desktop()}><Files /></Show>;
}
/** Screens that need the native core show the standard notice in web mode (lib/platform.ts). */
const gated = (cap: Capability, Screen: Component, title: string) => () => (
  <Requires cap={cap} fallback={<div class="page-screen"><PageHeader title={title} /><UnavailableNotice cap={cap} /></div>}><Screen /></Requires>
);
/** `/` and `/term` share one Home instance (no remount when a terminal
 * goes to the background); the terminal layer lives in the shell. `/files/edit`
 * is rendered by the shell directly (same full-screen-layer pattern) so
 * the Files list underneath keeps its scroll position. */
export function Root() { return <HashRouter root={Shell}><Route path={['/', '/term']} component={gated('terminal', Sessions, 'Sessions')} /><Route path="/chat" component={gated('chat', Chat, 'Chat')} /><Route path="/board" component={Board} /><Route path={['/files', '/files/edit']} component={gated('files', FilesRoute, 'Files')} /><Route path="/cron" component={Cron} /><Route path="/settings" component={Settings} /><Route path="/setup" component={SetupWizard} /><Route path="/preview" component={Preview} /></HashRouter>; }
const root = document.getElementById('root');
/** Visible viewport height (shrinks with the soft keyboard) for sheet caps. */
function trackVisualViewport(): void {
  const set = () => {
    const vv = window.visualViewport;
    // Android (adjustResize + edge-to-edge) reports the keyboard as a
    // native inset (MainActivity → window.unipiImeInset); a browser pans
    // the visual viewport instead. Either way: how far sheets must rise.
    const ime = (window as unknown as { unipiImeInset?: number }).unipiImeInset ?? 0;
    const panned = vv ? Math.max(0, window.innerHeight - (vv.offsetTop + vv.height)) : 0;
    const below = Math.max(panned, ime);
    const h = Math.min(vv?.height ?? window.innerHeight, window.innerHeight - ime);
    const root = document.documentElement.style;
    root.setProperty('--vvh', `${Math.round(h)}px`);
    root.setProperty('--vv-bottom', `${Math.round(below)}px`);
    // Keep a focused field inside a (now shorter) sheet in view.
    const active = document.activeElement;
    if (active instanceof HTMLElement && active.closest('.sheet')) {
      requestAnimationFrame(() => active.scrollIntoView({ block: 'center' }));
    }
  };
  set();
  window.visualViewport?.addEventListener('resize', set);
  window.visualViewport?.addEventListener('scroll', set);
  window.addEventListener('resize', set);
  window.addEventListener('unipi-ime', set);
}
if (root) {
  // Web build served by the kanboard daemon: `/p/<slug>` / `?project=` links → #/board.
  if (isWeb()) applyWebEntry();
  trackVisualViewport();
  watchForUpdates();
  watchDesktopUpdates();
  render(() => <Root />, root);
  void dismissSplashWhenReady();
}
