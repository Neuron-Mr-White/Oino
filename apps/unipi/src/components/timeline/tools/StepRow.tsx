/**
 * A delegated step (subagent-step / sidekick-step custom entry): a
 * tool-row-shaped line naming the agent and its task. A10 turns its
 * "Open session" paths into the `subagent` side-pane tab.
 */
import { Bot } from 'lucide-solid';
import type { SessionEntry } from '../../../lib/session/types';

export function StepRow(props: { entry: SessionEntry }) {
  const d = () => ((props.entry as { data?: Record<string, unknown> }).data ?? {}) as { kind?: string; name?: string; arg?: string; text?: string; output?: string };
  return (
    <div class="tool tl-step" data-state="ok" data-testid="tl-step">
      <div class="tool-row" aria-disabled="true">
        <span class="tool-icon">
          <Bot size={15} />
        </span>
        <span class="tool-name">{d().name ?? d().kind ?? 'step'}</span>
        <span class="tool-sep" />
        <span class="tool-subject">{d().arg ?? d().text ?? ''}</span>
      </div>
    </div>
  );
}
