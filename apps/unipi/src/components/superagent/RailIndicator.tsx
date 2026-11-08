import { LoaderCircle } from 'lucide-solid';
import { Match, Show, Switch } from 'solid-js';
import { useSelectedHost } from '../shell/HostSwitcher';
import { useSuperagentChatWorking } from './chatActivity';
import { useRailStatus } from './railStatus';

/**
 * The Superagent row's right end in the sidebar rail. Left to right: the working spinner
 * (the Superagent chat itself streams, same spinner as a working chat row), then a count pill
 * while anything waits on the owner (wins over the dot), else a pulsing dot while leads run.
 * They sit side by side in one flex group, never over each other.
 */
export function SuperagentRailIndicator() {
  const selected = useSelectedHost();
  const status = useRailStatus(() => selected()?.id);
  const working = useSuperagentChatWorking(() => selected()?.id);
  const needs = () => status()?.needs ?? 0;
  const leads = () => status()?.leads ?? 0;
  return (
    <span class="rail-end" data-testid="rail-superagent-end">
      <Show when={working()}>
        <span class="rail-working" role="img" aria-label="Superagent is working" title="Working" data-testid="rail-superagent-working">
          <LoaderCircle size={14} class="srow-spin" />
        </span>
      </Show>
      <Switch>
        <Match when={needs() > 0}>
          <span class="rail-count" role="img" aria-label={`${needs()} waiting for you`} title={`${needs()} waiting for you`} data-testid="rail-superagent-needs">{needs() > 9 ? '9+' : needs()}</span>
        </Match>
        <Match when={leads() > 0 && !working()}>
          <span class="rail-live" role="img" aria-label={`${leads()} ${leads() === 1 ? 'lead' : 'leads'} running`} title={`${leads()} ${leads() === 1 ? 'lead' : 'leads'} running`} data-testid="rail-superagent-live" />
        </Match>
      </Switch>
    </span>
  );
}
