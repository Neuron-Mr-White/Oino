/**
 * [ux-sessions] The session list's header (owner reference 2026-11-03, a zcode-style "Task
 * list"): the title with a search icon on the right (and, from the Properties move, the
 * filter icon). The search icon opens a query row that searches the HOST (`sessions_query`,
 * lib/session/sidebarSearch): `/` or Ctrl+F in the sidebar focuses it, Esc clears it.
 */
import { createEffect, on, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { Search, X } from 'lucide-solid';
import { clearSidebarSearch, openSidebarSearch, setSidebarSearchText, sidebarSearch } from '../../lib/session/sidebarSearch';

const editable = (el: EventTarget | null): boolean => {
  const node = el as HTMLElement | null;
  if (!node || typeof node.closest !== 'function') return false;
  return !!node.closest('input, textarea, select, [contenteditable=""], [contenteditable="true"], .cm-editor');
};

/** Inside the sidebar (desktop `.sb`, or the session list itself). */
const inSidebar = (el: EventTarget | null): boolean => {
  const node = el as HTMLElement | null;
  return !!node && typeof node.closest === 'function' && !!node.closest('.sb, .sb-sessions');
};

export function TaskListHeader(props: { trailing?: JSX.Element; label?: string }): JSX.Element {
  let input: HTMLInputElement | undefined;
  createEffect(
    on(
      sidebarSearch.focusTick,
      () => queueMicrotask(() => {
        input?.focus();
        input?.select();
      }),
      { defer: true },
    ),
  );
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.isComposing) return;
      const target = e.target;
      const slash = e.key === '/' && !e.ctrlKey && !e.metaKey && !e.altKey && !editable(target) && (inSidebar(target) || target === document.body);
      const find = (e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === 'f' && inSidebar(document.activeElement);
      if (!slash && !find) return;
      e.preventDefault();
      openSidebarSearch();
    };
    document.addEventListener('keydown', onKey);
    onCleanup(() => document.removeEventListener('keydown', onKey));
  });
  return (
    <div class="sb-tasks" data-testid="sb-tasks">
      <div class="sb-tasks-head">
        <span class="sb-tasks-title">{props.label ?? 'Task list'}</span>
        <span class="sb-tasks-tools">
          <button
            type="button"
            class="sb-tasks-icon"
            data-active={sidebarSearch.open() ? '' : undefined}
            aria-label="Search sessions (/)"
            title="Search sessions (/ or Ctrl+F)"
            aria-expanded={sidebarSearch.open()}
            data-testid="sb-search-toggle"
            onClick={() => (sidebarSearch.open() && !sidebarSearch.text() ? clearSidebarSearch() : openSidebarSearch())}
          >
            <Search size={14} />
          </button>
          {props.trailing}
        </span>
      </div>
      <Show when={sidebarSearch.open()}>
        <div class="sb-search" role="search">
          <Search size={13} class="sb-search-icon" aria-hidden="true" />
          <input
            ref={input}
            type="search"
            class="sb-search-input"
            placeholder="Search titles and messages"
            aria-label="Search sessions"
            autocomplete="off"
            spellcheck={false}
            maxLength={256}
            value={sidebarSearch.text()}
            data-testid="sb-search-input"
            onInput={(e) => setSidebarSearchText(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === 'Escape') {
                e.preventDefault();
                e.stopPropagation();
                clearSidebarSearch();
                (document.activeElement as HTMLElement | null)?.blur?.();
              }
            }}
          />
          <Show when={sidebarSearch.text()}>
            <button type="button" class="sb-search-clear" aria-label="Clear search" title="Clear (Esc)" data-testid="sb-search-clear" onClick={() => (setSidebarSearchText(''), input?.focus())}>
              <X size={13} />
            </button>
          </Show>
        </div>
      </Show>
    </div>
  );
}
