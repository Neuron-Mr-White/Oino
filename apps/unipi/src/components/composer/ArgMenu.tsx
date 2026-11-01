/**
 * The `/cmd <arg>` menu (P1-01): argument completions an extension command
 * registers (`complete-command`) — `/cron run <job>`, `/dream <sub>`,
 * `/kanboard …` — fetched from the session's `_oino/get_completions`
 * (ui-protocol §7.3). Same look and keys as the `/` menu: ↑/↓ move,
 * Enter/Tab pick, Esc closes.
 */
import { For, Show } from 'solid-js';

export type ArgItem = {
  /** What replaces the argument (from the group's `from` to the caret). */
  value: string;
  label: string;
  description?: string;
  tag?: string;
};

export function ArgMenu(props: {
  command: string;
  items: readonly ArgItem[];
  selected: number;
  onPick: (item: ArgItem) => void;
  onHover: (index: number) => void;
}) {
  return (
    <div class="cmenu" role="listbox" aria-label={`/${props.command} arguments`} data-testid="arg-menu">
      <For each={props.items}>
        {(item, i) => (
          <button
            type="button"
            role="option"
            class="cmenu-row"
            data-sel={i() === props.selected ? '' : undefined}
            id={`arg-opt-${i()}`}
            aria-selected={i() === props.selected ? 'true' : 'false'}
            onPointerDown={(e) => e.preventDefault()}
            onMouseDown={(e) => e.preventDefault()}
            onMouseEnter={() => props.onHover(i())}
            onClick={() => props.onPick(item)}
            title={item.description ? `${item.label} — ${item.description}` : item.label}
          >
            <span class="cmenu-name">{item.label}</span>
            <Show when={item.description}>
              <span class="cmenu-desc">{item.description}</span>
            </Show>
            <Show when={item.tag}>
              <span class="cmenu-src">{item.tag}</span>
            </Show>
          </button>
        )}
      </For>
    </div>
  );
}

type Group = { source?: string; from?: number | null; items?: Array<{ value?: string; label?: string; description?: string; tag?: string }> };

/** The `command:<name>` group of a `_oino/get_completions` reply as menu rows (pure).
 *  A row whose value is exactly what is typed already adds nothing and is dropped. */
export function argItemsOf(reply: unknown, command: string, typed: string): { from: number | null; items: ArgItem[] } {
  const groups = ((reply as { groups?: Group[] } | null)?.groups ?? []) as Group[];
  const group = groups.find((g) => g.source === `command:${command}`);
  if (!group) return { from: null, items: [] };
  const items = (group.items ?? [])
    .filter((it): it is { value: string; label?: string; description?: string; tag?: string } => typeof it.value === 'string')
    .filter((it) => it.value.trim() !== typed.trim())
    .slice(0, 60)
    .map((it) => ({
      value: it.value,
      label: it.label || it.value,
      ...(it.description ? { description: it.description } : {}),
      ...(it.tag ? { tag: it.tag } : {}),
    }));
  return { from: typeof group.from === 'number' ? group.from : null, items };
}
