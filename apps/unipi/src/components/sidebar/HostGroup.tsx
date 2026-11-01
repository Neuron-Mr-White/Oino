/**
 * One host's section of the sessions list (Z-8, WP-A4): This computer first,
 * then the paired PCs (both shells), each collapsible (state persisted per
 * host). Inside: the Pinned group (Z-14) and the project groups (D-9). An
 * offline host keeps its section with an inline Offline row + Retry (Z-90)
 * instead of disappearing.
 */
import { createEffect, createMemo, createSignal, For, on, Show, type JSX } from 'solid-js';
import { ChevronDown, ChevronRight, Monitor, RefreshCw } from 'lucide-solid';
import type { HostInfo } from '../../lib/schemas';
import { hubs } from '../../lib/session/hub';
import { sessionGroups } from '../../lib/session/selectors';
import { SessionRow, type ModeBadge } from './SessionRow';
import { createPaging, ProjectGroup } from './ProjectGroup';
import { ArchivedList } from './ArchivedList';
import type { RowVariant } from './SessionRow';

const COLLAPSE_KEY = 'unipi.sidebar.hosts';

type Collapsed = Record<string, boolean>;

function readCollapsed(): Collapsed {
  try {
    const v = JSON.parse(localStorage.getItem(COLLAPSE_KEY) ?? '{}') as Collapsed;
    return v && typeof v === 'object' ? v : {};
  } catch {
    return {};
  }
}

/** The persisted collapse state (shared by both shells; `undefined` = default open). */
const [collapsed, setCollapsed] = createSignal<Collapsed>(readCollapsed());
function toggleHost(hostId: string): void {
  setCollapsed((cur) => {
    const next = { ...cur, [hostId]: !cur[hostId] };
    try {
      localStorage.setItem(COLLAPSE_KEY, JSON.stringify(next));
    } catch {
      // not remembered
    }
    return next;
  });
}

/** `true` when the host is collapsed (tests can reset the store). */
export function hostCollapsed(hostId: string): boolean {
  return !!collapsed()[hostId];
}
export function resetHostCollapse(): void {
  setCollapsed({});
  try {
    localStorage.removeItem(COLLAPSE_KEY);
  } catch {
    // ignore
  }
}

export function HostGroup(props: {
  host: HostInfo;
  variant: RowVariant;
  activeSession?: { hostId: string; sessionId: string };
  modeOf: (card: { hostId: string; session_id: string }) => ModeBadge | undefined;
}): JSX.Element {
  const hub = createMemo(() => hubs().ensure(props.host.id));
  const groups = createMemo(() => sessionGroups(hub().state.cards, hub().state.meta));
  const total = () => groups().pinned.length + groups().projects.reduce((n, g) => n + g.cards.length, 0);
  const isCollapsed = () => hostCollapsed(props.host.id);
  const local = () => props.host.transport === 'local';
  const offline = () => hub().state.error !== undefined && !hub().state.loaded;
  const paging = createPaging();

  // Collapsing the host resets the projects' paging (ZCode WSTP-05).
  createEffect(
    on(isCollapsed, (now, was) => {
      if (now !== was) paging.reset();
    }),
  );

  const pin = (card: { session_id: string }, pinned: boolean) => void hub().setMeta(card.session_id, { pinned });

  return (
    <section class="sb-host" data-testid={`sb-host-${props.host.id}`} data-collapsed={isCollapsed() ? '' : undefined}>
      <button type="button" class="sb-host-head" data-testid={`sb-host-head-${props.host.id}`} onClick={() => toggleHost(props.host.id)} aria-expanded={!isCollapsed()}>
        <Show when={isCollapsed()} fallback={<ChevronDown size={14} class="sb-host-chevron" />}>
          <ChevronRight size={14} class="sb-host-chevron" />
        </Show>
        <Show when={local()} fallback={<Monitor size={14} class="sb-host-pc" />}>
          <Monitor size={14} class="sb-host-pc sb-host-pc-local" />
        </Show>
        <span class="sb-host-name">{local() ? 'This computer' : props.host.name}</span>
        <Show when={total() > 0}>
          <span class="sb-host-count" title={`${total()} sessions`}>{total()}</span>
        </Show>
        <Show when={offline()}>
          <span class="sb-host-offline" title={hub().state.error?.message ?? 'Offline'}>Offline</span>
          <span
            role="button"
            tabindex={0}
            class="sb-host-retry"
            title="Retry connecting"
            aria-label="Retry connecting"
            data-testid={`sb-host-retry-${props.host.id}`}
            onClick={(e) => (e.stopPropagation(), void hub().reload())}
            onKeyDown={(e) => e.key === 'Enter' && (e.stopPropagation(), void hub().reload())}
          >
            <RefreshCw size={13} />
          </span>
        </Show>
      </button>
      <Show when={!isCollapsed()}>
        <div class="sb-host-body">
          <Show when={hub().state.loaded && total() === 0 && !offline()}>
            <p class="sb-empty">{hub().state.loading ? 'Loading…' : 'No sessions yet'}</p>
          </Show>
          <Show when={offline()}>
            <p class="sb-empty sb-empty-error" role="alert">{hub().state.error?.message ?? 'This PC is offline'}</p>
          </Show>
          <Show when={groups().pinned.length > 0}>
            <div class="sb-project sb-pinned" data-testid="sb-pinned">
              <div class="sb-project-head" title="Pinned sessions">
                <span class="sb-project-label">Pinned</span>
                <span class="sb-project-count">{groups().pinned.length}</span>
              </div>
              <div class="sb-rows">
                <For each={groups().pinned}>
                  {(card) => (
                    <SessionRow
                      hostId={props.host.id}
                      card={card}
                      meta={hub().state.meta[card.session_id]}
                      active={props.activeSession?.hostId === props.host.id && props.activeSession?.sessionId === card.session_id}
                      variant={props.variant}
                      mode={props.modeOf({ hostId: props.host.id, session_id: card.session_id })}
                      showFolder={props.variant === 'page'}
                      onPin={pin}
                    />
                  )}
                </For>
              </div>
            </div>
          </Show>
          <For each={groups().projects}>
            {(group) => (
              <ProjectGroup
                hostId={props.host.id}
                cwd={group.cwd}
                label={group.label}
                cards={group.cards}
                meta={hub().state.meta}
                variant={props.variant}
                paging={paging}
                modeOf={(card) => props.modeOf({ hostId: props.host.id, session_id: card.session_id })}
                onPin={pin}
                activeOf={(card) => props.activeSession?.hostId === props.host.id && props.activeSession?.sessionId === card.session_id}
              />
            )}
          </For>
          {/* Z-13 (WP-A5): the host's trash — open read-only, restore or delete archived sessions. */}
          <ArchivedList
            hostId={props.host.id}
            variant={props.variant}
            activeSessionId={props.activeSession?.hostId === props.host.id ? props.activeSession.sessionId : undefined}
          />
        </div>
      </Show>
    </section>
  );
}
