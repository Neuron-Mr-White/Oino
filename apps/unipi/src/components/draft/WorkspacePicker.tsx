/**
 * The draft's workspace row (Z-19): where the new chat runs. A chip — folder
 * icon, name, chevron — opening a menu of this PC's recent session folders,
 * and "Open folder…" (the native dialog on this computer, a folder browser over
 * `fs_list` on a remote PC / the phone). "No workspace" runs the session in a
 * private scratch folder the host makes for that chat (never the user's home,
 * where the agent could touch everything). A PC chip sits beside it when more
 * than one PC is paired.
 */
import { createMemo, createResource, createSignal, For, Show } from 'solid-js';
import { ArrowLeft, Check, ChevronDown, ChevronRight, Folder, FolderLock, FolderOpen, FolderSearch, Monitor } from 'lucide-solid';
import { AnchoredMenu } from '../board/AnchoredMenu';
import { layout } from '../../lib/layout';
import { Button, Sheet } from '../phone';
import { fsList } from '../../lib/fs';
import { shortPath, folderName } from '../../lib/session/newSession';
import { isOwnPcFile, pickFolderNative } from '../../lib/openLocal';
import { isHomeCwd } from '../../lib/session/selectors';

/** What "No workspace" means, in the chip tooltip and the menu row. */
export const NO_WORKSPACE_DETAIL = 'Runs in a private scratch folder';

/** Scratch locations are not projects: hidden from Recent unless the user opened them explicitly. */
const isTempPath = (p: string) => /^\/(var\/)?tmp(\/|$)/.test(p);
const OPENED_KEY = 'unipi.workspace.opened';
const explicitlyOpened = (): string[] => {
  try {
    return JSON.parse(localStorage.getItem(OPENED_KEY) ?? '[]') as string[];
  } catch {
    return [];
  }
};
const rememberOpened = (path: string) => {
  try {
    localStorage.setItem(OPENED_KEY, JSON.stringify([path, ...explicitlyOpened().filter((p) => p !== path)].slice(0, 20)));
  } catch {
    // best effort
  }
};

type Item = { path: string; label: string; detail: string };

/** `/home/u/a/b` → `/home/u/a` (never above `/`). */
export const parentOf = (path: string): string => {
  const trimmed = path.replace(/\/+$/, '');
  const at = trimmed.lastIndexOf('/');
  return at <= 0 ? '/' : trimmed.slice(0, at);
};

/** The user's home among a host's `fs_roots` (it is listed first; a Unix or Windows home shape). */
export function homeOf(roots: readonly string[]): string | undefined {
  return roots.find((r) => /^\/home\/[^/]+\/?$|^\/Users\/[^/]+\/?$|^\/root\/?$|^[A-Za-z]:\\Users\\[^\\]+\\?$/.test(r));
}

