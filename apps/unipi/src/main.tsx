/**
 * App entry + routes (app spec §3.7, WP-A3). The shell (frames, sidebar,
 * rail, side pane, bottom panel, shortcuts) is `components/shell/AppShell`.
 *
 *   /                    last open session of the last host, else the draft for This computer
 *   /s/:host/:session    conversation
 *   /new/:host?cwd=      draft (Z-19, WP-A5 fills it)
 *   /board, /cron        rail pages
 *   /x/:entryId          extension sidebar page (`fullscreen` slot, U10)
 *   /settings/:section?  the one Settings screen
 *   /setup               (gone: a stale link lands on `/`)
 * Phone only (until A20 / A29): /chat (the Chat tab list), /files(/edit), /preview
 * (the forward viewer; on the desktop shell ports live in the side pane `+`
 * launcher, Z-104). On the desktop shell those redirect to `/`.
 */
import { render } from 'solid-js/web';
import { HashRouter, Navigate, Route, useLocation, useParams, useSearchParams } from '@solidjs/router';
import { Show } from 'solid-js';
import type { Component, JSX } from 'solid-js';
import { Puzzle } from 'lucide-solid';
import Board from './screens/Board';
import Chat from './screens/Chat';
import Cron from './screens/Cron';
import Preview from './screens/Preview';
import Settings from './screens/Settings';
import { FilesLayer } from './components/files/FilesTab';
import { PageHeader } from './components/PageHeader';
import { EmptyState } from './components/EmptyState';
import { Requires, UnavailableNotice } from './components/Requires';
import { AppShell } from './components/shell/AppShell';
import { SessionTabLayer } from './components/panes/tabs';
import { RootBoundary } from './components/ErrorBoundaries';
import { readLastSession, Workspace } from './components/shell/Workspace';
import { sessionHref } from './components/shell/Sidebar';
import { extPageView } from './components/shell/Rail';
import { SettingsNotices } from './components/SettingsNotices';
import { UpdateBanner } from './components/UpdateCard';
import { LOCAL_HOST } from './lib/host/client';
import { localHostId, resolveHostId } from './lib/host/localId';
import { useHostsOptional } from './hosts/context';
import { layout } from './lib/layout';
import { can, isWeb, type Capability } from './lib/platform';
import { dismissSplashWhenReady } from './lib/splash';
import { watchForUpdates } from './lib/update';
import { watchDesktopUpdates } from './lib/desktopUpdate';
import { announceWebviewRecovery } from './lib/webviewRecovery';
import { startKeyboardTracking } from './components/phone/services';
import './components/media/media.css';
import '@xterm/xterm/css/xterm.css';
import './style.css';
import './components/shell/shell.css';
import './components/panes/panes.css';
// Shared tokens + components (UNI-218): last, so it wins over the per-screen rules it unifies.
import './ui.css';
// Phone design system (phone spec §2): tokens, z scale, primitives — after ui.css, scoped to the phone shell.
import './components/phone/phone.css';

const desktop = () => layout().desktop();

/** Screens that need the native core show the standard notice in web mode (lib/platform.ts). */
const gated = (cap: Capability, Screen: Component, title: string) => () => (
  <Requires cap={cap} fallback={<div class="page-screen"><PageHeader title={title} /><UnavailableNotice cap={cap} /></div>}><Screen /></Requires>
);

/** A rail page: on the desktop shell it gets the page column (banner, notices, optional header). */
const railPage = (Screen: Component, title?: string) => () => (
  <Show when={desktop()} fallback={<Screen />}>
    <div class="app-shell ashell-page">
      <Show when={title}><PageHeader title={title!} /></Show>
      <main class="app-content">
        <Show when={title !== 'Settings'}><UpdateBanner onOpen={() => (location.hash = '#/settings')} /></Show>
        <SettingsNotices />
        <Screen />
      </main>
    </div>
  </Show>
);

/** Phone-only routes; the desktop shell sends them home. */
const phoneOnly = (Screen: Component) => () => <Show when={!desktop()} fallback={<Navigate href="/" />}><Screen /></Show>;

