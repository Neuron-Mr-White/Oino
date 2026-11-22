/**
 * The app's own host supervisor in Settings → This computer (UNI-235/A5): the
 * crash banner, the version-skew card ("Update the background host") and
 * "Keep available when UniPi is closed" (UNI-238/A7 — the systemd user unit).
 *
 * The host's own status row (version, who runs it, Restart) and the host log
 * live in `ThisComputer`, which owns the sharing status this section is about;
 * they are rendered around this card in the order the section wants.
 *
 * Everything here is desktop-only: the commands answer an error on the phone,
 * and this component renders nothing there.
 */
import { createSignal, onCleanup, onMount, Show } from 'solid-js';
import { AlertTriangle, Power, RefreshCw, Server } from 'lucide-solid';
import { call } from '../lib/ipc';
import { z } from 'zod';
import { can } from '../lib/platform';
import { alwaysOn, AlwaysOnSchema, type AlwaysOn } from '../lib/keepAvailable';
import { changeSharing } from '../lib/sharing';

const RuntimeSchema = z.object({
  plan: z.enum(['adopt', 'start-service', 'spawn', 'unsupported']),
  runBy: z.enum(['service', 'app', 'manual']).nullish(),
  version: z.string().nullish(),
  bin: z.string().nullish(),
  skew: z.enum(['ours-newer', 'theirs-newer']).nullish(),
  banner: z.string().nullish(),
  logTail: z.array(z.string()).default([]),
});
export type HostRuntime = z.infer<typeof RuntimeSchema>;

export async function hostRuntime(): Promise<HostRuntime | undefined> {
  const r = await call('host_runtime', undefined, RuntimeSchema);
  return r.ok ? r.value : undefined;
}
export async function hostLog(): Promise<string[]> {
  const r = await call('host_log', { lines: 200 }, z.array(z.string()).default([]));
  return r.ok && Array.isArray(r.value) ? r.value : [];
}
/** Restarts this PC's host (the host status row's button). */
export async function restartHost(): Promise<void> {
  await call('host_restart', undefined, z.string());
}

/** The supervisor card: crash banner, version skew, keep-available (+ linger). */
export function HostSupervisorCard() {
  if (!can('localSharing')) return null;
  const [runtime, setRuntime] = createSignal<HostRuntime | undefined>();
  const [always, setAlways] = createSignal<AlwaysOn | undefined>();
  const [error, setError] = createSignal<string | undefined>();
  const [busy, setBusy] = createSignal<string | undefined>();

  const refresh = async () => {
    setRuntime(await hostRuntime());
    setAlways(await alwaysOn());
  };
  onMount(() => void refresh());
  const timer = setInterval(() => void refresh(), 10_000);
  onCleanup(() => clearInterval(timer));

  const act = async (what: string, run: () => Promise<unknown>) => {
    setBusy(what);
    setError(undefined);
    try {
      await run();
      await refresh();
    } catch (e) {
      setError((e as { message?: string })?.message ?? String(e));
    } finally {
      setBusy(undefined);
    }
  };
  const updateSkew = () => act('skew', async () => void (await call('host_update_service', undefined, z.null())));
  const setAlwaysOn = (on: boolean) => act('always', async () => void (await call('always_on_set', { on }, AlwaysOnSchema)));
  // The one path that needs root — the same `sharing_change` the rest of
  // this section uses (the host CLI's `linger enable`, pkexec under it).
  const allowLinger = () => act('linger', async () => void (await changeSharing({ kind: 'linger_enable' })));

  const r = runtime;
  return (
    <div data-testid="host-supervisor">
      <Show when={r()?.banner}>
        {(banner) => (
          <div class="share-row-detail" role="alert" data-testid="host-banner">
            <p class="text-[15px] text-text">
              <AlertTriangle size={16} class="inline board-accent" /> {banner()}
            </p>
            <p class="text-xs text-text-dim mt-0.5">It restarts by itself; the host log at the end of this section says why it stopped.</p>
          </div>
        )}
      </Show>

      <Show when={r()?.skew === 'ours-newer'}>
        <div class="settings-row" data-testid="host-skew">
          <span class="settings-icon"><Server size={18} /></span>
          <div class="min-w-0">
            <p class="text-[15px] text-text">Update the background host</p>
            <p class="text-xs text-text-dim mt-0.5">
              This app ships a newer host ({r()?.version ? `the running one is ${r()!.version}` : 'the running one is older'}). Updating copies it over ~/.local/bin/unipi-host and restarts the service — phones drop for
              about 3 seconds and reconnect by themselves.
            </p>
          </div>
          <div class="shrink-0">
            <button type="button" class="btn btn-primary btn-sm" data-testid="host-skew-update" disabled={!!busy()} onClick={() => void updateSkew()}>
              <Show when={busy() === 'skew'} fallback={'Update'}>
                <RefreshCw size={16} class="refresh-spinning" />
              </Show>
            </button>
          </div>
        </div>
      </Show>

      <div class="settings-row">
        <span class="settings-icon"><Power size={18} /></span>
        <div class="min-w-0">
          <p class="text-[15px] text-text">Keep available when Unipi is closed</p>
          <p class="text-xs text-text-dim mt-0.5" data-testid="always-on-detail">
            {always()?.installed
              ? `On — a systemd user service runs the host (${always()?.bin ?? '~/.local/bin/unipi-host'}), even with no Unipi window open${always()?.running ? '' : ' (starting…)'}`
              : 'Off — phones reach this PC only while Unipi is open (it keeps the host running in the tray).'}
          </p>
          {/* Linger is a sub-line of THIS row, not a row of its own: it is the
              same decision ("can phones reach this PC?"), one step further. */}
          <Show when={always()}>
            {(a) => (
              <p class="text-xs text-text-dim mt-0.5" data-testid="always-on-linger">
                {a().linger === true
                  ? 'Also after you log out: on — phones reach this PC even when you are logged out, and after a restart.'
                  : a().linger === false
                    ? `Also after you log out: off — ${(a().lingerHint ?? 'A user service stops when you log out.').replace(/\.$/, '')}.`
                    : "Also after you log out: can't tell on this system (no loginctl)."}{' '}
                <Show when={a().lingerCommand}>
                  <code>{a().lingerCommand}</code>
                </Show>
              </p>
            )}
          </Show>
          <Show when={always()?.linger === false}>
            <button
              type="button"
              class="btn btn-secondary btn-sm"
              data-testid="always-on-linger-allow"
              disabled={!!busy()}
              onClick={() => void allowLinger()}
            >
              <Show when={busy() === 'linger'} fallback={'Allow it'}>
                <RefreshCw size={16} class="refresh-spinning" /> Waiting for password
              </Show>
            </button>
          </Show>
        </div>
        <div class="shrink-0">
          <button
            type="button"
            class="btn btn-secondary btn-sm"
            data-testid="always-on-toggle"
            disabled={!!busy()}
            onClick={() => void setAlwaysOn(!always()?.installed)}
          >
            <Show when={busy() === 'always'} fallback={always()?.installed ? 'Turn off' : 'Turn on'}>
              <RefreshCw size={16} class="refresh-spinning" />
            </Show>
          </button>
        </div>
      </div>

      <Show when={error()}>
        <p class="preview-field-error" role="alert">
          {error()}
        </p>
      </Show>
    </div>
  );
}
