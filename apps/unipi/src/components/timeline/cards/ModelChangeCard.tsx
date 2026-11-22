/**
 * Native change cards (WP-A8): model change, thinking-level change and
 * branch summary stay app-rendered (the spec's kept-native list) — light
 * boundary rows inside the work fold.
 */
import { Cpu, GitBranch, Sparkles } from 'lucide-solid';
import { Dynamic } from 'solid-js/web';
import type { SessionEntry } from '../../../lib/session/types';
import { modelDisplayName } from '../../../lib/model/displayName';
import { catalogueOf } from '../../../lib/model/thinking';

type NamedModel = { provider?: string; id?: string; name?: string };

/**
 * The name the chip, picker and toast use: the catalogue row's (or the chat's running model's)
 * `name`, else the id made readable. Id-only naming read "Dead 1" / "Mock 1" for "Dead Local" / "Mock".
 */
function nameOfModel(e: SessionEntry, hostId: string | undefined, current: NamedModel | undefined): string {
  const id = String((e as { modelId?: unknown }).modelId ?? (e as { model?: unknown }).model ?? '').trim();
  const provider = String((e as { provider?: unknown }).provider ?? '').trim();
  const same = (m: NamedModel) => m.id === id && (!provider || !m.provider || m.provider === provider);
  const row = (hostId ? catalogueOf(hostId)?.find(same) : undefined) ?? (current && same(current) ? current : undefined);
  return modelDisplayName(row ?? { id });
}

const textOf2 = (e: SessionEntry, hostId?: string, current?: NamedModel): string => {
  if (e.type === 'model_change') {
    const model = String((e as { modelId?: unknown }).modelId ?? (e as { model?: unknown }).model ?? '').trim();
    return model ? `Model → ${nameOfModel(e, hostId, current)}` : 'Model changed';
  }
  if (e.type === 'thinking_level_change') {
    const level = String((e as { thinkingLevel?: unknown; level?: unknown }).thinkingLevel ?? (e as { level?: unknown }).level ?? '').trim();
    return level ? `Thinking → ${level}` : 'Thinking level changed';
  }
  if (e.type === 'branch_summary') return `Branch: ${String((e as { title?: unknown }).title ?? 'earlier messages')}`;
  return String((e as { text?: unknown }).text ?? e.type);
};

export function ModelChangeCard(props: { entry: SessionEntry; hostId?: string; current?: () => NamedModel | undefined }) {
  const e = () => props.entry;
  const label = () => textOf2(e(), props.hostId, props.current?.());
  return (
    <div class="tl-card" data-kind={e().type} data-testid="tl-model-change" title={label()}>
      <span class="tool-icon">
        <Dynamic component={e().type === 'branch_summary' ? GitBranch : e().type === 'thinking_level_change' ? Sparkles : Cpu} size={15} />
      </span>
      <span class="tl-card-text truncate">{label()}</span>
    </div>
  );
}
