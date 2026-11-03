/**
 * [ux-sessions] One host's sidebar search results (lib/session/sidebarSearch): the host's
 * `sessions_query` pages — live and archived chats, title + first user message, the list's
 * Properties filters — as plain session rows (archived ones marked and opened read-only), a
 * first-message excerpt when only the message matched, "Show more" for the next page.
 */
import { createMemo, For, Show, type JSX } from 'solid-js';
import type { SessionCard } from '../../lib/host/client';
import { hubs } from '../../lib/session/hub';
import { activeFilterCount, listView, modelKey, resetListFilters } from '../../lib/session/listView';
import { createSearchPages, searchArgs, sidebarSearch } from '../../lib/session/sidebarSearch';
import { sessions } from '../../lib/session/store';
import { SessionRow, type RowVariant } from './SessionRow';

export function SearchResults(props: {
  hostId: string;
  variant: RowVariant;
  activeOf: (card: SessionCard) => boolean;
  onPin?: (card: SessionCard, pinned: boolean) => void;
}): JSX.Element {
  const view = () => listView(props.hostId);
  const hub = () => hubs().ensure(props.hostId);
  const pages = createSearchPages(
    () => props.hostId,
    () => (sidebarSearch.query() ? searchArgs(view(), sidebarSearch.query()) : undefined),
  );
  /** The Model facet stays client-side (the host learns a chat's model only once it opens). */
  const shown = createMemo(() => {
    const models = view().filters.models;
    if (!models.length) return pages.hits();
    return pages.hits().filter((h) => {
      const live = sessions().peek(props.hostId, h.card.session_id)?.state.model?.id;
      return models.includes(modelKey(h.card, live ? String(live) : undefined));
    });
  });
  const filters = () => activeFilterCount(view().filters);
  return (
    <div class="sb-results" data-testid={`sb-results-${props.hostId}`} aria-busy={pages.loading()}>
      <Show when={pages.loading() && !pages.hits().length}>
        <div class="sb-skeleton" role="status" aria-label="Searching sessions" data-testid={`sb-results-loading-${props.hostId}`}>
          <For each={[74, 58, 82]}>{(w) => <span class="sb-skel-row" style={{ width: `${w}%` }} />}</For>
        </div>
      </Show>
      <Show when={pages.error()}>
        {(e) => (
          <p class="sb-empty sb-empty-error" role="alert">
            {e().message}
            <button type="button" class="sb-reset-filters" onClick={() => pages.retry()}>
              Retry
            </button>
          </p>
        )}
      </Show>
      <Show when={!pages.loading() && !pages.error()}>
        <Show
          when={shown().length > 0}
          fallback={
            <p class="sb-empty sb-empty-filtered" data-testid={`sb-results-none-${props.hostId}`}>
              No sessions match “{sidebarSearch.query()}”
              <Show when={filters() > 0}>
                <button type="button" class="sb-reset-filters" onClick={() => resetListFilters(props.hostId)}>
                  Reset filters
                </button>
              </Show>
            </p>
          }
        >
          <div class="sb-results-count" data-testid={`sb-results-count-${props.hostId}`}>
            {pages.total()} {pages.total() === 1 ? 'result' : 'results'}
          </div>
        </Show>
      </Show>
      <div class="sb-rows">
        <For each={shown()}>
          {(hit) => (
            <div class="sb-row-slot sb-result" data-archived={hit.archived ? '' : undefined}>
              <SessionRow
                hostId={props.hostId}
                card={hit.archived ? hit.card : (hub().state.cards[hit.card.session_id] ?? hit.card)}
                meta={hit.archived ? undefined : hub().state.meta[hit.card.session_id]}
                active={props.activeOf(hit.card)}
                variant={props.variant}
                show={{ ...view().props, archived: true }}
                compact
                archived={!!hit.archived}
                onPin={hit.archived ? undefined : props.onPin}
              />
              <Show when={hit.snippet}>
                <span class="sb-result-snippet" title={hit.snippet}>
                  {hit.snippet}
                </span>
              </Show>
            </div>
          )}
        </For>
      </div>
      <Show when={pages.hasMore()}>
        <button type="button" class="sb-more" data-testid={`sb-results-more-${props.hostId}`} disabled={pages.loadingMore()} onClick={() => pages.more()}>
          {pages.loadingMore() ? 'Loading…' : 'Show more'}
          <span class="sb-more-count">{Math.min(50, pages.total() - pages.hits().length)}</span>
        </button>
      </Show>
      <Show when={pages.capped() && pages.total() > pages.hits().length}>
        <p class="sb-empty">Showing the first {pages.hits().length} — refine the search to see the rest.</p>
      </Show>
    </div>
  );
}
