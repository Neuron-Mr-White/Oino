/**
 * The one Update card (WP-A24, G-14): Settings › About shows exactly one card,
 * fed by this app's own updater (desktop in-app, the Android APK feed, or the
 * iOS ad-hoc notice) plus the host's release row (its version and the update
 * notice the host raises on `events_open`). The `updater` extension's overlay
 * is never opened by the app.
 *
 * Also hosts `DesktopUpdatePrompt` (the sheet a new desktop release opens) and
 * the slim `UpdateBanner` above Home content (absorbed from the former
 * DesktopUpdate.tsx — one file, one updater surface).
 */
import { Match, Show, Switch, createResource, createSignal } from 'solid-js';
import { Download, ExternalLink, RefreshCw, Server, Sparkles, X } from 'lucide-solid';
import { BoardSheet } from './board/BoardSheet';
import {
  checkDesktopUpdate,
  desktopUpdateState,
  dismissDesktopUpdate,
  installDesktopUpdate,
  promptOpen,
  type DesktopUpdateState,
  type UpdateInfo,
} from '../lib/desktopUpdate';
import { checkForUpdate, checkIosUpdate, installed, installNow, iosUpdate, startDownload, updateState } from '../lib/update';
import { openExternal } from '../lib/board/share';
import { hostClient } from '../lib/host/factory';
import { LOCAL_HOST } from '../lib/host/client';
import { resolveHostId } from '../lib/host/localId';
import { refreshThisComputer, thisComputer } from '../lib/thisComputer';
import { hubs } from '../lib/session/hub';
import { can, platform } from '../lib/platform';

const mb = (n: number) => `${(n / 1024 / 1024).toFixed(n > 100 * 1024 * 1024 ? 0 : 1)} MB`;

type HostInfoLike = { version?: string; name?: string };

/** The host's release row inside the card: its version + any update notice. */
function HostRow() {
  const hub = () => hubs().peek(resolveHostId(LOCAL_HOST));
  // D-22: the version comes from the local admin channel (the same `host_info` Settings › This computer
  // reads); the host-session request has no version field, so the row used to read "Host —".
  const [info] = createResource(async (): Promise<HostInfoLike | undefined> => {
    try {
      if (thisComputer().kind !== 'ready') await refreshThisComputer();
      const state = thisComputer();
      if (state.kind === 'ready') return { version: state.info.version };
      return await hostClient(LOCAL_HOST).request<HostInfoLike>('host_info');
    } catch {
      return undefined;
    }
  });
  const notice = () => hub()?.state.notices.find((n) => /update|release/i.test(n.text));
  return (
    <div class="update-host" data-testid="update-host-row">
      <Server size={14} class="shrink-0" />
      <span class="min-w-0">
        Host {info()?.version ?? '—'}
        <Show when={notice()}>
          <span class="block text-xs text-amber"> {notice()!.text}</span>
        </Show>
      </span>
    </div>
  );
}

/** The one card. */
export function UpdateCard() {
  return (
    <div class="update-card" data-testid="update-card">
      <Show when={platform().os === 'ios'} fallback={<Show when={can('desktopUpdater')} fallback={<ApkBody />}><DesktopBody /></Show>}>
        <IosBody />
      </Show>
      <HostRow />
    </div>
  );
}

/** The Android APK updater body (the former UpdateCard). */
function ApkBody() {
  const cur = installed();
  const s = updateState;
  return (
    <>
      <div class="update-head">
        <Sparkles size={16} class="shrink-0 board-accent" />
        <span class="flex-1 min-w-0">
          <span class="block text-sm">
            <Switch fallback="Unipi is up to date">
              <Match when={!cur}>Updates come with the Android app</Match>
              <Match when={s().kind === 'checking'}>Checking for updates…</Match>
              <Match when={s().kind === 'idle'}>Updates</Match>
              <Match when={'release' in s() && s().kind !== 'error'}>
                Version {(s() as { release: { version: string; build: number } }).release.version} ({(s() as { release: { build: number } }).release.build}) is ready
              </Match>
              <Match when={s().kind === 'error'}>Update failed</Match>
            </Switch>
          </span>
          <span class="block text-xs text-text-dim">
            <Show when={cur} fallback="Get the APK from oino.dev/download">
              Installed: {cur!.versionName} ({cur!.versionCode})
            </Show>
          </span>
        </span>
      </div>
      <Show when={s().kind === 'error'}>
        <p class="update-error">{(s() as { message: string }).message}</p>
      </Show>
      <Show when={s().kind === 'available' && (s() as { release: { notes?: string } }).release.notes}>
        <p class="update-notes">{(s() as { release: { notes?: string } }).release.notes}</p>
      </Show>
      <Show when={s().kind === 'downloading'}>
        {(() => {
          const d = () => s() as { done: number; total: number };
          return (
            <div class="update-progress" role="progressbar" aria-valuenow={d().total ? Math.round((d().done / d().total) * 100) : undefined}>
              <span style={{ width: d().total ? `${Math.min(100, (d().done / d().total) * 100)}%` : '30%' }} />
              <em>
                {mb(d().done)}
                {d().total ? ` of ${mb(d().total)}` : ''}
              </em>
            </div>
          );
        })()}
      </Show>
      <Show when={cur}>
        <div class="update-actions">
          <Switch>
            <Match when={s().kind === 'available' || (s().kind === 'error' && 'release' in s() && (s() as { release?: unknown }).release)}>
              <button type="button" class="update-btn primary" onClick={() => startDownload((s() as { release: Parameters<typeof startDownload>[0] }).release)}>
                <Download size={16} /> Update
              </button>
            </Match>
            <Match when={s().kind === 'ready'}>
              <button type="button" class="update-btn primary" onClick={() => installNow()}>
                <Download size={16} /> Install
              </button>
            </Match>
            <Match when={s().kind !== 'downloading'}>
              <button type="button" class="update-btn" disabled={s().kind === 'checking'} onClick={() => void checkForUpdate(true)}>
                <RefreshCw size={15} /> Check now
              </button>
            </Match>
          </Switch>
        </div>
      </Show>
    </>
  );
}

