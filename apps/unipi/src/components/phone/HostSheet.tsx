/**
 * The PC chip and its "Your PCs" sheet (phone spec §1.4), plus the offline
 * banner every tab page shows while the selected PC is unreachable (PH-15).
 */
import { createSignal, For, Show } from 'solid-js';
import { useLocation, useNavigate } from '@solidjs/router';
import { Check, ChevronDown, Monitor, Plus, Settings2, WifiOff } from 'lucide-solid';
import { useHostsOptional } from '../../hosts/context';
import { hubs } from '../../lib/session/hub';
import { hostStatus, useSelectedHost, type HostStatus } from '../shell/HostSwitcher';
import { setAddHostOpen } from '../Pairing';
import { Sheet } from './Sheet';
import { ListGroup, ListRow } from './lists';
import { haptic } from './services';

const statusWord = (s: HostStatus) => (s === 'ok' ? 'Connected' : s === 'offline' ? 'Offline' : 'Connecting…');

/** The PC chip (status dot · name · chevron; 36 visual / 48 hit) + the Your PCs sheet. */
export function HostSheetButton() {
  const hosts = useHostsOptional();
  const selected = useSelectedHost();
  const navigate = useNavigate();
  const location = useLocation();
  const [open, setOpen] = createSignal(false);
  const status = (): HostStatus => (selected() ? hostStatus(selected()!.id) : 'checking');
  const none = () => !hosts?.loading() && (hosts?.hosts().length ?? 0) === 0;
  const pick = (id: string) => {
    if (hosts?.selectedId() !== id) haptic('selection');
    hosts?.selectHost(id);
    setOpen(false);
    const draft = /^\/new\/([^/?#]+)/.exec(location.pathname)?.[1];
    if (draft !== undefined && decodeURIComponent(draft) !== id) navigate(`/new/${encodeURIComponent(id)}`, { replace: true });
  };
  return (
    <>
      <button
        type="button"
        class="pc-chip hs-chip ph-pc-chip"
        aria-haspopup="dialog"
        aria-label={none() ? 'Add a PC' : `Computer: ${selected()?.name ?? 'none'} (${statusWord(status())}). Choose a PC`}
        data-testid="host-chip"
        onClick={() => (none() ? setAddHostOpen(true) : setOpen(true))}
      >
        <Show when={!none()} fallback={<Plus size={16} />}>
          <span class="hs-dot" data-state={status()} />
        </Show>
        <span class="truncate">{none() ? 'Add a PC' : (selected()?.name ?? 'Choose a PC')}</span>
        <Show when={!none()}>
          <ChevronDown size={16} />
        </Show>
      </button>
      <Sheet open={open()} onClose={() => setOpen(false)} label="Your PCs" title="Your PCs" testId="host-sheet">
        <ListGroup inset>
          <For each={hosts?.hosts() ?? []}>
            {(host) => {
              const s = () => hostStatus(host.id);
              const current = () => selected()?.id === host.id;
              return (
                <ListRow
                  testId={`hs-host-${host.id}`}
                  leading={<span class="ph-host-lead" data-state={s()}><Monitor size={22} /><span class="hs-dot" data-state={s()} /></span>}
                  title={host.name}
                  subtitle={host.transport === 'local' ? `This computer · ${statusWord(s())}` : statusWord(s())}
                  trailing={current() ? <Check size={22} class="ph-check" aria-label="Selected" /> : undefined}
                  selected={current()}
                  onClick={() => pick(host.id)}
                />
              );
            }}
          </For>
        </ListGroup>
        <ListGroup inset>
          <ListRow testId="hs-add" leading={<Plus size={22} />} title="Add a PC" onClick={() => (setOpen(false), setAddHostOpen(true))} />
          <ListRow testId="hs-manage" leading={<Settings2 size={22} />} title="Manage PCs" chevron onClick={() => (setOpen(false), navigate('/settings/hosts'))} />
        </ListGroup>
      </Sheet>
    </>
  );
}

/** Reconnect a host now (re-runs its session list; the hub's own retry loop keeps going). */
export function retryHost(hostId: string): void {
  void hubs().ensure(hostId).reload().catch(() => undefined);
}

/** "<PC> is offline · Retry" — a 48 px row under the header (§1.4, PH-15). */
export function OfflineBanner(p: { hostId: string }) {
  const hosts = useHostsOptional();
  const name = () => hosts?.hosts().find((h) => h.id === p.hostId)?.name ?? 'Your PC';
  const [busy, setBusy] = createSignal(false);
  return (
    <Show when={p.hostId && hostStatus(p.hostId) === 'offline'}>
      <div class="ph-offline" role="status" data-testid="offline-banner">
        <WifiOff size={18} aria-hidden="true" />
        <span class="ph-offline-text">{name()} is offline</span>
        <button
          type="button"
          class="ph-offline-retry"
          disabled={busy()}
          onClick={() => {
            setBusy(true);
            retryHost(p.hostId);
            setTimeout(() => setBusy(false), 1500);
          }}
        >
          {busy() ? 'Retrying…' : 'Retry'}
        </button>
      </div>
    </Show>
  );
}
