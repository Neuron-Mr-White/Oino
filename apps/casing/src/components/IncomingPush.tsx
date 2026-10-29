/**
 * "Incoming from <PC>" sheet (UNI-214): a file pushed with `unipi-host push`.
 * Mounted once in the shell; it starts the long-poll (lib/push.ts) and shows
 * the oldest incoming file. States: offer → downloading (progress) → done,
 * plus verify failed (checksum), error, and Android's "install unknown apps"
 * permission missing. Plain web mode renders nothing (it can't reach a PC).
 */
import { Match, Show, Switch, createMemo, onCleanup, onMount } from 'solid-js';
import { Check, FileDown, Package, ShieldAlert, TriangleAlert } from 'lucide-solid';
import { BoardSheet } from './board/BoardSheet';
import { useHosts } from '../hosts/context';
import {
  ACTION_LABEL,
  canReceivePushes,
  currentIncoming,
  dismiss,
  humanSize,
  isApk,
  offerActions,
  openInstallSettings,
  platformMode,
  retryInstall,
  runAction,
  startPushPoll,
  type Incoming,
} from '../lib/push';
import './incomingPush.css';

export function IncomingPushRunner() {
  const hosts = useHosts();
  onMount(() => {
    if (!canReceivePushes()) return;
    onCleanup(startPushPoll(() => hosts.hosts()));
    // Back from Android's "install unknown apps" switch: try again.
    const onVisible = () => {
      const inc = currentIncoming();
      if (!document.hidden && inc?.state.kind === 'permission') void retryInstall(inc);
    };
    document.addEventListener('visibilitychange', onVisible);
    onCleanup(() => document.removeEventListener('visibilitychange', onVisible));
  });
  return (
    <Show when={currentIncoming()}>
      {(inc) => <IncomingPushSheet incoming={inc()} />}
    </Show>
  );
}

const pct = (done: number, total: number) => (total ? Math.min(100, Math.round((done / total) * 100)) : 0);

export function IncomingPushSheet(props: { incoming: Incoming }) {
  const inc = () => props.incoming;
  const item = () => inc().item;
  const state = () => inc().state;
  const mode = platformMode();
  const actions = createMemo(() => offerActions(item(), mode));
  const busy = () => state().kind === 'downloading';
  const close = () => {
    // Closing mid-download would orphan it; the sheet stays until it ends.
    if (busy()) return;
    void dismiss(inc());
  };
  const Icon = () => (isApk(item()) ? <Package size={22} /> : <FileDown size={22} />);
  return (
    <BoardSheet
      open
      onClose={close}
      label={`Incoming from ${inc().hostName}`}
      class="push-sheet"
      testId="push-sheet"
      guarded={busy()}
      footer={
        <div class="push-actions">
          <Switch>
            <Match when={state().kind === 'offer'}>
              <Show when={mode !== 'web'} fallback={<button type="button" class="pair-secondary" onClick={close}>Dismiss</button>}>
                {actions().map((action, i) => (
                  <button type="button" class={i === 0 ? 'pair-primary' : 'pair-secondary'} onClick={() => void runAction(inc(), action)}>
                    {ACTION_LABEL[action]}
                  </button>
                ))}
                <button type="button" class="push-dismiss btn btn-ghost" onClick={close}>Dismiss</button>
              </Show>
            </Match>
            <Match when={state().kind === 'downloading'}>
              <button type="button" class="pair-secondary" disabled>Downloading…</button>
            </Match>
            <Match when={state().kind === 'permission'}>
              <button type="button" class="pair-primary" onClick={() => openInstallSettings()}>Open settings</button>
              <button type="button" class="pair-secondary" onClick={() => void retryInstall(inc())}>I’ve allowed it — install</button>
              <button type="button" class="push-dismiss btn btn-ghost" onClick={close}>Dismiss</button>
            </Match>
            <Match when={state().kind === 'verify_failed' || state().kind === 'error'}>
              <button type="button" class="pair-primary" onClick={() => void runAction(inc(), (state() as { action: 'install' | 'save' | 'open' }).action)}>Try again</button>
              <button type="button" class="push-dismiss btn btn-ghost" onClick={close}>Dismiss</button>
            </Match>
            <Match when={state().kind === 'done'}>
              <button type="button" class="pair-primary" onClick={close}>Done</button>
            </Match>
          </Switch>
        </div>
      }
    >
      <div class="push-file" data-state={state().kind}>
        <span class="push-file-icon"><Icon /></span>
        <span class="push-file-text">
          <span class="push-file-name" title={item().name}>{item().name}</span>
          <span class="push-file-meta">{humanSize(item().size)} · from {inc().hostName}</span>
        </span>
      </div>
      <Show when={item().note}>
        <p class="push-note">“{item().note}”</p>
      </Show>
      <Switch>
        <Match when={state().kind === 'offer'}>
          <Show
            when={mode !== 'web'}
            fallback={<p class="push-msg" data-testid="push-web">Receiving files needs the UniPi app — open it on your phone or computer to install or save this.</p>}
          >
            <p class="push-msg">
              {isApk(item()) && mode === 'android'
                ? 'Install it now — Android will ask you to confirm. Or save the APK to Downloads.'
                : mode === 'desktop'
                  ? 'Save it somewhere on this computer, or open it.'
                  : 'Open it in another app, or save it to Downloads.'}
            </p>
          </Show>
        </Match>
        <Match when={state().kind === 'downloading'}>
          {(() => {
            const d = () => state() as { done: number; total: number };
            return (
              <div class="push-progress" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct(d().done, d().total)}>
                <span style={{ width: `${pct(d().done, d().total)}%` }} />
                <em>{humanSize(d().done)} of {humanSize(d().total)} · {pct(d().done, d().total)}%</em>
              </div>
            );
          })()}
          <p class="push-msg push-dim">Checking the file as it arrives…</p>
        </Match>
        <Match when={state().kind === 'permission'}>
          <div class="push-alert" data-testid="push-permission">
            <ShieldAlert size={18} class="shrink-0" />
            <span>
              <strong>Allow UniPi to install apps</strong>
              Android blocks installs from apps until you allow it once: open settings, turn on <em>Allow from this source</em> for UniPi, then come back.
            </span>
          </div>
        </Match>
        <Match when={state().kind === 'verify_failed'}>
          <div class="push-alert" data-testid="push-verify-failed">
            <TriangleAlert size={18} class="shrink-0" />
            <span>
              <strong>The file didn’t arrive intact</strong>
              {/arrive intact/.test((state() as { message: string }).message) ? '' : `${(state() as { message: string }).message} `}Nothing was installed or saved. Try again, or push it again from the PC.
            </span>
          </div>
        </Match>
        <Match when={state().kind === 'error'}>
          <div class="push-alert" data-testid="push-error">
            <TriangleAlert size={18} class="shrink-0" />
            <span>
              <strong>{(state() as { message: string }).message}</strong>
              {(state() as { hint?: string }).hint ?? ''}
            </span>
          </div>
        </Match>
        <Match when={state().kind === 'done'}>
          <div class="push-ok" data-testid="push-done">
            <Check size={18} class="shrink-0" />
            <span>{(state() as { message: string }).message}</span>
          </div>
        </Match>
      </Switch>
    </BoardSheet>
  );
}
