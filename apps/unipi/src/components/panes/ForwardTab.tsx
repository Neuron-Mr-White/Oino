/**
 * `forward` pane tab (Z-104, WP-A3): a host port opened from the `+`
 * launcher's "Local ports". This computer's ports load straight from
 * 127.0.0.1; a paired PC's port goes through a loopback forward
 * (`forward_start`, reused when one is already running). The browser tab
 * (wave 3, B8) takes over this entry point.
 */
import { createResource, Match, Switch } from 'solid-js';
import { Plug } from 'lucide-solid';
import type { AppError } from '@unipi/protocol';
import { forwardList, forwardStart, forwardStop, type ForwardInfo } from '../../lib/forward';
import { useHostsOptional } from '../../hosts/context';
import { setPaneComponent, type PaneTabProps } from '../../lib/panes/kinds';
import { ForwardViewer } from '../../screens/Preview';
import { EmptyState } from '../EmptyState';

type Loaded = { ok: true; info: ForwardInfo } | { ok: false; error: AppError };

export function localForward(hostId: string, port: number): ForwardInfo {
  return { id: `local-${port}`, hostId, remotePort: port, localPort: port, url: `http://127.0.0.1:${port}/`, startedAt: Date.now(), active: 0, total: 0 } as ForwardInfo;
}

async function startForward(hostId: string, port: number, local: boolean): Promise<Loaded> {
  if (local) return { ok: true, info: localForward(hostId, port) };
  const running = await forwardList();
  const hit = running.ok ? running.value.find((f) => f.hostId === hostId && f.remotePort === port) : undefined;
  if (hit) return { ok: true, info: hit };
  const started = await forwardStart(hostId, port);
  return started.ok ? { ok: true, info: started.value } : { ok: false, error: started.error };
}

export function ForwardTab(props: PaneTabProps) {
  const hosts = useHostsOptional();
  const hostId = () => String(props.tab.params?.hostId ?? props.hostId);
  const port = () => Number(props.tab.params?.port);
  const local = () => props.tab.params?.local === true;
  const hostName = () => hosts?.hosts().find((h) => h.id === hostId())?.name ?? 'PC';
  const [loaded, { refetch }] = createResource(() => [hostId(), port(), local()] as const, ([h, p, l]) => startForward(h, p, l));
  const stop = async (info: ForwardInfo) => {
    if (!info.id.startsWith('local-')) await forwardStop(info.id);
    props.close();
  };
  return (
    <Switch fallback={<EmptyState compact icon={<Plug size={22} />} title={`Opening localhost:${port()}…`} testId="forward-starting" />}>
      <Match when={loaded()?.ok === false && (loaded() as { ok: false; error: AppError })}>
        {(failed) => (
          <EmptyState
            compact
            icon={<Plug size={22} />}
            title={`Could not open port ${port()}`}
            body={failed().error.message}
            tone="error"
            action={
              <button type="button" class="btn btn-secondary btn-sm" onClick={() => void refetch()}>
                Try again
              </button>
            }
            testId="forward-error"
          />
        )}
      </Match>
      <Match when={loaded()?.ok === true && (loaded() as { ok: true; info: ForwardInfo }).info}>
        {(info) => <ForwardViewer embedded forward={info()} hostName={hostName()} onClose={props.close} onStop={() => void stop(info())} />}
      </Match>
    </Switch>
  );
}

setPaneComponent('forward', ForwardTab);
