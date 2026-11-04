/**
 * The workspace folder browser pane (owner-fix-5 redesign): a centred modal
 * over the app instead of a 320px menu with two visible rows. Finder-style
 * Miller columns (one `fs_list` per column, cached for the pane's session),
 * a Places rail (home, this PC's fs roots, recent workspaces), a breadcrumb
 * path bar that becomes a text input (Enter jumps, Esc reverts), a filter for
 * the active column, a "git" pill when a selected folder's listing contains a
 * `.git` entry, and a footer with [System dialog…] (own PC only) · Cancel ·
 * "Use this folder". Keyboard: ↑/↓ within the column, →/Enter opens, ← back,
 * Ctrl/Cmd+Enter uses, "/" focuses the filter, typing filters.
 *
 * The desktop workspace menu always opens this pane (own PC included); the
 * native dialog demotes to the footer's secondary button. The phone keeps its
 * Sheet + crumbs flow and never renders this.
 */
import { createEffect, createMemo, createResource, createSignal, For, Index, on, onCleanup, onMount, Show } from 'solid-js';
import { ChevronRight, Clock, Folder, Home, X } from 'lucide-solid';
import { fsList } from '../../lib/fs';
import { shortPath, folderName } from '../../lib/session/newSession';
import { pickFolderNative } from '../../lib/openLocal';
import { useHostsOptional } from '../../hosts/context';
import { wrapTab } from '../../lib/focusTrap';

export type FolderListing = { dirs: string[]; hasGit: boolean; error?: string };

/** D-20: a folder the PC doesn't let the app see reads calmly (no raw scope message, no CLI hint). */
export const calmFsError = (message: string): string =>
  /outside the allowed|allowed folders|E_SCOPE|scope/i.test(message)
    ? 'Unipi can’t open this folder on the PC. Go back to a folder it lists.'
    : message;

/** One column of the Miller view: `path` is listed; `sel` is the highlighted child name. */
type Col = { path: string; sel?: string };

const join = (dir: string, name: string) => `${dir.replace(/\/+$/, '')}/${name}`;

