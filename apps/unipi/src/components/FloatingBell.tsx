/**
 * The "needs you" bell (UNI-161 §3): starts the cross-host poll
 * (lib/needsYouPoll.ts) once, on app open/resume + while visible, and owns
 * the drawer's open/close state. On the phone it's the floating Unicrab
 * (UNI-200, FloatingMascot: draggable, snaps to a side, chat bubbles); on
 * the desktop shell the crab sits inside the page header's tools
 * (PageHeader, UNI-218 F9), so only the drawer is here.
 *
 * It also feeds the mascot's state + bubbles to the Android overlay
 * (MascotOverlayRunner) and the desktop mascot window.
 */
import { createEffect, createSignal, on, onCleanup, onMount, Show } from 'solid-js';
import { NeedsYouDrawer } from './NeedsYouBell';
import { FloatingMascot } from './FloatingMascot';
import { useHosts } from '../hosts/context';
import { useSettings } from '../settings/context';
import { pollNeedsYou, startNeedsYouPoll } from '../lib/needsYouPoll';
import { layout } from '../lib/layout';
import { floatingOn, setFloatingOn } from '../lib/mascot';
import './needsYou.css';

const [needsYouOpen, setNeedsYouOpen] = createSignal(false);
export { needsYouOpen, setNeedsYouOpen };
/** The phone's floating crab on/off (Settings → Unicrab, and its own "Hide" quick action). */
const [floating, setFloatingSignal] = createSignal(floatingOn());
export const floatingMascotOn = floating;
export function setFloatingMascotOn(on: boolean): void {
  setFloatingOn(on);
  setFloatingSignal(on);
}

export function FloatingBell() {
  const hosts = useHosts();
  const [settings] = useSettings();
  onMount(() => {
    const stop = startNeedsYouPoll(() => hosts.hosts(), 20_000, () => settings.notify);
    onCleanup(stop);
  });
  // The host list loads asynchronously (list_hosts): poll again the moment
  // it's ready/changes, instead of waiting for the next tick.
  createEffect(on(hosts.hosts, (list) => {
    if (list.length) void pollNeedsYou(list);
  }));
  return (
    <>
      <Show when={!layout().desktop() && floating() && !needsYouOpen()}>
        <FloatingMascot onOpenDrawer={() => setNeedsYouOpen(true)} onHide={() => setFloatingMascotOn(false)} />
      </Show>
      <NeedsYouDrawer open={needsYouOpen()} onClose={() => setNeedsYouOpen(false)} />
    </>
  );
}