/** `/`: the web build (board only) opens the Board; the phone its Chat tab; the desktop the last session or a draft. */
function Home() {
  if (!can('chat') && can('board')) return <Navigate href="/board" />;
  if (!desktop()) return <Navigate href="/chat" />;
  const last = readLastSession();
  return <Navigate href={last ? sessionHref(resolveHostId(last.hostId), last.sessionId) : `/new/${resolveHostId(LOCAL_HOST)}`} />;
}

/**
 * `/new/local`, `/s/local/…`: the `local` alias is resolved here, once — the
 * route is replaced by This computer's real id, so no screen ever keys hubs,
 * sessions or panes by the alias (lib/host/localId). While the hosts load it
 * waits; with no local host at all (web / harness) the alias renders as is.
 */
function HostRoute(props: { host: string; children: (hostId: string) => JSX.Element }) {
  const hosts = useHostsOptional();
  const location = useLocation();
  const real = () => (props.host === LOCAL_HOST ? localHostId() : props.host);
  const settled = () => !hosts || (!hosts.loading() && (hosts.hosts().length > 0 || !!hosts.error()));
  return (
    <Show when={real()} fallback={<Show when={props.host === LOCAL_HOST && settled() && !localHostId()}>{props.children(props.host)}</Show>}>
      {(id) => (
        <Show when={id() === props.host} fallback={<Navigate href={`${location.pathname.replace(/^\/(new|s)\/local\b/, `/$1/${encodeURIComponent(id())}`)}${location.search}`} />}>
          {props.children(id())}
        </Show>
      )}
    </Show>
  );
}

function ConversationRoute() {
  const params = useParams<{ host: string; session: string }>();
  const body = () => (
    <HostRoute host={params.host}>
      {(hostId) => (
        <Show when={params.session} keyed>
          {(session) => <Workspace hostId={hostId} sessionId={session} />}
        </Show>
      )}
    </HostRoute>
  );
  return <Show when={desktop()} fallback={<div class="ws-layer">{body()}</div>}>{body()}</Show>;
}

function DraftRoute() {
  const params = useParams<{ host: string }>();
  const [search] = useSearchParams<{ cwd?: string }>();
  const body = () => <HostRoute host={params.host}>{(hostId) => <Workspace hostId={hostId} cwd={search.cwd} />}</HostRoute>;
  return <Show when={desktop()} fallback={<div class="ws-layer">{body()}</div>}>{body()}</Show>;
}

function ExtPageRoute() {
  const params = useParams<{ entryId: string }>();
  return (
    <Show
      when={extPageView()}
      fallback={<EmptyState icon={<Puzzle size={22} />} title="This page isn't available" body="The extension that adds it is off or not installed." testId="ext-page-missing" />}
    >
      {(View) => {
        const V = View();
        return <V entryId={decodeURIComponent(params.entryId)} />;
      }}
    </Show>
  );
}

export function Root() {
  return (
    <RootBoundary>
      <HashRouter root={AppShell}>
        <Route path="/" component={Home} />
        <Route path="/s/:host/:session" component={ConversationRoute} />
        <Route path="/new/:host" component={DraftRoute} />
        <Route path="/chat" component={phoneOnly(gated('chat', Chat, 'Chat'))} />
        <Route path="/board" component={railPage(Board)} />
        <Route path="/cron" component={railPage(Cron)} />
        <Route path="/x/:entryId" component={railPage(ExtPageRoute)} />
        <Route path={['/settings', '/settings/:section']} component={railPage(Settings, 'Settings')} />
        <Route path="/setup" component={() => <Navigate href="/" />} />
        <Route path={['/files', '/files/edit']} component={phoneOnly(gated('files', FilesLayer, 'Files'))} />
        <Route path="/preview" component={phoneOnly(Preview)} />
      </HashRouter>
      {/* WP-A10: the phone's session-tab sheets / diff layer (no-ops on the desktop). */}
      <SessionTabLayer />
    </RootBoundary>
  );
}
const root = document.getElementById('root');
if (root) {
  startKeyboardTracking();
  watchForUpdates();
  watchDesktopUpdates();
  render(() => <Root />, root);
  void dismissSplashWhenReady();
  if (can('hosts')) void announceWebviewRecovery();
}
