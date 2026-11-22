/**
 * What this UniPi Oino install ships (P0-01): the app, the bundled
 * `unipi-oino-host`, the `oino` engine and the bundled extensions — from the
 * core's `about_components`, which only reads the install itself.
 *
 * The installer bundles everything, so a fresh PC opens straight into the app.
 * There is no setup screen: when the install is genuinely broken (the host or
 * the extensions are missing) `InstallCheck` says so once, with the fix, and
 * Settings → About → Components keeps showing it.
 */
import { createSignal, onMount } from 'solid-js';
import { z } from 'zod';
import { call } from './ipc';
import { platform } from './platform';
import { toasts } from './toasts';
import { openSettingsAt } from './settings/open';

const BinarySchema = z.object({ version: z.string().optional(), path: z.string().optional() });

export const ComponentsSchema = z.object({
  app: z.string(),
  host: BinarySchema,
  oino: BinarySchema,
  extensions: z.object({ count: z.number(), dir: z.string().optional() }),
  problems: z.array(z.object({ component: z.string(), message: z.string(), fix: z.string() })),
});
export type Components = z.infer<typeof ComponentsSchema>;

export type ComponentsState =
  | { kind: 'idle' | 'loading' }
  | { kind: 'ready'; info: Components }
  | { kind: 'error'; message: string };

const [state, setState] = createSignal<ComponentsState>({ kind: 'idle' });
export const componentsState = state;

let inflight: Promise<ComponentsState> | undefined;

/** Reads the install again (desktop only; a phone has no bundled host). */
export function refreshComponents(): Promise<ComponentsState> {
  if (platform().mode !== 'tauri-desktop') return Promise.resolve(state());
  if (inflight) return inflight;
  if (state().kind !== 'ready') setState({ kind: 'loading' });
  inflight = call('about_components', {}, ComponentsSchema).then((result) => {
    inflight = undefined;
    const next: ComponentsState = result.ok ? { kind: 'ready', info: result.value } : { kind: 'error', message: result.error.message };
    setState(next);
    return next;
  });
  return inflight;
}

/** Test support. */
export function resetComponents(): void {
  inflight = undefined;
  setState({ kind: 'idle' });
}

/** The one-line message a broken install shows (message + fix), or undefined. */
export function brokenInstallMessage(info: Components | undefined): string | undefined {
  const first = info?.problems[0];
  return first ? `${first.message} ${first.fix}` : undefined;
}

/**
 * Mounted once by the shell: on a desktop install that is genuinely broken,
 * one error toast with the fix and a way to the About card. Never navigates,
 * never blocks the app, never offers to install anything.
 */
export function InstallCheck() {
  onMount(() => {
    if (platform().mode !== 'tauri-desktop') return;
    void refreshComponents().then((s) => {
      const message = s.kind === 'ready' ? brokenInstallMessage(s.info) : undefined;
      if (message) toasts().push(message, 'error', { label: 'Details', run: () => openSettingsAt('about', 'about-components') });
    });
  });
  return null;
}
