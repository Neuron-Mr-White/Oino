/**
 * Native change cards (WP-A8): model change, thinking-level change and
 * branch summary stay app-rendered (the spec's kept-native list) — light
 * boundary rows inside the work fold.
 */
import { Cpu, GitBranch, Sparkles } from 'lucide-solid';
import { Dynamic } from 'solid-js/web';
import type { SessionEntry } from '../../../lib/session/types';

const textOf2 = (e: SessionEntry): string => {
  if (e.type === 'model_change') {
    const model = String((e as { modelId?: unknown }).modelId ?? (e as { model?: unknown }).model ?? '').trim();
    return model ? `Model → ${model}` : 'Model changed';
  }
  if (e.type === 'thinking_level_change') {
    const level = String((e as { thinkingLevel?: unknown; level?: unknown }).thinkingLevel ?? (e as { level?: unknown }).level ?? '').trim();
    return level ? `Thinking → ${level}` : 'Thinking level changed';
  }
  if (e.type === 'branch_summary') return `Branch: ${String((e as { title?: unknown }).title ?? 'earlier messages')}`;
  return String((e as { text?: unknown }).text ?? e.type);
};

export function ModelChangeCard(props: { entry: SessionEntry }) {
  const e = () => props.entry;
  const label = () => textOf2(e());
  return (
    <div class="tl-card" data-kind={e().type} data-testid="tl-model-change" title={label()}>
      <span class="tool-icon">
        <Dynamic component={e().type === 'branch_summary' ? GitBranch : e().type === 'thinking_level_change' ? Sparkles : Cpu} size={15} />
      </span>
      <span class="tl-card-text truncate">{label()}</span>
    </div>
  );
}
