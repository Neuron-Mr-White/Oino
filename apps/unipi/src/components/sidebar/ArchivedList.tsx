/**
 * The Archived list (Z-13, WP-A5): one collapsible "Archived" footer per host
 * group in the sessions list. Opening it lazily asks the host for its trashed
 * sessions (`sessions_list{archived:true}`, §3.14) and offers:
 *
 * - Restore — `session_restore` puts the file back and the row reappears live;
 * - Delete — `session_delete` unlinks that one file (a confirm step first);
 * - Delete all — `trash_empty` empties the host's trash (confirm first).
 *
 * The same component serves the phone Chat tab (`variant="page"`).
 */
import { createMemo, createSignal, For, Show } from 'solid-js';
import { ArchiveRestore, ChevronDown, ChevronRight, Trash2 } from 'lucide-solid';
import type { SessionCard } from '../../lib/host/client';
import { deleteArchived, emptyTrash, restoreSession, sessionHref } from '../../lib/session/actions';
import { hubs } from '../../lib/session/hub';
import { sessionTitle } from '../../lib/session/selectors';
import { shortAge } from '../../lib/time';
import { observeTitleOverflow, type RowVariant } from './SessionRow';

const titleOf = (card: SessionCard): string => sessionTitle(card, undefined, card.session_id);

/** Archived rows shown before "Show more" (a trash of hundreds of chats). */
export const ARCHIVED_PAGE = 30;

export function ArchivedList(props: { hostId: string; variant: RowVariant; activeSessionId?: string }) {
  const hub = () => hubs().ensure(props.hostId);
  const state = hub().state;
  const [open, setOpen] = createSignal(false);
  /** The confirm step of "Delete all" (Z-13's delete-all is destructive). */
  const [confirmAll, setConfirmAll] = createSignal(false);
  /** Which row's Delete is confirming. */
  const [confirmId, setConfirmId] = createSignal<string | undefined>();
  const [limit, setLimit] = createSignal(ARCHIVED_PAGE);
  /** The first `limit` rows, plus the shown chat wherever it sorts. */
  const shown = createMemo(() => {
    const all = state.archived;
    if (all.length <= limit()) return all;
    const head = all.slice(0, limit());
    const active = props.activeSessionId ? all.slice(limit()).find((c) => c.session_id === props.activeSessionId) : undefined;
    return active ? [...head, active] : head;
  });
  const hidden = () => Math.max(0, state.archived.length - limit());

  const toggle = () => {
    const next = !open();
    setOpen(next);
    if (next) void hub().loadArchived();
  };

  return (
    <div class="sb-archived" data-variant={props.variant} data-testid={`sb-archived-${props.hostId}`}>
      <button
        type="button"
        class="sb-archived-head"
        aria-expanded={open()}
        title="Sessions moved to this PC's trash — open, restore or delete them"
        data-testid={`sb-archived-head-${props.hostId}`}
        onClick={toggle}
      >
        <Show when={open()} fallback={<ChevronRight size={14} class="sb-host-chevron" />}>
          <ChevronDown size={14} class="sb-host-chevron" />
        </Show>
        <ArchiveRestore size={14} />
        <span class="sb-archived-label">Archived</span>
        <Show when={state.archivedLoaded && state.archived.length > 0}>
          <span class="sb-host-count" title={`${state.archived.length} archived`}>{state.archived.length}</span>
        </Show>
      </button>
      <Show when={open()}>
        <div class="sb-archived-body" data-testid={`sb-archived-body-${props.hostId}`}>
          <Show when={state.archivedLoading && !state.archivedLoaded}>
            <div class="sb-skeleton" role="status" aria-label="Loading archived sessions" data-testid={`sb-archived-skeleton-${props.hostId}`}>
              <For each={[70, 82, 60]}>{(w) => <span class="sb-skel-row" style={{ width: `${w}%` }} />}</For>
            </div>
          </Show>
          <Show when={state.archivedLoaded && state.archived.length === 0 && !state.archivedLoading}>
            <p class="sb-empty">Nothing archived</p>
          </Show>
          <Show when={!state.archivedLoaded ? state.error : undefined} keyed>
            {(error) => <p class="sb-empty sb-empty-error" role="alert">{error.message}</p>}
          </Show>
          <For each={shown()}>
            {(card) => {
              const title = () => titleOf(card);
              const confirming = () => confirmId() === card.session_id;
              const active = () => props.activeSessionId === card.session_id;
              return (
                <div
                  class="sb-archived-row"
                  data-confirm={confirming() ? '' : undefined}
                  aria-current={active() ? 'page' : undefined}
                  data-testid={`sb-archived-row-${card.session_id}`}
                >
                  <a
                    href={`#${sessionHref(props.hostId, card.session_id)}`}
                    class="sb-archived-open"
                    title={title()}
                    aria-current={active() ? 'page' : undefined}
                    data-testid={`sb-archived-open-${card.session_id}`}
                  >
                    <span ref={observeTitleOverflow} class="srow-title sb-archived-title">
                      {title()}
                    </span>
                  </a>
                  <span class="srow-time sb-archived-time">{shortAge(card.last_activity ?? card.updated_at)}</span>
                  <div class="sb-archived-actions">
                    <Show
                      when={!confirming()}
                      fallback={
                        <>
                          <button
                            type="button"
                            class="sb-archived-cancel"
                            onClick={() => setConfirmId(undefined)}
                          >
                            Cancel
                          </button>
                          <button
                            type="button"
                            class="sb-archived-del sb-archived-del-confirm text-wm-red"
                            title="Confirm delete"
                            onClick={() => {
                              setConfirmId(undefined);
                              void deleteArchived(props.hostId, card.session_id);
                            }}
                          >
                            Delete
                          </button>
                        </>
                      }
                    >
                      <button
                        type="button"
                        class="sb-archived-restore"
                        aria-label={`Restore ${title()}`}
                        title="Restore session"
                        onClick={() => void restoreSession(props.hostId, card.session_id)}
                      >
                        <ArchiveRestore size={13} />
                        <Show when={props.variant === 'page'}>
                          <span>Restore</span>
                        </Show>
                      </button>
                      <button
                        type="button"
                        class="sb-archived-del"
                        aria-label="Delete archived session"
                        title="Delete archived session"
                        onClick={() => setConfirmId(card.session_id)}
                      >
                        <Trash2 size={13} />
                      </button>
                    </Show>
                  </div>
                </div>
              );
            }}
          </For>
          <Show when={hidden() > 0}>
            <button
              type="button"
              class="sb-more"
              data-testid={`sb-archived-more-${props.hostId}`}
              title={`${hidden()} more archived`}
              onClick={() => setLimit((n) => n + ARCHIVED_PAGE)}
            >
              Show more
              <span class="sb-more-count">{Math.min(hidden(), ARCHIVED_PAGE)}</span>
            </button>
          </Show>
          <Show when={state.archivedLoaded && state.archived.length > 0}>
            <Show
              when={!confirmAll()}
              fallback={
                <button
                  type="button"
                  class="sb-archived-empty text-wm-red"
                  data-testid="sb-archived-empty-confirm"
                  onClick={() => (setConfirmAll(false), void emptyTrash(props.hostId))}
                >
                  Delete all {state.archived.length} archived?
                </button>
              }
            >
              <button type="button" class="sb-archived-empty" data-testid="sb-archived-empty" onClick={() => setConfirmAll(true)}>
                <Trash2 size={13} />
                Delete all
              </button>
            </Show>
          </Show>
        </div>
      </Show>
    </div>
  );
}
