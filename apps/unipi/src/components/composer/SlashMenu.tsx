/**
 * The `/` menu (WP-A12, Z-62): commands, prompt templates and skills of the
 * open session, one row each with its source label (`app`, `command`,
 * `prompt`, `skill` — `lib/chat/commands.ts`), above the composer. No `#`
 * conversations / `$` skills triggers (Z-62 ruling: unipi keeps `/` and `@`).
 *
 * Keyboard: ↑/↓ move, Enter/Tab pick, Esc closes (handled by the composer's
 * key path — the rows are buttons, so a pointer pick works too).
 */
import { For, Show } from 'solid-js';
import type { CommandInfo } from '../../lib/session/types';
import { SOURCE_LABEL, TERMINAL_ONLY, sourceOf } from '../../lib/chat/commands';

/** The source tag one row shows (terminal-only pi built-ins say "terminal"). */
export function commandSource(c: CommandInfo): string {
  return TERMINAL_ONLY.has(c.name) && c.source === 'builtin' ? 'terminal' : SOURCE_LABEL[sourceOf(c)];
}

export function SlashMenu(props: {
  items: readonly CommandInfo[];
  /** The highlighted row (keyboard) — also the aria-activedescendant. */
  selected: number;
  onPick: (name: string) => void;
  onHover: (index: number) => void;
  label?: string;
}) {
  return (
    <div class="cmenu" role="listbox" aria-label={props.label ?? 'Commands'} data-testid="slash-menu">
      <For each={props.items}>
        {(c, i) => (
          <button
            type="button"
            role="option"
            class="cmenu-row"
            data-sel={i() === props.selected ? '' : undefined}
            id={`slash-opt-${i()}`}
            aria-selected={i() === props.selected ? 'true' : 'false'}
            onPointerDown={(e) => e.preventDefault()}
            onMouseDown={(e) => e.preventDefault()}
            onMouseEnter={() => props.onHover(i())}
            onClick={() => props.onPick(c.name)}
          >
            <span class="cmenu-name">/{c.name}</span>
            <Show when={c.description}>
              <span class="cmenu-desc">{c.description}</span>
            </Show>
            <span class="cmenu-src" data-src={commandSource(c)}>
              {commandSource(c)}
            </span>
          </button>
        )}
      </For>
    </div>
  );
}
