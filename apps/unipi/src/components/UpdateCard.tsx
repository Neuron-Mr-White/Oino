/**
 * Update card (Settings, and a slim banner on Home when a build is ready to
 * fetch). Driven by lib/update.ts; the APK comes from unipi.nrn.one.
 */
import { Match, Show, Switch } from 'solid-js';
import { Download, RefreshCw, Sparkles } from 'lucide-solid';
import { checkForUpdate, installed, installNow, startDownload, updateState } from '../lib/update';

const mb = (n: number) => `${(n / 1024 / 1024).toFixed(n > 100 * 1024 * 1024 ? 0 : 1)} MB`;

export function UpdateCard() {
  const cur = installed();
  const s = updateState;
  return (
    <div class="update-card" data-testid="update-card">
      <div class="update-head">
        <Sparkles size={16} class="shrink-0 board-accent" />
        <span class="flex-1 min-w-0">
          <span class="block text-sm">
            <Switch fallback="UniPi is up to date">
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
            <Show when={cur} fallback="Get the APK from unipi.nrn.one">
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
    </div>
  );
}

/** Slim banner above the content when an update is waiting (tap → Settings). */
export function UpdateBanner(props: { onOpen: () => void }) {
  const s = updateState;
  return (
    <Show when={s().kind === 'available' || s().kind === 'ready'}>
      <button type="button" class="update-banner" onClick={() => props.onOpen()}>
        <Sparkles size={15} class="shrink-0" />
        <span class="flex-1 truncate text-left">
          UniPi {(s() as { release: { version: string } }).release.version} ({(s() as { release: { build: number } }).release.build}) is ready
        </span>
        <span class="update-banner-go">{s().kind === 'ready' ? 'Install' : 'Update'}</span>
      </button>
    </Show>
  );
}
