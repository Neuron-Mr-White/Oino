/**
 * The phone model sheet's sticky footer (phone spec §3.5, PH-05): the thinking
 * level as one `Segmented` row (Off · Low · Medium · High …, the levels the
 * current model supports) — the phone has no separate Thinking chip. Hidden for
 * a model that does not reason (one level). Same host calls as the desktop's
 * `ThinkingChip` (`get_available_thinking_levels` / `set_thinking_level`).
 */
import { createEffect, Show } from 'solid-js';
import type { SessionHandle } from '../../lib/session/store';
import { canThink, ensureThinkingLevels, setThinkingLevel, thinkingLabel, thinkingLevels } from '../../lib/model/thinking';
import { haptic, Segmented } from '../phone';

export function ThinkingFooter(props: { session: SessionHandle }) {
  createEffect(() => {
    void props.session.state.model?.provider;
    void props.session.state.model?.id;
    void props.session.state.conn;
    ensureThinkingLevels(props.session);
  });
  const levels = () => thinkingLevels(props.session) ?? [];
  const current = () => props.session.state.thinkingLevel ?? levels()[0] ?? 'off';
  return (
    <Show when={canThink(levels())}>
      <div class="ph-think-foot" data-testid="model-thinking-footer">
        <span class="ph-think-title">Thinking</span>
        <Segmented
          label="Thinking level"
          value={current()}
          options={levels().map((level) => ({ value: level, label: thinkingLabel(level) }))}
          onChange={(level) => {
            if (level === current()) return;
            haptic('selection');
            void setThinkingLevel(props.session, level).catch(() => undefined);
          }}
        />
      </div>
    </Show>
  );
}
