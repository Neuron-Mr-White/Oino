/**
 * The `@` menu (WP-A12, Z-62): files (and folders) of the session's project,
 * from the host `fs_list` (a bare `@`) or `fs_search` name mode (a query) —
 * ZCode behaviour in the unipi look: leaf name first, path second, folders
 * keep the query going (no trailing space), files close with one.
 */
import { For } from 'solid-js';
import { File as FileIcon, Folder } from 'lucide-solid';

export type MentionItem = {
  /** What inserts (with the `@`): `/abs/path` (folders keep no trailing space). */
  value: string;
  /** The row's main label: the leaf name. */
  label: string;
  /** The full path shown second. */
  path: string;
  dir: boolean;
};

export function MentionMenu(props: {
  items: readonly MentionItem[];
  selected: number;
  onPick: (item: MentionItem) => void;
  onHover: (index: number) => void;
}) {
  return (
    <div class="cmenu" role="listbox" aria-label="Files" data-testid="mention-menu">
      <For each={props.items}>
        {(item, i) => (
          <button
            type="button"
            role="option"
            class="cmenu-row"
            data-sel={i() === props.selected ? '' : undefined}
            id={`mention-opt-${i()}`}
            aria-selected={i() === props.selected ? 'true' : 'false'}
            onPointerDown={(e) => e.preventDefault()}
            onMouseDown={(e) => e.preventDefault()}
            onMouseEnter={() => props.onHover(i())}
            onClick={() => props.onPick(item)}
          >
            <span class="cmenu-icon">{item.dir ? <Folder size={15} /> : <FileIcon size={15} />}</span>
            <span class="cmenu-name">{item.label}</span>
            <span class="cmenu-desc cmenu-path">{item.path}</span>
          </button>
        )}
      </For>
    </div>
  );
}