export function WorkspacePicker(props: {
  hostId: string;
  /** The chosen folder (`undefined` = No workspace). */
  value: string | undefined;
  onChange: (path: string | undefined) => void;
  recent: readonly string[];
  roots: readonly string[];
  home?: string;
  /** Hide the "No workspace" choice (e.g. Cron jobs require an explicit folder). */
  allowNone?: boolean;
  placeholder?: string;
  disabled?: boolean;
}) {
  const [anchor, setAnchor] = createSignal<DOMRect | undefined>();
  /** The folder browser's current path (`undefined` = the list view). */
  const [browse, setBrowse] = createSignal<string | undefined>();
  const [filter, setFilter] = createSignal('');
  const openBrowse = (dir: string) => {
    setFilter('');
    setBrowse(dir);
  };
  const close = () => {
    setAnchor(undefined);
    setBrowse(undefined);
    setFilter('');
  };
  const pick = (path: string | undefined) => {
    props.onChange(path);
    close();
  };
  const label = () => (props.value ? folderName(props.value) : (props.placeholder ?? 'No workspace'));
  const short = (p: string) => shortPath(p, props.home);

  const recentItems = createMemo<Item[]>(() => {
    const opened = explicitlyOpened();
    return props.recent
      // Home and the scratch folders are "No workspace", not projects.
      .filter((p) => p !== props.home && !isHomeCwd(p) && (!isTempPath(p) || opened.includes(p)))
      .slice(0, 6)
      .map((path) => ({ path, label: folderName(path), detail: short(path) }));
  });

  /** "Open folder…": the native dialog on this computer, the in-app browser for a remote PC / the phone. */
  const openFolder = async () => {
    const start = props.value || props.home || props.roots[0] || '/';
    if (isOwnPcFile(props.hostId)) {
      try {
        const picked = await pickFolderNative(start);
        if (picked) {
          rememberOpened(picked);
          pick(picked);
        } else close();
        return;
      } catch {
        // fall through to the in-app browser
      }
    }
    openBrowse(start);
  };

  const [listing] = createResource(browse, async (path) => {
    const res = await fsList(props.hostId, path);
    // D-20: a folder the PC doesn't let the app see reads calmly (no raw scope message, no CLI hint).
    if (!res.ok) return { error: /outside the allowed|allowed folders|E_SCOPE|scope/i.test(String(res.error.message)) ? 'Unipi can’t open this folder on the PC. Go back to a folder it lists.' : String(res.error.message), dirs: [] as string[] };
    return {
      error: undefined,
      dirs: res.value.entries
        .filter((e) => e.kind === 'dir' && !e.name.startsWith('.'))
        .map((e) => e.name)
        .sort((a, b) => a.localeCompare(b)),
    };
  });
  const phone = () => !layout().desktop();
  /** Phone breadcrumb (PH-30): "/" or "~" first, then each folder — tapping one goes up to it. */
  const crumbs = (dir: string): Array<{ label: string; path: string }> => {
    const home = props.home;
    const inHome = !!home && (dir === home || dir.startsWith(`${home}/`));
    const base = inHome ? home! : '';
    const rest = (inHome ? dir.slice(home!.length) : dir).split('/').filter(Boolean);
    const out = [{ label: inHome ? '~' : '/', path: inHome ? home! : '/' }];
    let at = base;
    for (const part of rest) {
      at = `${at}/${part}`;
      out.push({ label: part, path: at });
    }
    return out;
  };
  const join = (dir: string, name: string) => `${dir.replace(/\/+$/, '')}/${name}`;

  const row = (item: Item) => (
    <button
      type="button"
      role="menuitemradio"
      aria-checked={props.value === item.path}
      class="ws-row"
      title={item.path}
      onClick={() => pick(item.path)}
    >
      <Folder size={15} class="ws-row-ic" />
      <span class="ws-row-text">
        <span class="ws-row-label">{item.label}</span>
        <Show when={item.detail}>
          <span class="ws-row-detail">{item.detail}</span>
        </Show>
      </span>
      <Show when={props.value === item.path}>
        <Check size={15} class="ws-row-check" />
      </Show>
    </button>
  );

  const menuBody = () => (
            <div class="ws-menu" data-testid="workspace-menu">
              <Show
                when={browse()}
                fallback={
                  <>
                    <Show when={props.allowNone !== false}>
                      <button type="button" role="menuitemradio" aria-checked={!props.value} class="ws-row" onClick={() => pick(undefined)} data-testid="workspace-none">
                        <FolderLock size={15} class="ws-row-ic" />
                        <span class="ws-row-text">
                          <span class="ws-row-label">No workspace</span>
                          <span class="ws-row-detail">{NO_WORKSPACE_DETAIL}</span>
                        </span>
                        <Show when={!props.value}>
                          <Check size={15} class="ws-row-check" />
                        </Show>
                      </button>
                    </Show>
                    <Show when={recentItems().length}>
                      <div class="ws-label">Recent</div>
                      <For each={recentItems()}>{row}</For>
                    </Show>
                    <div class="ws-sep" role="separator" />
                    <button
                      type="button"
                      role="menuitem"
                      class="ws-row ws-row-open"
                      onClick={() => void openFolder()}
                      data-testid="workspace-open"
                    >
                      <FolderSearch size={15} class="ws-row-ic" />
                      <span class="ws-row-text">
                        <span class="ws-row-label">Open folder…</span>
                      </span>
                    </button>
                  </>
                }
              >
                {(dir) => {
                  const filteredDirs = () => {
                    const all = listing()?.dirs ?? [];
                    const q = filter().trim().toLowerCase();
                    return q ? all.filter((n) => n.toLowerCase().includes(q)) : all;
                  };
                  return (
                  <div class="ws-browse" data-testid="workspace-browser">
                    <Show
                      when={phone()}
                      fallback={
                        <div class="ws-browse-head">
                          <button
                            type="button"
                            class="ws-browse-up"
                            aria-label="Up one folder"
                            disabled={dir() === '/'}
                            onClick={() => openBrowse(parentOf(dir()))}
                          >
                            <ArrowLeft size={15} />
                          </button>
                          <span class="ws-browse-path" title={dir()}>
                            {short(dir())}
                          </span>
                        </div>
                      }
                    >
                      <nav class="ws-crumbs" aria-label="Folder path" data-testid="workspace-crumbs">
                        <button type="button" class="ws-crumb ws-crumb-back" onClick={() => (setFilter(''), setBrowse(undefined))}>
                          <ArrowLeft size={18} /> Places
                        </button>
                        <For each={crumbs(dir())}>
                          {(c, i) => (
                            <button type="button" class="ws-crumb" aria-current={i() === crumbs(dir()).length - 1 ? 'location' : undefined} onClick={() => openBrowse(c.path)}>
                              {c.label}
                            </button>
                          )}
                        </For>
                      </nav>
                    </Show>
                    <Show when={(listing()?.dirs.length ?? 0) > 6}>
                      <input
                        type="search"
                        class="dm-search"
                        placeholder="Filter folders…"
                        aria-label="Filter folders"
                        value={filter()}
                        onInput={(e) => setFilter(e.currentTarget.value)}
                        data-testid="workspace-filter"
                      />
                    </Show>
                    <div class="ws-browse-list">
                      <Show when={!listing.loading} fallback={<div class="ws-browse-note">Loading…</div>}>
                        <Show when={!listing()?.error} fallback={<div class="ws-browse-note">{listing()?.error}</div>}>
                          <Show when={filteredDirs().length} fallback={<div class="ws-browse-note">{filter().trim() ? `No folder matches “${filter().trim()}”` : 'No folders inside'}</div>}>
                            <For each={filteredDirs()}>
                              {(name) => (
                                <button type="button" class="ws-row ws-row-dir" onClick={() => openBrowse(join(dir(), name))}>
                                  <Folder size={15} class="ws-row-ic" />
                                  <span class="ws-row-text">
                                    <span class="ws-row-label">{name}</span>
                                  </span>
                                </button>
                              )}
                            </For>
                          </Show>
                        </Show>
                      </Show>
                    </div>
                    <div class="ws-browse-foot" hidden={phone()}>
                      <button type="button" class="ws-btn" onClick={() => (setFilter(''), setBrowse(undefined))}>
                        Back
                      </button>
                      <button type="button" class="ws-btn ws-btn-primary" disabled={listing.loading || !!listing()?.error} onClick={() => (rememberOpened(dir()), pick(dir()))} data-testid="workspace-use">
                        <FolderOpen size={14} />
                        Use this folder
                      </button>
                    </div>
                  </div>
                  );
                }}
              </Show>
            </div>
  );

  return (
    <>
      <button
        type="button"
        class="ws-chip"
        aria-haspopup="menu"
        aria-expanded={!!anchor()}
        title={props.value ? `Workspace: ${props.value}` : `No workspace — ${NO_WORKSPACE_DETAIL.toLowerCase()}`}
        disabled={props.disabled}
        onClick={(e) => (anchor() ? close() : setAnchor((e.currentTarget as HTMLElement).getBoundingClientRect()))}
        data-testid="workspace-chip"
      >
        <Show when={props.value} fallback={<FolderLock size={14} class="ws-chip-ic" />}>
          <Folder size={14} class="ws-chip-ic" />
        </Show>
        {/* Phone (§3.6, PH-30): a 56 px "Workspace" row — label, value, chevron. */}
        <Show when={phone()}>
          <span class="ws-chip-key">Workspace</span>
        </Show>
        <span class="ws-chip-label">{label()}</span>
        <Show when={phone()} fallback={<ChevronDown size={13} class="ws-chip-chev" />}>
          <ChevronRight size={18} class="ws-chip-chev" />
        </Show>
      </button>
      <Show when={phone()}>
        <Sheet open={!!anchor()} onClose={close} label="Workspace" title="Workspace" detents={['full']} initialDetent="full" class="ws-sheet" testId="workspace-sheet"
          footer={
            browse() ? (
              <Button kind="primary" block disabled={listing.loading || !!listing()?.error} onClick={() => (rememberOpened(browse()!), pick(browse()!))} testId="workspace-use">
                Use this folder
              </Button>
            ) : undefined
          }
        >
          {menuBody()}
        </Sheet>
      </Show>
      <Show when={!phone() && anchor()}>
        {(rect) => (
          <AnchoredMenu anchor={rect()} width={320} label="Workspace" onDismiss={close}>
            {menuBody()}
          </AnchoredMenu>
        )}
      </Show>
    </>
  );
}

