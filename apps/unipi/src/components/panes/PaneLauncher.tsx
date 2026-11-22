/**
 * The `+` launcher (Z-5, WP-A3): the tab kinds that have a view (Files, Git,
 * Tree, Side chat, …; terminal and browser once wave 3 lands; extension
 * panels the user can open) and — side pane only — **Local ports** (Z-104):
 * the host's listening ports, which open the forward viewer as a tab (the
 * browser tab takes over in wave 3). Also rendered inline as the content of
 * an empty pane.
 */
import { createResource, For, Match, Show, Switch } from 'solid-js';
import { Plug, RefreshCw } from 'lucide-solid';
import type { ListeningPort } from '@unipi/protocol';
import { canForward, pickerState, portLabel, portsList } from '../../lib/forward';
import { launchableKinds } from '../../lib/panes/kinds';
import type { PaneArea } from '../../lib/panes';
import { AnchoredMenu } from '../board/AnchoredMenu';

export type LaunchSpec = { kind: string; params?: Record<string, unknown>; title?: string };

export const PORTS_MAX = 8;

/** Pure: the ports the launcher lists (web servers first, then the rest), at most `PORTS_MAX`. */
export function launcherPorts(list: ListeningPort[]): ListeningPort[] {
  const state = pickerState({ ok: true, value: { ports: list, any: true, allowed: [] } });
  return state.kind === 'ports' ? [...state.web, ...state.other].slice(0, PORTS_MAX) : [];
}

function LocalPorts(props: { hostId: string; local: boolean; onPick: (spec: LaunchSpec) => void }) {
  const [ports, { refetch }] = createResource(() => props.hostId, (id) => portsList(id));
  const state = () => pickerState(ports());
  return (
    <div class="launcher-group" data-testid="launcher-ports">
      <div class="launcher-label">
        <span>Local ports</span>
        <button type="button" class="launcher-refresh" aria-label="Refresh ports" onClick={() => void refetch()}>
          <RefreshCw size={13} class={ports.loading ? 'refresh-spinning' : ''} />
        </button>
      </div>
      <Switch>
        <Match when={state().kind === 'loading'}>
          <p class="launcher-note">Looking for web apps…</p>
        </Match>
        <Match when={state().kind === 'disabled' || state().kind === 'error'}>
          <p class="launcher-note">{(state() as { error: { message: string } }).error.message}</p>
        </Match>
        <Match when={state().kind === 'empty'}>
          <p class="launcher-note">No web apps running on this PC.</p>
        </Match>
        <Match when={state().kind === 'ports'}>
          <For each={launcherPorts(ports()!.ok ? (ports() as { ok: true; value: { ports: ListeningPort[] } }).value.ports : [])}>
            {(p) => (
              <button
                type="button"
                role="menuitem"
                class="launcher-row"
                data-testid={`launcher-port-${p.port}`}
                onClick={() => props.onPick({ kind: 'forward', params: { hostId: props.hostId, port: p.port, local: props.local } })}
              >
                <Plug size={16} class="launcher-icon" />
                <span class="launcher-row-label">localhost:{p.port}</span>
                <span class="launcher-row-hint">{portLabel(p)}</span>
              </button>
            )}
          </For>
        </Match>
      </Switch>
    </div>
  );
}

/** The launcher entries (inline list). */
export function LauncherList(props: { area: PaneArea; hasSession: boolean; hostId?: string; localHost: boolean; inline?: boolean; emptyText?: string; onPick: (spec: LaunchSpec) => void }) {
  const kinds = () => launchableKinds(props.area, props.hasSession);
  return (
    <div class="launcher" role={props.inline ? 'menu' : undefined} aria-label={props.inline ? 'Open a tab' : undefined} data-inline={props.inline ? '' : undefined} data-testid="pane-launcher">
      <Show when={kinds().length > 0}>
        <div class="launcher-group">
          <For each={kinds()}>
            {(k) => {
              const Icon = k.icon;
              return (
                <button type="button" role="menuitem" class="launcher-row" data-testid={`launcher-${k.kind}`} onClick={() => props.onPick({ kind: k.kind })}>
                  <Icon size={16} class="launcher-icon" />
                  <span class="launcher-row-label">{k.launcher!.label}</span>
                </button>
              );
            }}
          </For>
        </div>
      </Show>
      <Show when={props.area === 'side' && props.hostId && canForward()}>
        <LocalPorts hostId={props.hostId!} local={props.localHost} onPick={props.onPick} />
      </Show>
      <Show when={kinds().length === 0 && !(props.area === 'side' && props.hostId && canForward())}>
        <p class="launcher-note">{props.emptyText ?? 'Nothing to open here yet.'}</p>
      </Show>
    </div>
  );
}

/** The `+` popover. */
export function PaneLauncher(props: { anchor: DOMRect; area: PaneArea; hasSession: boolean; hostId?: string; localHost: boolean; onPick: (spec: LaunchSpec) => void; onDismiss: () => void }) {
  return (
    <AnchoredMenu anchor={props.anchor} width={280} label="Open a tab" onDismiss={props.onDismiss}>
      <LauncherList area={props.area} hasSession={props.hasSession} hostId={props.hostId} localHost={props.localHost} onPick={(spec) => (props.onDismiss(), props.onPick(spec))} />
    </AnchoredMenu>
  );
}
