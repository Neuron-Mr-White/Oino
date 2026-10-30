/**
 * Session takeover (WP-A9, Z-51; DIRECTION §3.1.5, host.md §8): another
 * runner (the TUI, a `oino -p` job, another host or app) took the session
 * over, so this view keeps its state but goes **read-only** — the composer is
 * disabled and **Take back** reopens the session here (`session_open`, which
 * takes it back from the other runner, host.md §8.1).
 *
 * The host sends `{kind:"taken_over", from}` both when *we* took a session
 * from another runner (a one-line notice, §8.2) and when *we* lost one (§8.4);
 * on the wire the two are indistinguishable without a stream that stays
 * usable for the first case, and `lib/session/stream.ts` (A2) treats every
 * `taken_over` as a loss and stops reconnecting. So one banner carries both
 * halves: the notice sentence ("Taken over from the terminal.") and the
 * read-only state with Take back. A host-side discriminator would split them
 * (see the WP-A9 report).
 */
import { CircleAlert, Undo2 } from 'lucide-solid';
import type { Banner, SessionState } from '../../lib/session/types';

/** `from` on the wire (`tui` / `cli` / `host:<name>`) as the sentence names it. */
export function takeoverSource(from: string | undefined): string {
  if (!from) return 'another window';
  if (from === 'tui') return 'the terminal';
  if (from === 'cli') return 'the CLI';
  const host = /^host:(.+)$/.exec(from);
  return host ? host[1]! : from;
}

/** Pure: this view lost the session to another runner (the composer is read-only). */
export function isTakenOverReadOnly(state: Pick<SessionState, 'banner'>): boolean {
  return state.banner?.kind === 'taken_over';
}

export function ReadOnlyBanner(props: { banner: Banner; onTakeBack: () => void }) {
  return (
    <div class="chat-banner tl-banner tl-readonly" data-kind="taken_over" data-testid="tl-readonly-banner" role="alert">
      <CircleAlert size={18} class="shrink-0" />
      <span class="min-w-0 flex-1">
        Taken over from {takeoverSource(props.banner.from)}. Read-only here — take it back to keep going in this window.
      </span>
      <button type="button" class="chat-banner-action" onClick={() => props.onTakeBack()} data-testid="tl-take-back">
        <Undo2 size={14} /> Take back
      </button>
    </div>
  );
}
