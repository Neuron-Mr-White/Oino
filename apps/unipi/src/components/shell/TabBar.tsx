/**
 * The phone's bottom tab bar (§3.9, WP-A29): Chat · Board · Settings.
 * Cron and the extension pages live under "More" in the Chat header menu
 * (screens/Chat.tsx) — the bar keeps the unipi three-tab shell (Z-115).
 * Full-screen layers (conversation `.ws-layer`, Files, panes) paint over
 * it, so it never needs to hide itself.
 *
 * Board is the `unipi.kanboard/board` reserved page (WP-U12): its owner's gate
 * shows/hides the tab live (§8 rule 2 — no "disabled" placeholder).
 */
import { Show } from 'solid-js';
import { A } from '@solidjs/router';
import { Columns3, MessagesSquare, Settings as SettingsIcon } from 'lucide-solid';
import { reservedGate } from '../../ui/slots/reserved';

export function TabBar() {
  return (
    <nav class="bottom-nav" aria-label="Main navigation" data-testid="tab-bar">
      <A href="/chat" data-testid="tab-chat">
        <MessagesSquare size={22} strokeWidth={1.5} />
        <span>Chat</span>
      </A>
      <Show when={reservedGate('unipi.kanboard/board')}>
        <A href="/board" data-testid="tab-board">
          <Columns3 size={22} strokeWidth={1.5} />
          <span>Board</span>
        </A>
      </Show>
      <A href="/settings" data-testid="tab-settings">
        <SettingsIcon size={22} strokeWidth={1.5} />
        <span>Settings</span>
      </A>
    </nav>
  );
}
