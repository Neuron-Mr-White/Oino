/**
 * UNI-263: desktop app updates — the card in Settings → About and the prompt
 * that opens when a newer version is out (lib/desktopUpdate.ts). Plus the
 * iPhone notice (ad hoc builds can't update themselves).
 */
import { Match, Show, Switch } from 'solid-js';
import { Download, ExternalLink, RefreshCw, Sparkles } from 'lucide-solid';
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
import { checkIosUpdate, iosUpdate } from '../lib/update';
import { openExternal } from '../lib/board/share';

const mb = (n: number) => `${(n / 1024 / 1024).toFixed(1)} MB`;

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
            <em>{d().kind === 'installing' ? 'Installing… UniPi restarts by itself' : `${mb(d().done ?? 0)}${d().total ? ` of ${mb(d().total!)}` : ''}`}</em>
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

function headline(s: DesktopUpdateState): string {
  switch (s.kind) {
    case 'checking':
      return 'Checking for updates…';
    case 'current':
      return 'UniPi is up to date';
    case 'available':
      return `UniPi ${s.info.version} is available`;
    case 'downloading':
      return `Downloading UniPi ${s.info.version}…`;
    case 'installing':
      return `Installing UniPi ${s.info.version}…`;
    case 'error':
      return s.info?.version ? `Updating to ${s.info.version} failed` : 'Update check failed';
    default:
      return 'Updates';
  }
}

/** Settings → About (desktop app). */
export function DesktopUpdateCard() {
  const s = desktopUpdateState;
  return (
    <div class="update-card" data-testid="desktop-update-card">
      <div class="update-head">
        <Sparkles size={16} class="shrink-0 board-accent" />
        <span class="flex-1 min-w-0">
          <span class="block text-sm">{headline(s())}</span>
          <span class="block text-xs text-text-dim">
            <Show when={infoOf(s())} fallback="UniPi checks unipi.nrn.one on start and every few hours, and asks before updating">
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
    </div>
  );
}

/** The prompt: opens when a newer version is out; nothing installs until "Update and restart". */
export function DesktopUpdatePrompt() {
  const s = desktopUpdateState;
  const busy = () => s().kind === 'downloading' || s().kind === 'installing';
  return (
    <BoardSheet
      open={promptOpen()}
      onClose={() => {
        if (!busy()) dismissDesktopUpdate();
      }}
      label={headline(s())}
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
          fallback={<>This copy was installed from a package, so it can't replace itself. Download the new version from unipi.nrn.one and install it the same way.</>}
        >
          You have {infoOf(s())?.current}. UniPi downloads the new version, checks its signature, installs it and restarts. Open chats and terminals reconnect after the restart.
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

/** iPhone (ad hoc build): "a new version is out — open unipi.nrn.one/ios on this iPhone". */
export function IosUpdateCard() {
  const s = iosUpdate;
  return (
    <div class="update-card" data-testid="ios-update-card">
      <div class="update-head">
        <Sparkles size={16} class="shrink-0 board-accent" />
        <span class="flex-1 min-w-0">
          <span class="block text-sm">
            <Show when={s().release} fallback={s().checking ? 'Checking for updates…' : s().current ? 'UniPi is up to date' : 'Updates'}>
              UniPi {s().release!.version} is available
            </Show>
          </span>
          <span class="block text-xs text-text-dim">
            <Show when={s().release} fallback="This iPhone build can't update itself; UniPi tells you when a new one is out">
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
    </div>
  );
}
