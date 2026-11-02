/**
 * The "needs you" bell (UNI-161 §3): owns the drawer's open/close state. The
 * items come from the needs-you store (fed by host hub events, WP-A19; the v1
 * `chat_list` poll is gone with herdr, D-14). On the phone it's the floating Unicrab
 * (UNI-200, FloatingMascot: draggable, snaps to a side, chat bubbles); on
 * the desktop shell the crab sits inside the page header's tools
 * (PageHeader, UNI-218 F9), so only the drawer is here.
 *
 * It also feeds the mascot's state + bubbles to the Android overlay
 * (MascotOverlayRunner) and the desktop mascot window.
 */
import { createSignal, Show } from 'solid-js';
import { NeedsYouDrawer } from './NeedsYouBell';
import { FloatingMascot } from './FloatingMascot';
import { layout } from '../lib/layout';
import { floatingOn, setFloatingOn } from '../lib/mascot';
import { sheetOpen, useKeyboard } from './phone/services';
import './needsYou.css';

import { needsYouOpen, setNeedsYouOpen } from '../lib/needsYouOpen';
export { needsYouOpen, setNeedsYouOpen };
/**
 * The phone's floating crab on/off (Settings → Unicrab, and its own "Hide" quick action).
 * Phone spec §3.13 (PH-12): off unless the user turned it on — needs-you lives in the Chats
 * header + the Chats tab badge. Only an explicit stored "on" (set via Settings) shows it.
 */
function explicitlyOn(): boolean {
  try {
    return localStorage.getItem('unipi.mascot.floating') === 'on' && floatingOn();
  } catch {
    return false;
  }
}
const [floating, setFloatingSignal] = createSignal(explicitlyOn());
export const floatingMascotOn = floating;
export function setFloatingMascotOn(on: boolean): void {
  setFloatingOn(on);
  setFloatingSignal(on);
}

export function FloatingBell() {
  const keyboard = useKeyboard();
  return (
    <>
      <Show when={!layout().desktop() && floating() && !needsYouOpen() && !sheetOpen() && !keyboard.open()}>
        <FloatingMascot onOpenDrawer={() => setNeedsYouOpen(true)} onHide={() => setFloatingMascotOn(false)} />
      </Show>
      <NeedsYouDrawer open={needsYouOpen()} onClose={() => setNeedsYouOpen(false)} />
    </>
  );
}
