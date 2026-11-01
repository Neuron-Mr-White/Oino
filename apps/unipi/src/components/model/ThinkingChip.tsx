/**
 * Thinking chip (WP-A14, Z-71): a separate composer control (the model sheet
 * no longer holds the level radios) — a brain icon, the level text, and a
 * menu of the levels the current model supports. Ctrl/Cmd+T cycles without
 * opening the menu (the shell's binding sends `cycle_thinking_level`).
 * No ladder bar (Z-71: "no ladder bar unless asked").
 *
 * The list and the changes are the host's (`get_available_thinking_levels`,
 * `set_thinking_level`); a model that does not reason has one level, so the
 * chip stays hidden rather than offering a dead control.
 */
import { createEffect, createSignal, For, onCleanup, Show } from 'solid-js';
import { Brain, Check, ChevronUp } from 'lucide-solid';
import type { SessionHandle } from '../../lib/session/store';
import { canThink, ensureThinkingLevels, setThinkingLevel, thinkingHint, thinkingLabel, thinkingLevels } from '../../lib/model/thinking';
import { AnchoredMenu } from '../board/AnchoredMenu';
import { menuKeys } from '../composer/menuKeys';
import { RollingLabel } from '../RollingLabel';

export function ThinkingChip(props: { session: SessionHandle; disabled?: boolean }) {
  // Track model + connection: a switch or a reconnect re-asks the host.
  createEffect(() => {
    void props.session.state.model?.provider;
    void props.session.state.model?.id;
    void props.session.state.conn;
    ensureThinkingLevels(props.session);
  });
  const levels = () => thinkingLevels(props.session);
  const current = () => props.session.state.thinkingLevel ?? levels()?.[0] ?? 'off';
  const [open, setOpen] = createSignal(false);
  const [anchor, setAnchor] = createSignal<DOMRect | undefined>();
  let chip: HTMLButtonElement | undefined;

  const toggle = (e: MouseEvent) => {
    if (props.disabled) return;
    if (open()) {
      setOpen(false);
      return;
    }
    setAnchor((e.currentTarget as HTMLElement).getBoundingClientRect());
    setOpen(true);
  };
  const pick = (level: string) => {
    setOpen(false);
    if (level !== current()) void setThinkingLevel(props.session, level).catch(() => undefined);
  };

  return (
    <Show when={canThink(levels())}>
      <button
        type="button"
        class="tb-chip tb-chip-think"
        aria-label={`Thinking level: ${thinkingLabel(current())}. Change it`}
        aria-haspopup="menu"
        aria-expanded={open()}
        title={`Thinking: ${thinkingLabel(current())}`}
        disabled={props.disabled}
        ref={chip}
        onClick={toggle}
        data-testid="thinking-chip"
      >
        <Brain size={15} class="tb-ic" />
        <RollingLabel class="tb-label" text={thinkingLabel(current())} testId="thinking-chip-label" />
        <ChevronUp size={12} class="tb-chev" />
      </button>
      <Show when={open() && anchor()}>
        {/* D-23: the permission menu's look — label over a one-line description; D-05: keyboard. */}
        <AnchoredMenu anchor={anchor()!} width={276} label="Thinking level" onDismiss={() => setOpen(false)}>
          <div class="menu-keys" ref={(el) => onCleanup(menuKeys(el, { close: () => setOpen(false), trigger: () => chip }))}>
            <For each={levels()}>
              {(level) => (
                <button
                  type="button"
                  role="menuitemradio"
                  aria-checked={level === current()}
                  class="perm-menu-row think-menu-row"
                  data-label={thinkingLabel(level)}
                  onClick={() => pick(level)}
                  data-testid={`thinking-option-${level}`}
                >
                  <span class="perm-menu-check">{level === current() ? <Check size={16} /> : null}</span>
                  <span class="perm-menu-label">{thinkingLabel(level)}</span>
                  <Show when={thinkingHint(level)}>
                    <span class="perm-menu-desc">{thinkingHint(level)}</span>
                  </Show>
                </button>
              )}
            </For>
          </div>
        </AnchoredMenu>
      </Show>
    </Show>
  );
}
