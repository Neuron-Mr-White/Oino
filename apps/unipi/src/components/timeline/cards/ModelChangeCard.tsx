/**
 * Native change cards (WP-A8): model change, thinking-level change and
 * branch summary stay app-rendered (the spec's kept-native list) — light
 * boundary rows inside the work fold.
 */
import { Cpu, GitBranch, Sparkles } from 'lucide-solid';
import { Dynamic } from 'solid-js/web';
import type { SessionEntry } from '../../../lib/session/types';

const textOf2 = (e: SessionEntry): string => {
  if (e.type === 'model_change') return `Model → ${String((e as { modelId?: unknown }).modelId ?? (e as { model?: unknown }).model ?? '')}`;
  if (e.type === 'thinking_level_change') return `Thinking → ${String((e as { level?: unknown }).level ?? '')}`;
  if (e.type === 'branch_summary') return `Branch: ${String((e as { title?: unknown }).title ?? 'earlier messages')}`;
  return String((e as { text?: unknown }).text ?? e.type);
};

export function ModelChangeCard(props: { entry: SessionEntry }) {
  const e = () => props.entry;
  return (
    <div class="tl-card" data-kind={e().type} data-testid="tl-model-change">
      <span class="tool-icon">
        <Dynamic component={e().type === 'branch_summary' ? GitBranch : e().type === 'thinking_level_change' ? Sparkles : Cpu} size={15} />
      </span>
      <span class="tl-card-text truncate">{textOf2(e())}</span>
    </div>
  );
}
