/**
 * A delegated step (subagent-step / sidekick-step custom entry): the tool-row
 * shape naming the agent and its task. Z-42: it opens **the** child
 * transcript — the `subagent` side-pane tab (a sheet on the phone) — instead
 * of a second "Open session" path.
 */
import { Show } from 'solid-js';
import { Bot, ChevronRight } from 'lucide-solid';
import type { SessionEntry } from '../../../lib/session/types';
import type { TimelineCtx } from '../context';
import { agentOfStepData } from '../../panes/tabs/agentEntry';
import { openSubagentTab } from '../../panes/tabs/sessionTabs';

export function StepRow(props: { entry: SessionEntry; ctx: TimelineCtx }) {
  const d = () => ((props.entry as { data?: Record<string, unknown> }).data ?? {}) as { kind?: string; name?: string; arg?: string; text?: string; output?: string };
  const agent = () => agentOfStepData(props.entry.data);
  const open = () => {
    const a = agent();
    if (!a) return;
    openSubagentTab({ hostId: props.ctx.hostId, sessionId: props.ctx.sessionId(), cwd: props.ctx.cwd() }, a);
  };
  return (
    <div class="tool tl-step" data-state="ok" data-testid="tl-step">
      <Show
        when={agent()}
        fallback={
          <div class="tool-row" aria-disabled="true">
            <span class="tool-icon">
              <Bot size={15} />
            </span>
            <span class="tool-name">{d().name ?? d().kind ?? 'step'}</span>
            <span class="tool-sep" />
            <span class="tool-subject">{d().arg ?? d().text ?? ''}</span>
          </div>
        }
      >
        <button type="button" class="tool-row" onClick={open} title="Open the subagent transcript" data-testid="tl-step-open">
          <span class="tool-icon">
            <Bot size={15} />
          </span>
          <span class="tool-name">{agent()!.name ?? d().name ?? d().kind ?? 'step'}</span>
          <span class="tool-sep" />
          <span class="tool-subject">{d().arg ?? d().text ?? ''}</span>
          <ChevronRight size={15} class="proc-chev" />
        </button>
      </Show>
    </div>
  );
}