function infoOf(s: DesktopUpdateState): UpdateInfo | undefined {
  return 'info' in s ? s.info : undefined;
}

function Progress(props: { s: DesktopUpdateState }) {
  return (
    <Show when={props.s.kind === 'downloading' || props.s.kind === 'installing'}>
      {(() => {
        const d = () => props.s as { kind: string; done?: number; total?: number };
        const pct = () => (d().kind === 'installing' ? 100 : d().total ? Math.min(100, ((d().done ?? 0) / d().total!) * 100) : 30);
        return (
          <div class="update-progress" role="progressbar" aria-valuenow={Math.round(pct())} data-testid="desktop-update-progress">
            <span style={{ width: `${pct()}%` }} />
            <em>{d().kind === 'installing' ? 'Installing… Unipi restarts by itself' : `${mb(d().done ?? 0)}${d().total ? ` of ${mb(d().total!)}` : ''}`}</em>
          </div>
        );
      })()}
    </Show>
  );
}

function Actions(props: { s: DesktopUpdateState; compact?: boolean }) {
  const info = () => infoOf(props.s);
  const canInstall = () => info()?.kind === 'in_app' && !!info()?.version;
  return (
    <div class="update-actions">
      <Switch>
        <Match when={props.s.kind === 'downloading' || props.s.kind === 'installing'}>{null}</Match>
        <Match when={info()?.version && !canInstall()}>
          <button type="button" class="update-btn primary" data-testid="desktop-update-download" onClick={() => void openExternal(info()!.downloadPage)}>
            <ExternalLink size={15} /> Download {info()!.version}
          </button>
        </Match>
        <Match when={canInstall() && (props.s.kind === 'available' || props.s.kind === 'error')}>
          <button type="button" class="update-btn primary" data-testid="desktop-update-install" onClick={() => void installDesktopUpdate()}>
            <Download size={16} /> {props.s.kind === 'error' ? 'Try again' : 'Update and restart'}
          </button>
        </Match>
      </Switch>
      <Show when={!props.compact && props.s.kind !== 'downloading' && props.s.kind !== 'installing'}>
        <button type="button" class="update-btn" data-testid="desktop-update-check" disabled={props.s.kind === 'checking'} onClick={() => void checkDesktopUpdate(true)}>
          <RefreshCw size={15} class={props.s.kind === 'checking' ? 'refresh-spinning' : undefined} /> Check for updates
        </button>
      </Show>
    </div>
  );
}

function desktopHeadline(s: DesktopUpdateState): string {
  switch (s.kind) {
    case 'checking':
      return 'Checking for updates…';
    case 'current':
      return 'Unipi is up to date';
    case 'available':
      return `Unipi ${s.info.version} is available`;
    case 'downloading':
      return `Downloading Unipi ${s.info.version}…`;
    case 'installing':
      return `Installing Unipi ${s.info.version}…`;
    case 'error':
      return s.info?.version ? `Updating to ${s.info.version} failed` : 'Update check failed';
    default:
      return 'Updates';
  }
}

/** The desktop in-app updater body (the former DesktopUpdateCard). */
function DesktopBody() {
  const s = desktopUpdateState;
  return (
    <>
      <div class="update-head">
        <Sparkles size={16} class="shrink-0 board-accent" />
        <span class="flex-1 min-w-0">
          <span class="block text-sm">{desktopHeadline(s())}</span>
          <span class="block text-xs text-text-dim">
            <Show when={infoOf(s())} fallback="Unipi checks oino.dev on start and every few hours, and asks before updating">
              Installed: {infoOf(s())!.current}
              {infoOf(s())!.kind === 'manual' ? ' · installed from a package: updates are downloaded by hand' : ''}
            </Show>
          </span>
        </span>
      </div>
      <Show when={s().kind === 'error'}>
        <p class="update-error" data-testid="desktop-update-error">{(s() as { message: string }).message}</p>
      </Show>
      <Show when={s().kind === 'available' && infoOf(s())?.notes}>
        <p class="update-notes">{infoOf(s())!.notes}</p>
      </Show>
      <Progress s={s()} />
      <Actions s={s()} />
    </>
  );
}

