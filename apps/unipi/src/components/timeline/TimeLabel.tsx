/**
 * The turn tail's time (WP-A7, Z-26): `HH:MM` today, `Yesterday HH:MM`,
 * `M/D, HH:MM` this year, `Y/M/D, HH:MM` older — rendered as a `<time>` with
 * the exact ISO instant in `datetime`/`title` for hover. Nothing renders when
 * the session entry has no timestamp.
 */
import { createMemo, Show } from 'solid-js';
import { defaultLocale, formatMessageTimeLabel } from '../../lib/timeline/timeLabel';

export function TimeLabel(props: { at?: number; locale?: string }) {
  const label = createMemo(() => formatMessageTimeLabel(props.at, props.locale ?? defaultLocale()));
  const iso = () => (props.at ? new Date(props.at).toISOString() : undefined);
  return (
    <Show when={label()}>
      {(text) => (
        <time class="tl-time" datetime={iso()} title={iso()} data-testid="tl-time">
          {text()}
        </time>
      )}
    </Show>
  );
}
