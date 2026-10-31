/**
 * Card dispatcher (WP-A8): one place for the entries that render as cards
 * inside a work fold. Native: model / thinking-level changes and branch
 * summaries (`ModelChangeCard`), bash executions (the terminal light row).
 * Everything `custom` / `custom_message` goes through the extension
 * renderer (`RendererCard`, G-21) with the native light row as fallback.
 */
import { Show } from 'solid-js';
import { CornerDownRight, SquareTerminal } from 'lucide-solid';
import type { SessionEntry } from '../../../lib/session/types';
import type { TimelineCtx } from '../context';
import { agentOfEntry } from '../../panes/tabs/agentEntry';
import { openSubagentTab } from '../../panes/tabs/sessionTabs';
import { ModelChangeCard } from './ModelChangeCard';
import { RendererCard } from './RendererCard';

export function CardRow(props: { entry: SessionEntry; ctx: TimelineCtx }) {
  const e = () => props.entry;
  if (e().type === 'model_change' || e().type === 'thinking_level_change' || e().type === 'branch_summary') {
    return <ModelChangeCard entry={e()} />;
  }
  if (e().message?.role === 'bashExecution') {
    const cmdText = () => String((e().message?.content as string) ?? '');
    return (
      <div class="tl-card" data-kind="user-bash" data-testid="tl-bash" title={cmdText()}>
        <span class="tool-icon">
          <SquareTerminal size={15} />
        </span>
        <span class="tl-card-text truncate">{cmdText()}</span>
      </div>
    );
  }
  if (e().type === 'custom_message' && ((e() as { display?: unknown }).display === false || (e().message as { display?: unknown } | undefined)?.display === false)) {
    return null;
  }
  // Z-42: a subagent / sidekick card has one way into the child transcript —
  // the `subagent` tab (a sheet on the phone). No second "Open session" path.
  const agent = () => agentOfEntry(e());
  const openAgent = () => {
    const a = agent();
    if (!a) return;
    openSubagentTab({ hostId: props.ctx.hostId, sessionId: props.ctx.sessionId(), cwd: props.ctx.cwd() }, a);
  };
  return (
    <div class="tl-cardblock">
      <RendererCard entry={e()} ctx={props.ctx} />
      <Show when={agent()}>
        <button type="button" class="tl-card-transcript" onClick={openAgent} data-testid="tl-card-transcript">
          <CornerDownRight size={13} />
          <span>Transcript</span>
        </button>
      </Show>
    </div>
  );
}
