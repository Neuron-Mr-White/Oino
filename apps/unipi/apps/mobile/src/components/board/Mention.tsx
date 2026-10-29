/**
 * `@` task mentions for a phone textarea (web mention.tsx, UNI-131):
 * typing `@` (at the start or after a space) shows the most recently
 * created tasks as a horizontal chip strip above the field (no caret
 * popover — the soft keyboard owns the bottom half); typing more filters
 * by id or title; a tap inserts the id and keeps the keyboard up.
 */
import { createMemo, createSignal, For, Show } from 'solid-js';
import { displayTitle } from '../../lib/board/api';
import { mentionAt, mentionCandidates } from '../../lib/board/view';
import { boardStore } from '../../lib/board/store';
import { StatusGlyph } from './Glyphs';
import { setFieldValue } from '../../lib/fieldEdit';

export function createMention(area: () => HTMLTextAreaElement | undefined, setValue: (next: string) => void, selfId?: () => string | undefined) {
  const [mention, setMention] = createSignal<{ start: number; needle: string } | null>(null);
  const store = boardStore();
  const list = createMemo(() => {
    const current = mention();
    return current ? mentionCandidates(store.state.tasks, current.needle, selfId?.()) : [];
  });
  /** Call from the textarea's input / click / keyup. */
  const update = () => {
    const el = area();
    if (!el || el.selectionStart !== el.selectionEnd) return setMention(null);
    setMention(mentionAt(el.value, el.selectionStart ?? el.value.length));
  };
  const insert = (id: string) => {
    const current = mention();
    const el = area();
    if (!current || !el) return;
    const caret = el.selectionStart ?? el.value.length;
    const next = `${el.value.slice(0, current.start)}${id} ${el.value.slice(caret)}`;
    const position = current.start + id.length + 1;
    // UNI-270: as one undoable edit (Ctrl/⌘+Z takes the mention back out).
    setFieldValue(el, next, position);
    setValue(next);
    el.focus();
    setMention(null);
  };
  return { list, update, insert, close: () => setMention(null) };
}

export function MentionStrip(props: { mention: ReturnType<typeof createMention> }) {
  const keep = (event: Event) => event.preventDefault();
  return (
    <Show when={props.mention.list().length > 0}>
      <div class="mention-strip" role="listbox" aria-label="Mention a task">
        <For each={props.mention.list()}>
          {(task) => (
            <button
              type="button"
              role="option"
              class="mention-chip"
              onPointerDown={keep}
              onMouseDown={keep}
              onClick={() => props.mention.insert(task.id)}
            >
              <StatusGlyph status={task.status} size={12} />
              <span class="font-mono">{task.id}</span>
              <span class="truncate" classList={{ derived: !task.title }}>
                {displayTitle(task)}
              </span>
            </button>
          )}
        </For>
      </div>
    </Show>
  );
}