export function FolderBrowser(props: {
  hostId: string;
  home?: string;
  roots: readonly string[];
  /** The picker's current folder: the browser opens there (with its ancestors as columns). */
  initial?: string;
  /** Recent workspaces (the picker's `recentWorkspaces`). */
  recentItems: ReadonlyArray<{ path: string; label: string }>;
  /** This computer: the footer offers the native dialog. */
  ownPc: boolean;
  onClose: () => void;
  /** Picking a folder (button, double-click, Ctrl/Cmd+Enter, system dialog). */
  onUse: (path: string) => void;
}) {
  const hosts = useHostsOptional();
  const pcName = createMemo(() => {
    const h = hosts?.hosts().find((x) => x.id === props.hostId);
    return h ? (h.transport === 'local' ? 'This computer' : h.name) : undefined;
  });
  const short = (p: string) => shortPath(p, props.home);

  // ── listings (one fs_list per column, cached for this pane's session) ──
  const cache = new Map<string, FolderListing>();
  const inflight = new Map<string, Promise<FolderListing>>();
  // Bumped whenever the cache learns a folder: the git-pill reads the cache, which
  // Solid cannot track — this signal makes the reads reactive.
  const [cacheTick, setCacheTick] = createSignal(0);
  const hasGitOf = (path: string) => {
    cacheTick();
    return cache.get(path)?.hasGit ?? false;
  };
  const fetchListing = (path: string): Promise<FolderListing> => {
    const hit = cache.get(path);
    if (hit) return Promise.resolve(hit);
    let pending = inflight.get(path);
    if (!pending) {
      pending = fsList(props.hostId, path, true).then((res) => {
        const out: FolderListing = res.ok
          ? {
              dirs: res.value.entries
                .filter((e) => e.kind === 'dir' && !e.name.startsWith('.'))
                .map((e) => e.name)
                .sort((a, b) => a.localeCompare(b)),
              hasGit: res.value.entries.some((e) => e.name === '.git' && e.kind === 'dir'),
            }
          : { dirs: [], hasGit: false, error: calmFsError(String(res.error.message)) };
        cache.set(path, out);
        inflight.delete(path);
        setCacheTick((n) => n + 1);
        return out;
      });
      inflight.set(path, pending);
    }
    return pending;
  };

  // ── the columns ──
  /** Where a chain for `target` should start: home, the deepest containing root, or `/`. */
  const rootFor = (target: string): string => {
    if (props.home && (target === props.home || target.startsWith(`${props.home}/`))) return props.home;
    const under = props.roots
      .filter((r) => target === r || target.startsWith(`${r.replace(/\/+$/, '')}/`))
      .sort((a, b) => b.length - a.length)[0];
    return under ?? (target.startsWith('/') ? '/' : target);
  };
  /** Columns from `root` down to `target`, each ancestor selecting its child. */
  const chainFor = (target: string, root: string): Col[] => {
    const out: Col[] = [{ path: root }];
    if (target !== root) {
      let at = root;
      for (const part of target.slice(root.length).split('/').filter(Boolean)) {
        out[out.length - 1]!.sel = part;
        at = join(at, part);
        out.push({ path: at });
      }
    }
    return out;
  };
  const [cols, setCols] = createSignal<Col[]>(
    props.initial ? chainFor(props.initial, rootFor(props.initial)) : [{ path: props.home ?? props.roots[0] ?? '/' }],
  );
  /** The deepest folder with a selection, else the last column's own folder. */
  const selected = () => {
    const c = cols();
    for (let i = c.length - 1; i >= 0; i--) {
      const sel = c[i]!.sel;
      if (sel) return join(c[i]!.path, sel);
    }
    return c[c.length - 1]!.path;
  };
  /** The deepest opened column (what the path bar shows). */
  const deepest = () => cols()[cols().length - 1]!.path;
  /** Click (or →/Enter): select `name` in column `at` and open it into the next column. */
  const openFolder = (at: number, name: string) => {
    setCols((c) =>
      c.slice(0, at + 1).map((col, i) => (i === at ? { ...col, sel: name } : col)).concat([{ path: join(c[at]!.path, name) }]),
    );
  };
  const selectIn = (at: number, name: string | undefined) => {
    setCols((c) => c.map((col, i) => (i === at ? { ...col, sel: name } : col)));
  };
  const jumpTo = (path: string) => setCols(chainFor(path, rootFor(path)));
  const goBack = () => setCols((c) => (c.length > 1 ? c.slice(0, -1) : c));

  // ── path bar: breadcrumbs ⇄ text input ──
  const [editing, setEditing] = createSignal(false);
  const [draft, setDraft] = createSignal('');
  const [pathError, setPathError] = createSignal<string | undefined>();
  const crumbs = createMemo(() => {
    const dir = deepest();
    const inHome = !!props.home && (dir === props.home || dir.startsWith(`${props.home}/`));
    const base = inHome ? props.home! : '';
    const rest = (inHome ? dir.slice(props.home!.length) : dir).split('/').filter(Boolean);
    const out = [{ label: inHome ? '~' : '/', path: inHome ? props.home! : '/' }];
    let at = base;
    for (const part of rest) {
      at = `${at}/${part}`;
      out.push({ label: part, path: at });
    }
    return out;
  });
  const startEdit = () => {
    setPathError(undefined);
    setDraft(deepest());
    setEditing(true);
  };
  const submitEdit = async () => {
    const target = draft().trim();
    if (!target) return setEditing(false);
    const probe = await fetchListing(target).catch(
      () => ({ dirs: [], hasGit: false, error: calmFsError('outside the allowed folders') }) as FolderListing,
    );
    if (probe.error) {
      setPathError(probe.error);
      return;
    }
    setPathError(undefined);
    setEditing(false);
    jumpTo(target);
  };
  const exitEdit = () => {
    setEditing(false);
    setPathError(undefined);
  };

  // ── filter (the active column) ──
  const [filter, setFilter] = createSignal('');
  let filterEl: HTMLInputElement | undefined;
  const focusFilter = (seed?: string) => {
    filterEl?.focus();
    if (seed !== undefined && !filterEl?.value) setFilter(seed);
  };

  // ── keyboard ──
  const useSelected = () => props.onUse(selected());
  const onKeys = (e: KeyboardEvent) => {
    const inInput = e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement;
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'l') {
      e.preventDefault();
      startEdit();
      return;
    }
    if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
      e.preventDefault();
      useSelected();
      return;
    }
    if (e.key === 'Escape') {
      e.stopPropagation();
      if (editing()) exitEdit();
      else props.onClose();
      return;
    }
    if (inInput) return;
    if (e.key === '/') {
      e.preventDefault();
      focusFilter();
      return;
    }
    const last = cols().length - 1;
    const active = cols()[last]!;
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const listing = cache.get(active.path);
      if (!listing) return;
      const q = filter().trim().toLowerCase();
      const dirs = q ? listing.dirs.filter((n) => n.toLowerCase().includes(q)) : listing.dirs;
      if (!dirs.length) return;
      const at = active.sel ? dirs.indexOf(active.sel) : -1;
      const next = e.key === 'ArrowDown' ? Math.min(dirs.length - 1, at + 1) : Math.max(0, at < 0 ? 0 : at - 1);
      selectIn(last, dirs[next]!);
      return;
    }
    if ((e.key === 'ArrowRight' || e.key === 'Enter') && active.sel) {
      e.preventDefault();
      openFolder(last, active.sel);
      return;
    }
    if (e.key === 'ArrowLeft') {
      e.preventDefault();
      goBack();
      return;
    }
    // A printable character goes to the filter.
    if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) focusFilter(e.key);
  };

  // ── lifecycle ──
  let dialog: HTMLDivElement | undefined;
  let scroller: HTMLDivElement | undefined;
  onMount(() => {
    queueMicrotask(() => dialog?.focus({ preventScroll: true }));
  });
  // Opening a column scrolls the Miller area to its end.
  createEffect(
    on(
      () => cols().length,
      () => requestAnimationFrame(() => scroller && (scroller.scrollLeft = scroller.scrollWidth)),
    ),
  );
  onCleanup(() => {
    cache.clear();
    inflight.clear();
  });

  const useSystemDialog = () => {
    void pickFolderNative(selected()).then((picked) => picked && props.onUse(picked));
  };

  return (
    <div class="sm-overlay fb-overlay" role="presentation" onClick={props.onClose}>
      <div
        class="sm-dialog fb-dialog"
        role="dialog"
        aria-modal="true"
        aria-label="Choose a workspace"
        tabindex="-1"
        ref={(el) => (dialog = el)}
        onKeyDown={(e: KeyboardEvent) => {
          if (wrapTab(e, e.currentTarget as HTMLElement)) e.stopPropagation();
          onKeys(e);
        }}
        onClick={(e) => e.stopPropagation()}
        data-testid="workspace-browser"
      >
        <div class="sm-head fb-head">
          <span class="sm-title">
            <h2>Choose a workspace</h2>
            <Show when={pcName()}>
              <span class="sm-sub">{pcName()}</span>
            </Show>
          </span>
          <button type="button" class="ghost-icon-button" aria-label="Close" onClick={props.onClose} data-testid="workspace-browser-close">
            <X size={20} />
          </button>
        </div>
        <div class="fb-pathbar">
          <Show
            when={editing()}
            fallback={
              <div class="fb-crumbs" onClick={(e) => e.target === e.currentTarget && startEdit()} data-testid="workspace-crumbs" title="Click to type a path">
                <For each={crumbs()}>
                  {(c, i) => (
                    <>
                      <Show when={i() > 0}>
                        <ChevronRight size={12} class="fb-crumb-sep" />
                      </Show>
                      <button type="button" class="fb-crumb" aria-current={i() === crumbs().length - 1 ? 'location' : undefined} onClick={() => jumpTo(c.path)}>
                        {c.label}
                      </button>
                    </>
                  )}
                </For>
              </div>
            }
          >
            <div class="fb-pathedit">
              <input
                type="text"
                class="fb-path-input"
                aria-label="Folder path"
                value={draft()}
                spellcheck={false}
                onInput={(e) => setDraft(e.currentTarget.value)}
                onKeyDown={(e: KeyboardEvent) => {
                  if (e.key === 'Enter') void submitEdit();
                  else if (e.key === 'Escape') {
                    e.stopPropagation();
                    exitEdit();
                  }
                }}
                data-testid="workspace-path-edit"
              />
              <Show when={pathError()}>
                <span class="fb-path-error" role="alert" data-testid="workspace-path-error">
                  {pathError()}
                </span>
              </Show>
            </div>
          </Show>
          <input
            type="search"
            class="fb-filter"
            placeholder="Filter"
            aria-label="Filter folders"
            value={filter()}
            onInput={(e) => setFilter(e.currentTarget.value)}
            ref={(el) => (filterEl = el)}
            data-testid="workspace-filter"
          />
        </div>
        <div class="fb-body">
          <nav class="fb-places" aria-label="Places">
            <div class="fb-places-label">Places</div>
            <Show when={props.home}>
              <button type="button" class="fb-place" onClick={() => jumpTo(props.home!)} data-testid="workspace-place-home">
                <Home size={14} />
                <span>Home</span>
              </button>
            </Show>
            <For each={props.roots.filter((r) => r !== props.home)}>
              {(root) => (
                <button type="button" class="fb-place" onClick={() => jumpTo(root)} title={root}>
                  <Folder size={14} />
                  <span class="fb-place-name">{folderName(root) || root}</span>
                </button>
              )}
            </For>
            <Show when={props.recentItems.length}>
              <div class="fb-places-label">Recent</div>
              <For each={props.recentItems}>
                {(item) => (
                  <button type="button" class="fb-place" onClick={() => jumpTo(item.path)} title={item.path} data-testid="workspace-place-recent">
                    <Clock size={14} />
                    <span class="fb-place-name">{item.label}</span>
                  </button>
                )}
              </For>
            </Show>
          </nav>
          <div class="fb-columns" ref={(el) => (scroller = el)} data-testid="workspace-columns">
            {/* Index (not For): a selection change rewrites the column objects, and the
              columns must not remount (their listings would flash back to skeletons). */}
            <Index each={cols()}>
              {(col, i) => (
                <BrowserColumn
                  col={col()}
                  at={i}
                  filter={filter()}
                  cache={cache}
                  hasGit={hasGitOf}
                  fetchListing={fetchListing}
                  onSelect={(name) => openFolder(i, name)}
                  onUse={useSelected}
                />
              )}
            </Index>
          </div>
        </div>
        <div class="fb-foot">
          <span class="fb-foot-path" title={selected()}>
            <bdi dir="ltr">{short(selected())}</bdi>
          </span>
          <Show when={hasGitOf(selected())}>
            <span class="fb-git" title="This folder is a git repository" data-testid="workspace-git-pill">
              git
            </span>
          </Show>
          <span class="fb-foot-grow" />
          <Show when={props.ownPc}>
            <button type="button" class="ws-btn" onClick={useSystemDialog} data-testid="workspace-sysdlg">
              System dialog…
            </button>
          </Show>
          <button type="button" class="ws-btn" onClick={props.onClose}>
            Cancel
          </button>
          <button type="button" class="ws-btn ws-btn-primary" onClick={useSelected} data-testid="workspace-use">
            <Folder size={14} />
            Use this folder
          </button>
        </div>
      </div>
    </div>
  );
}

