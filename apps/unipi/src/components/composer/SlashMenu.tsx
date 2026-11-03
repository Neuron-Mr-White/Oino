/**
 * The `/` menu (WP-A12, Z-62): commands, prompt templates and skills of the
 * open session, one row each with its source label (`app`, `command`,
 * `prompt`, `skill` — `lib/chat/commands.ts`), above the composer. The same
 * list is the `$` skills menu (owner spec 2026-11-03, `prefix="$"`).
 *
 * Keyboard: ↑/↓ move, Enter/Tab pick, Esc closes (handled by the composer's
 * key path — the rows are buttons, so a pointer pick works too).
 */
import { For, Show } from 'solid-js';
import type { CommandInfo } from '../../lib/session/types';
import { SOURCE_LABEL, sourceOf } from '../../lib/chat/commands';

/** The source tag one row shows (`app` for pi built-ins: each has an app surface, G-15). */
export function commandSource(c: CommandInfo): string {
  return SOURCE_LABEL[sourceOf(c)];
}

export function SlashMenu(props: {
  items: readonly CommandInfo[];
  /** The highlighted row (keyboard) — also the aria-activedescendant. */
  selected: number;
  onPick: (name: string) => void;
  onHover: (index: number) => void;
  label?: string;
  /** The trigger shown before each name: `/` (commands) or `$` (skills). */
  prefix?: '/' | '$';
  testId?: string;
}) {
  const p = () => props.prefix ?? '/';
  return (
    <div class="cmenu" role="listbox" aria-label={props.label ?? 'Commands'} data-testid={props.testId ?? 'slash-menu'}>
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
            title={c.description ? `${p()}${c.name} — ${c.description}` : `${p()}${c.name}`}
          >
            <span class="cmenu-name">
              {p()}
              {c.name}
            </span>
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
