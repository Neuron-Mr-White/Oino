import { LoaderCircle } from 'lucide-solid';
import { Show } from 'solid-js';
import { useSelectedHost } from '../shell/HostSwitcher';
import { useSuperagentChatWorking } from './chatActivity';

/**
 * The Superagent row's right end in the sidebar rail: only the working spinner while the
 * Superagent chat itself streams. Counts live on the page's workbench capsule (owner, R5).
 */
export function SuperagentRailIndicator() {
  const selected = useSelectedHost();
  const working = useSuperagentChatWorking(() => selected()?.id);
  return (
    <span class="rail-end" data-testid="rail-superagent-end">
      <Show when={working()}>
        <span class="rail-working" role="img" aria-label="Superagent is working" title="Working" data-testid="rail-superagent-working">
          <LoaderCircle size={14} class="srow-spin" />
        </span>
      </Show>
    </span>
  );
}
