/**
 * New-chat empty state (Z-87, WP-A28). Hybrid of ZCode and unipi (audit K):
 * the Unicrab (not ZCode's logo) over a time-of-day greeting, one short line
 * saying what the app can do, and a muted lore line — **no recents** (the
 * sessions sidebar is the one place to resume something, Z-19).
 *
 * The greeting comes from the *device's* local hour (`lib/greeting.ts`, ZCode
 * `chat-empty-time-greeting.md`); the title never carries the workspace name.
 */
import { Show } from 'solid-js';
import { Unicrab } from './Unicrab';
import { greetingFor } from '../lib/greeting';
import { loreLine } from '../lib/lore';
import { shortCwd } from '../lib/session/selectors';

export function EmptyChat(props: {
  /** The draft's folder (draft route) or the session's cwd. */
  cwd?: string;
  /** The PC's name for a paired host ("coffee"); omitted for This computer. */
  hostName?: string;
  /** Tighter type scale inside the phone's composer-led draft. */
  compact?: boolean;
  /** The new-chat start screen: greeting + one line, no crab (the draft draws
   *  a faint watermark behind instead) and no lore. */
  variant?: 'start';
  testId?: string;
}) {
  const where = () => (props.hostName ? `on ${props.hostName}` : 'on this PC');
  const hint = () =>
    props.variant === 'start' && !props.cwd
      ? `Ask anything — UniPi reads, edits and runs code ${where()}.`
      : `Ask for a change in ${props.cwd ? shortCwd(props.cwd) : 'your project'} — UniPi reads, edits and runs it ${where()}.`;
  return (
    <section
      class="empty-chat"
      data-compact={props.compact ? '' : undefined}
      data-variant={props.variant}
      data-testid={props.testId ?? 'empty-chat'}
      aria-labelledby="empty-chat-title"
    >
      <Show when={props.variant !== 'start'}>
        <Unicrab state="idle" size={112} class="empty-chat-crab" title="Unicrab" />
      </Show>
      <h2 id="empty-chat-title" class="empty-chat-greeting" data-testid="empty-chat-greeting">
        {greetingFor(new Date())}
      </h2>
      <p class="empty-chat-hint">{hint()}</p>
      <Show when={props.variant !== 'start' && loreLine()}>
        {(line) => <p class="empty-chat-lore">{line()}</p>}
      </Show>
    </section>
  );
}
