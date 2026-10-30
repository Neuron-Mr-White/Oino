/**
 * `header` slot host (WP-U10; ui-protocol §6.2, §3.2 `setHeader` row): a
 * band at the top of the transcript that scrolls away with it — one band
 * per entry in registry order, no card chrome.
 */
import { For, Show } from 'solid-js';
import { EntryView } from '../NodeView';
import type { UiEntry } from '../types';
import type { UiTarget } from '../actions';

export function HeaderBand(props: { entries: UiEntry[]; target: UiTarget }) {
  return (
    <Show when={props.entries.length > 0}>
      <div class="ui-headerband" data-testid="ui-headerband">
        <For each={props.entries}>
          {(entry) => (
            <div class="ui-headerband-entry" data-entry={entry.id}>
              <EntryView entry={entry} target={props.target} mode="auto" />
            </div>
          )}
        </For>
      </div>
    </Show>
  );
}