/** One Miller column: the subfolders of `col.path`, skeletons while loading. */
function BrowserColumn(props: {
  col: Col;
  at: number;
  filter: string;
  cache: Map<string, FolderListing>;
  hasGit: (path: string) => boolean;
  fetchListing: (path: string) => Promise<FolderListing>;
  /** Click: select the row and open it into the next column. */
  onSelect: (name: string) => void;
  /** Double-click: use the folder. */
  onUse: () => void;
}) {
  const [listing] = createResource(() => props.col.path, (path) => props.fetchListing(path));
  const dirs = createMemo(() => {
    const all = listing()?.dirs ?? [];
    const q = props.filter.trim().toLowerCase();
    return q ? all.filter((n) => n.toLowerCase().includes(q)) : all;
  });
  // Keep the selected row visible in its column.
  createEffect(
    on(
      () => props.col.sel,
      (sel) =>
        sel &&
        requestAnimationFrame(() => {
          // jsdom (tests) has no scrollIntoView.
          document.querySelector(`[data-col="${props.at}"] [data-sel]`)?.scrollIntoView?.({ block: 'nearest' });
        }),
    ),
  );
  return (
    <div class="fb-col" data-col={props.at} data-testid={`workspace-col-${props.at}`}>
      <Show
        when={!listing.loading}
        fallback={
          <div class="fb-col-skel" role="status" aria-label="Loading folders">
            {Array.from({ length: 5 }, () => (
              <span class="fb-skel-row" />
            ))}
          </div>
        }
      >
        <Show when={!listing()?.error} fallback={<div class="fb-note">{listing()?.error}</div>}>
          <Show when={dirs().length} fallback={<div class="fb-note">{props.filter.trim() ? `No folder matches “${props.filter.trim()}”` : 'No folders inside'}</div>}>
            <For each={dirs()}>
              {(name) => {
                const path = join(props.col.path, name);
                return (
                  <button type="button" class="fb-row" data-sel={props.col.sel === name ? '' : undefined} title={name} onClick={() => props.onSelect(name)} onDblClick={props.onUse}>
                    <Folder size={14} class="fb-row-ic" />
                    <span class="fb-row-name">{name}</span>
                    <Show when={props.hasGit(path)}>
                      <span class="fb-git" title="This folder is a git repository">
                        git
                      </span>
                    </Show>
                    <ChevronRight size={13} class="fb-row-chev" />
                  </button>
                );
              }}
            </For>
          </Show>
        </Show>
      </Show>
    </div>
  );
}
