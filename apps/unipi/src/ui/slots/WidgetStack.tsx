/**
 * `widget` slot host (WP-U10, G-3; ui-protocol §6.2, §3.2 `setWidget` row):
 * the stack above the composer (and `widgetBelow` below it, desktop only) —
 * each entry card-less with a 1 px rule between, `lines` views in the
 * monospace mini style and `tree` views through `EntryView`.
 *
 * Phone (§13.6 row 6): `widgetBelow` folds into this stack, and beyond 3
 * entries only the first 3 show — the rest sit behind a "+N widgets" chip
 * (§6.4 per-slot cap).
 */
import { createSignal, For, Show } from 'solid-js';
import { ChevronDown, ChevronUp } from 'lucide-solid';
import { EntryView } from '../NodeView';
import { liveTrayEntries } from './trayHint';
import type { UiEntry } from '../types';
import type { UiTarget } from '../actions';

const PHONE_MAX = 3;

export function WidgetStack(props: { entries: UiEntry[]; target: UiTarget; phone?: boolean }) {
  const entries = () => liveTrayEntries(props.entries);
  const over = () => (props.phone ? Math.max(0, entries().length - PHONE_MAX) : 0);
  const [expanded, setExpanded] = createSignal(false);
  const shown = () => (over() && !expanded() ? entries().slice(0, PHONE_MAX) : entries());
  return (
    <Show when={entries().length > 0}>
      <div class="ui-widgets" data-count={entries().length} data-testid="ui-widgets">
        <For each={shown()}>
          {(entry) => (
            <div class="ui-widget" data-entry={entry.id}>
              <Show when={entry.title}>
                <p class="ui-widget-title">{entry.title}</p>
              </Show>
              <EntryView entry={entry} target={props.target} mode="auto" />
            </div>
          )}
        </For>
        <Show when={over() > 0}>
          <button type="button" class="ui-widgets-more" aria-expanded={expanded()} onClick={() => setExpanded((v) => !v)} data-testid="ui-widgets-more">
            {expanded() ? <ChevronUp size={13} /> : <ChevronDown size={13} />}
            {expanded() ? `Hide ${over()} widgets` : `+${over()} widgets`}
          </button>
        </Show>
      </div>
    </Show>
  );
}
