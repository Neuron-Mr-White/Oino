import { Match, Switch } from 'solid-js';
import { useSelectedHost } from '../shell/HostSwitcher';
import { useRailStatus } from './railStatus';

/** The Superagent row's right end in the sidebar rail: a count pill while anything waits on the owner (wins), a pulsing dot while leads run, else nothing. */
export function SuperagentRailIndicator() {
  const selected = useSelectedHost();
  const status = useRailStatus(() => selected()?.id);
  const needs = () => status()?.needs ?? 0;
  const leads = () => status()?.leads ?? 0;
  return (
    <Switch>
      <Match when={needs() > 0}>
        <span class="rail-count" role="img" aria-label={`${needs()} waiting for you`} title={`${needs()} waiting for you`} data-testid="rail-superagent-needs">{needs() > 9 ? '9+' : needs()}</span>
      </Match>
      <Match when={leads() > 0}>
        <span class="rail-live" role="img" aria-label={`${leads()} ${leads() === 1 ? 'lead' : 'leads'} running`} title={`${leads()} ${leads() === 1 ? 'lead' : 'leads'} running`} data-testid="rail-superagent-live" />
      </Match>
    </Switch>
  );
}
