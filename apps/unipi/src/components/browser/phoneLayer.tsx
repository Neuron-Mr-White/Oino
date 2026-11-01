/**
 * The Browser on the phone (browser.md §9.5, app spec §3.9): there is no side pane, so
 * the ⋯ menu's Panes → Browser opens the full-height browser sheet over the chat (the
 * imported `BrowserSheet`: tab strip, URL bar, Take over strip, touch mapping, soft
 * keyboard). Phone defaults (adaptive quality, frames only while shown) are the
 * stream's own. The element picker stays desktop-only for now (§9.5).
 *
 * The sheet mounts once, on first open, in its own root on `document.body` so it never
 * depends on which route is showing.
 */
import { createSignal, Show } from 'solid-js';
import { render } from 'solid-js/web';
import { z } from 'zod';
import { call } from '../../lib/ipc';
import { BrowserSheet } from '../chat/BrowserSide';

const [shown, setShown] = createSignal<string | undefined>();
const [hostName, setHostName] = createSignal('your PC');
let mounted = false;

const HostRow = z.object({ id: z.string(), name: z.string().optional() }).passthrough();

/** The PC's display name (`list_hosts`), "your PC" when it can't be had. */
async function hostNameOf(hostId: string): Promise<string> {
  try {
    const r = await call('list_hosts', {}, z.array(HostRow));
    return (r.ok && r.value.find((h) => h.id === hostId)?.name) || 'your PC';
  } catch {
    return 'your PC';
  }
}

function PhoneBrowserLayer() {
  return (
    <Show when={shown()} keyed>
      {(id: string) => <BrowserSheet open hostId={id} hostName={hostName()} onClose={() => setShown(undefined)} />}
    </Show>
  );
}

/** Opens the browser sheet for `hostId` (the phone's Panes → Browser). */
export function openPhoneBrowser(hostId: string): void {
  if (!mounted && typeof document !== 'undefined') {
    mounted = true;
    const host = document.createElement('div');
    host.className = 'browser-phone-layer';
    document.body.appendChild(host);
    render(() => <PhoneBrowserLayer />, host);
  }
  setHostName('your PC');
  setShown(hostId);
  void hostNameOf(hostId).then((name) => {
    if (shown() === hostId) setHostName(name);
  });
}

/** Closes it (tests, route changes). */
export function closePhoneBrowser(): void {
  setShown(undefined);
}

/** Is it open? (tests) */
export const phoneBrowserOpen = (): boolean => !!shown();
