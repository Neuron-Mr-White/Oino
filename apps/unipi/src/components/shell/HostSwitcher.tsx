/**
 * The global host selector (owner feedback, [ui-shell] #1): one "which PC"
 * control for the whole app — `hosts.selectedId()` — that Board, Cron,
 * Preview and New chat follow. Desktop: a profile-style row at the bottom of
 * the sidebar (PC icon · host name · status dot · chevron) with the Settings
 * gear beside it. Phone: the same menu behind a compact chip at the top of
 * the Chat tab. The menu lists every paired PC (connected / offline), then
 * "Add a PC…" and "Manage hosts" (Settings › Hosts & devices).
 */
import { createEffect, createSignal, For, on, Show } from 'solid-js';
import { useLocation, useNavigate } from '@solidjs/router';
import { Check, ChevronsUpDown, ChevronDown, Monitor, Plus, Settings2 } from 'lucide-solid';
import { useHostsOptional } from '../../hosts/context';
import { hubs } from '../../lib/session/hub';
import { desktopShell } from '../../lib/layout';
import type { HostInfo } from '../../lib/schemas';
import { AnchoredMenu, MenuSeparator } from '../board/AnchoredMenu';
import { setAddHostOpen } from '../Pairing';
import '../board/anchored-menu.css';

/** `/s/:host/…` or `/new/:host` → that host (the selection follows the shown chat). */
function routeHost(pathname: string): string | undefined {
  const m = /^\/(?:s|new)\/([^/?#]+)/.exec(pathname);
  return m ? decodeURIComponent(m[1]!) : undefined;
}

/** Opening a chat or a draft on a PC selects that PC (mounted once in AppShell). */
export function FollowRouteHost() {
  const hosts = useHostsOptional();
  const location = useLocation();
  createEffect(
    on([() => routeHost(location.pathname), () => hosts?.hosts()], ([id, list]) => {
      if (id && list?.some((h) => h.id === id) && hosts?.selectedId() !== id) hosts?.selectHost(id);
    }),
  );
  return null;
}

export type HostStatus = 'ok' | 'checking' | 'offline';

/** A host's connection, as the sidebar's session list sees it (its hub). */
export function hostStatus(id: string): HostStatus {
  const state = hubs().ensure(id).state;
  if (state.loaded && state.error === undefined) return 'ok';
  if (state.error !== undefined && !state.loaded) return 'offline';
  return state.loaded ? 'ok' : 'checking';
}

const statusWord = (s: HostStatus) => (s === 'ok' ? 'Connected' : s === 'offline' ? 'Offline' : 'Connecting…');

/** The selected host (falls back to This computer, then the first PC). */
export function useSelectedHost(): () => HostInfo | undefined {
  const hosts = useHostsOptional();
  return () => {
    const all = hosts?.hosts() ?? [];
    const id = hosts?.selectedId();
    return all.find((h) => h.id === id) ?? all.find((h) => h.transport === 'local') ?? all[0];
  };
}

function HostMenu(props: { anchor: DOMRect; onDismiss: () => void; alignEnd?: boolean }) {
  const hosts = useHostsOptional();
  const navigate = useNavigate();
  const location = useLocation();
  const selected = useSelectedHost();
  const pick = (id: string) => {
    hosts?.selectHost(id);
    props.onDismiss();
    // A draft is "a new chat on <PC>": switching the PC moves the draft with it.
    const draft = /^\/new\/([^/?#]+)/.exec(location.pathname)?.[1];
    if (draft !== undefined && decodeURIComponent(draft) !== id) navigate(`/new/${encodeURIComponent(id)}`);
  };
  return (
    <AnchoredMenu anchor={props.anchor} onDismiss={props.onDismiss} label="Choose a PC" width={desktopShell() ? Math.max(232, Math.round(props.anchor.width)) : 288} alignEnd={props.alignEnd} class="hs-menu">
      <div class="board-menu-label">Computers</div>
      <For each={hosts?.hosts() ?? []}>
        {(host) => {
          const status = () => hostStatus(host.id);
          const current = () => selected()?.id === host.id;
          return (
            <button
              type="button"
              role="menuitemradio"
              aria-checked={current()}
              class="recent-popover-row board-menu-row hs-menu-row"
              data-testid={`hs-host-${host.id}`}
              onClick={() => pick(host.id)}
            >
              <span class="board-menu-icon"><Monitor size={16} /></span>
              <span class="hs-menu-text">
                <span class="hs-menu-name">{host.name}</span>
                <span class="hs-menu-sub" data-state={status()}>
                  <span class="hs-dot" data-state={status()} />
                  {host.transport === 'local' ? `This computer · ${statusWord(status())}` : statusWord(status())}
                </span>
              </span>
              <Show when={current()}>
                <Check size={16} class="hs-menu-check" />
              </Show>
            </button>
          );
        }}
      </For>
      <MenuSeparator />
      <button type="button" role="menuitem" class="recent-popover-row board-menu-row" data-testid="hs-add" onClick={() => (props.onDismiss(), setAddHostOpen(true))}>
        <span class="board-menu-icon"><Plus size={16} /></span>
        <span class="min-w-0 flex-1 truncate text-left">Add a PC…</span>
      </button>
      <button type="button" role="menuitem" class="recent-popover-row board-menu-row" data-testid="hs-manage" onClick={() => (props.onDismiss(), navigate('/settings/hosts'))}>
        <span class="board-menu-icon"><Settings2 size={16} /></span>
        <span class="min-w-0 flex-1 truncate text-left">Manage hosts</span>
      </button>
    </AnchoredMenu>
  );
}

/** The opener + menu state shared by the row and the chip. */
function useHostMenu() {
  const [anchor, setAnchor] = createSignal<DOMRect | undefined>();
  const open = (e: MouseEvent) => setAnchor((e.currentTarget as HTMLElement).getBoundingClientRect());
  return { anchor, open, close: () => setAnchor(undefined) };
}

/** Desktop: the sidebar's bottom "profile" row. */
export function HostRow() {
  const selected = useSelectedHost();
  const menu = useHostMenu();
  const status = () => (selected() ? hostStatus(selected()!.id) : 'checking');
  return (
    <>
      <button
        type="button"
        class="hs-row"
        aria-haspopup="menu"
        aria-expanded={menu.anchor() !== undefined}
        aria-label={`Computer: ${selected()?.name ?? 'none'} (${statusWord(status())}). Choose a PC`}
        title="Choose a PC"
        data-testid="host-row"
        onClick={menu.open}
      >
        <span class="hs-row-icon" data-local={selected()?.transport === 'local' ? '' : undefined}>
          <Monitor size={16} />
        </span>
        <span class="hs-row-name">{selected()?.name ?? 'Choose a PC'}</span>
        <span class="hs-dot" data-state={status()} title={statusWord(status())} />
        <ChevronsUpDown size={14} class="hs-row-chevron" />
      </button>
      <Show when={menu.anchor()} keyed>
        {(anchor) => <HostMenu anchor={anchor} onDismiss={menu.close} />}
      </Show>
    </>
  );
}

/** Phone: the same selector as a compact chip (top of the Chat tab, Board, Cron). */
export function HostChip() {
  const selected = useSelectedHost();
  const menu = useHostMenu();
  const status = () => (selected() ? hostStatus(selected()!.id) : 'checking');
  return (
    <>
      <button
        type="button"
        class="pc-chip hs-chip"
        aria-haspopup="menu"
        aria-label={`Computer: ${selected()?.name ?? 'none'} (${statusWord(status())}). Choose a PC`}
        data-testid="host-chip"
        onClick={menu.open}
      >
        <span class="hs-dot" data-state={status()} />
        <span class="truncate">{selected()?.name ?? 'Choose a PC'}</span>
        <ChevronDown size={14} />
      </button>
      <Show when={menu.anchor()} keyed>
        {(anchor) => <HostMenu anchor={anchor} onDismiss={menu.close} />}
      </Show>
    </>
  );
}