/** The iPhone ad-hoc body (the former IosUpdateCard). */
function IosBody() {
  const s = iosUpdate;
  return (
    <>
      <div class="update-head">
        <Sparkles size={16} class="shrink-0 board-accent" />
        <span class="flex-1 min-w-0">
          <span class="block text-sm">
            <Show when={s().release} fallback={s().checking ? 'Checking for updates…' : s().current ? 'Unipi is up to date' : 'Updates'}>
              Unipi {s().release!.version} is available
            </Show>
          </span>
          <span class="block text-xs text-text-dim">
            <Show when={s().release} fallback="This iPhone build can't update itself; Unipi tells you when a new one is out">
              Open {s().release!.page.replace('https://', '')} in Safari on this iPhone to install it
            </Show>
          </span>
        </span>
      </div>
      <Show when={s().error}>
        <p class="update-error">{s().error}</p>
      </Show>
      <div class="update-actions">
        <Show when={s().release}>
          <button type="button" class="update-btn primary" data-testid="ios-update-open" onClick={() => void openExternal(s().release!.page)}>
            <ExternalLink size={15} /> Open in Safari
          </button>
        </Show>
        <button type="button" class="update-btn" disabled={s().checking} onClick={() => void checkIosUpdate(true)}>
          <RefreshCw size={15} /> Check now
        </button>
      </div>
    </>
  );
}

const BANNER_DISMISSED_KEY = 'unipi.updateBanner.dismissed';
function readBannerDismissed(): string | undefined {
  try {
    return localStorage.getItem(BANNER_DISMISSED_KEY) ?? undefined;
  } catch {
    return undefined;
  }
}
const [bannerDismissed, setBannerDismissed] = createSignal<string | undefined>(readBannerDismissed());
function dismissBanner(version: string): void {
  setBannerDismissed(version);
  try {
    localStorage.setItem(BANNER_DISMISSED_KEY, version);
  } catch {
    /* storage unavailable: dismissed until the app restarts */
  }
}

/** Slim banner above the content when an update is waiting (tap → Settings). Dismissible (×) per
 *  version, remembered: a newer version shows it again ([ui-shell] #4). */
export function UpdateBanner(props: { onOpen: () => void }) {
  const s = updateState;
  const d = desktopUpdateState;
  const desktopReady = () => d().kind === 'available';
  const apk = () => s() as { release: { version: string; build: number } };
  const version = () =>
    desktopReady() ? (d() as { info: { version: string } }).info.version : s().kind === 'available' || s().kind === 'ready' ? `${apk().release.version}+${apk().release.build}` : undefined;
  const label = () => (desktopReady() ? `Unipi ${(d() as { info: { version: string } }).info.version} is available` : `Unipi ${apk().release.version} (${apk().release.build}) is ready`);
  return (
    <Show when={version() !== undefined && version() !== bannerDismissed()}>
      <div class="update-banner" data-testid="update-banner">
        <button type="button" class="update-banner-main" onClick={() => props.onOpen()}>
          <Sparkles size={15} class="shrink-0" />
          <span class="flex-1 truncate text-left">{label()}</span>
          <span class="update-banner-go">{desktopReady() || s().kind === 'available' ? 'Update' : 'Install'}</span>
        </button>
        <button type="button" class="update-banner-close" aria-label={`Dismiss: ${label()}`} title="Dismiss" data-testid="update-banner-dismiss" onClick={() => dismissBanner(version()!)}>
          <X size={15} />
        </button>
      </div>
    </Show>
  );
}

/** The prompt: opens when a newer desktop version is out; nothing installs until "Update and restart". */
export function DesktopUpdatePrompt() {
  const s = desktopUpdateState;
  const busy = () => s().kind === 'downloading' || s().kind === 'installing';
  return (
    <BoardSheet
      open={promptOpen()}
      onClose={() => {
        if (!busy()) dismissDesktopUpdate();
      }}
      label={desktopHeadline(s())}
      testId="desktop-update-prompt"
      guarded={busy()}
      footer={
        busy() ? undefined : (
          <div class="flex gap-2 justify-end w-full">
            <button type="button" class="btn btn-secondary" data-testid="desktop-update-later" onClick={() => dismissDesktopUpdate()}>
              Later
            </button>
            <Actions s={s()} compact />
          </div>
        )
      }
    >
      <p class="preview-muted" style={{ margin: '0 0 8px' }}>
        <Show
          when={infoOf(s())?.kind === 'in_app'}
          fallback={<>This copy was installed from a package, so it can't replace itself. Download the new version from oino.dev/download and install it the same way.</>}
        >
          You have {infoOf(s())?.current}. Unipi downloads the new version, checks its signature, installs it and restarts. Open chats and terminals reconnect after the restart.
        </Show>
      </p>
      <Show when={infoOf(s())?.notes}>
        <p class="update-notes" data-testid="desktop-update-notes">{infoOf(s())!.notes}</p>
      </Show>
      <Show when={s().kind === 'error'}>
        <p class="update-error">{(s() as { message: string }).message}</p>
      </Show>
      <Progress s={s()} />
    </BoardSheet>
  );
}