/** The PC chip (only when more than one PC is paired). */
export function HostPicker(props: {
  hostId: string;
  hosts: ReadonlyArray<{ id: string; name: string; transport?: string }>;
  onPick: (id: string) => void;
  disabled?: boolean;
}) {
  const [anchor, setAnchor] = createSignal<DOMRect | undefined>();
  const nameOf = (h: { name: string; transport?: string }) => (h.transport === 'local' ? 'This computer' : h.name);
  const current = () => props.hosts.find((h) => h.id === props.hostId);
  return (
    <>
      <button
        type="button"
        class="ws-chip"
        aria-haspopup="menu"
        aria-expanded={!!anchor()}
        title="Which PC runs the chat"
        disabled={props.disabled}
        onClick={(e) => (anchor() ? setAnchor(undefined) : setAnchor((e.currentTarget as HTMLElement).getBoundingClientRect()))}
        data-testid="host-chip"
      >
        <Monitor size={14} class="ws-chip-ic" />
        <span class="ws-chip-label">{current() ? nameOf(current()!) : 'PC'}</span>
        <ChevronDown size={13} class="ws-chip-chev" />
      </button>
      <Show when={anchor()}>
        {(rect) => (
          <AnchoredMenu anchor={rect()} width={240} label="PC" onDismiss={() => setAnchor(undefined)}>
            <div class="ws-menu">
              <For each={props.hosts}>
                {(h) => (
                  <button
                    type="button"
                    role="menuitemradio"
                    aria-checked={h.id === props.hostId}
                    class="ws-row"
                    onClick={() => {
                      setAnchor(undefined);
                      props.onPick(h.id);
                    }}
                  >
                    <Monitor size={15} class="ws-row-ic" />
                    <span class="ws-row-text">
                      <span class="ws-row-label">{nameOf(h)}</span>
                    </span>
                    <Show when={h.id === props.hostId}>
                      <Check size={15} class="ws-row-check" />
                    </Show>
                  </button>
                )}
              </For>
            </div>
          </AnchoredMenu>
        )}
      </Show>
    </>
  );
}
