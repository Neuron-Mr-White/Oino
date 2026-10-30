/**
 * Card dispatcher (WP-A8): one place for the entries that render as cards
 * inside a work fold. Native: model / thinking-level changes and branch
 * summaries (`ModelChangeCard`), bash executions (the terminal light row).
 * Everything `custom` / `custom_message` goes through the extension
 * renderer (`RendererCard`, G-21) with the native light row as fallback.
 */
import { SquareTerminal } from 'lucide-solid';
import type { SessionEntry } from '../../../lib/session/types';
import type { TimelineCtx } from '../context';
import { ModelChangeCard } from './ModelChangeCard';
import { RendererCard } from './RendererCard';

export function CardRow(props: { entry: SessionEntry; ctx: TimelineCtx }) {
  const e = () => props.entry;
  if (e().type === 'model_change' || e().type === 'thinking_level_change' || e().type === 'branch_summary') {
    return <ModelChangeCard entry={e()} />;
  }
  if (e().message?.role === 'bashExecution') {
    return (
      <div class="tl-card" data-kind="user-bash" data-testid="tl-bash">
        <span class="tool-icon">
          <SquareTerminal size={15} />
        </span>
        <span class="tl-card-text truncate">{String((e().message?.content as string) ?? '')}</span>
      </div>
    );
  }
  return <RendererCard entry={e()} ctx={props.ctx} />;
}
