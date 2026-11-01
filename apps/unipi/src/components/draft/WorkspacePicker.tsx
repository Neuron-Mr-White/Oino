/**
 * The draft's workspace row (Z-19): where the new chat runs. A chip — folder
 * icon, name, chevron — opening a menu of this PC's recent session folders,
 * its roots and "Open folder…" (a folder browser over `fs_list`; the app has
 * no native folder dialog). "No workspace" runs the session in the user's
 * home folder on that PC. A PC chip sits beside it when more than one PC is
 * paired.
 */
import { createMemo, createResource, createSignal, For, Show } from 'solid-js';
import { ArrowLeft, Check, ChevronDown, Folder, FolderOpen, FolderSearch, House, Monitor } from 'lucide-solid';
import { AnchoredMenu } from '../board/AnchoredMenu';
import { fsList } from '../../lib/fs';
import { shortPath, folderName } from '../../lib/session/newSession';

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
  disabled?: boolean;
}) {
  const [anchor, setAnchor] = createSignal<DOMRect | undefined>();
  /** The folder browser's current path (`undefined` = the list view). */
  const [browse, setBrowse] = createSignal<string | undefined>();
  const close = () => {
    setAnchor(undefined);
    setBrowse(undefined);
  };
  const pick = (path: string | undefined) => {
    props.onChange(path);
    close();
  };
  const label = () => (props.value ? folderName(props.value) : 'No workspace');
  const short = (p: string) => shortPath(p, props.home);

  const recentItems = createMemo<Item[]>(() =>
    props.recent.filter((p) => p !== props.home).slice(0, 6).map((path) => ({ path, label: folderName(path), detail: short(path) })),
  );
  const rootItems = createMemo<Item[]>(() =>
    props.roots
      .filter((p) => !props.recent.includes(p) || p === props.home)
      .slice(0, 6)
      .map((path) => ({ path, label: path === props.home ? 'Home' : short(path), detail: path === props.home ? short(path) : '' })),
  );

  const [listing] = createResource(browse, async (path) => {
    const res = await fsList(props.hostId, path);
    // D-20: a folder the PC doesn't let the app see reads calmly (no raw scope message, no CLI hint).
    if (!res.ok) return { error: /outside the allowed|allowed folders|E_SCOPE|scope/i.test(String(res.error.message)) ? 'UniPi can’t open this folder on the PC. Go back to a folder it lists.' : String(res.error.message), dirs: [] as string[] };
    return {
      error: undefined,
      dirs: res.value.entries
        .filter((e) => e.kind === 'dir' && !e.name.startsWith('.'))
        .map((e) => e.name)
        .sort((a, b) => a.localeCompare(b)),
    };
  });
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

  return (
    <>
      <button
        type="button"
        class="ws-chip"
        aria-haspopup="menu"
        aria-expanded={!!anchor()}
        title={props.value ? `Workspace: ${props.value}` : `No workspace — runs in ${props.home ? short(props.home) : 'your home folder'}`}
        disabled={props.disabled}
        onClick={(e) => (anchor() ? close() : setAnchor((e.currentTarget as HTMLElement).getBoundingClientRect()))}
        data-testid="workspace-chip"
      >
        <Show when={props.value} fallback={<House size={14} class="ws-chip-ic" />}>
          <Folder size={14} class="ws-chip-ic" />
        </Show>
        <span class="ws-chip-label">{label()}</span>
        <ChevronDown size={13} class="ws-chip-chev" />
      </button>
      <Show when={anchor()}>
        {(rect) => (
          <AnchoredMenu anchor={rect()} width={320} label="Workspace" onDismiss={close}>
            <div class="ws-menu" data-testid="workspace-menu">
              <Show
                when={browse()}
                fallback={
                  <>
                    <button type="button" role="menuitemradio" aria-checked={!props.value} class="ws-row" onClick={() => pick(undefined)} data-testid="workspace-none">
                      <House size={15} class="ws-row-ic" />
                      <span class="ws-row-text">
                        <span class="ws-row-label">No workspace</span>
                        <span class="ws-row-detail">Runs in {props.home ? short(props.home) : 'your home folder'}</span>
                      </span>
                      <Show when={!props.value}>
                        <Check size={15} class="ws-row-check" />
                      </Show>
                    </button>
                    <Show when={recentItems().length}>
                      <div class="ws-label">Recent</div>
                      <For each={recentItems()}>{row}</For>
                    </Show>
                    <Show when={rootItems().length}>
                      <div class="ws-label">On this PC</div>
                      <For each={rootItems()}>{row}</For>
                    </Show>
                    <div class="ws-sep" role="separator" />
                    <button
                      type="button"
                      role="menuitem"
                      class="ws-row ws-row-open"
                      onClick={() => setBrowse(props.value ?? props.home ?? props.roots[0] ?? '/')}
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
                {(dir) => (
                  <div class="ws-browse" data-testid="workspace-browser">
                    <div class="ws-browse-head">
                      <button
                        type="button"
                        class="ws-browse-up"
                        aria-label="Up one folder"
                        disabled={dir() === '/'}
                        onClick={() => setBrowse(parentOf(dir()))}
                      >
                        <ArrowLeft size={15} />
                      </button>
                      <span class="ws-browse-path" title={dir()}>
                        {short(dir())}
                      </span>
                    </div>
                    <div class="ws-browse-list">
                      <Show when={!listing.loading} fallback={<div class="ws-browse-note">Loading…</div>}>
                        <Show when={!listing()?.error} fallback={<div class="ws-browse-note">{listing()?.error}</div>}>
                          <Show when={listing()?.dirs.length} fallback={<div class="ws-browse-note">No folders inside</div>}>
                            <For each={listing()?.dirs ?? []}>
                              {(name) => (
                                <button type="button" class="ws-row ws-row-dir" onClick={() => setBrowse(join(dir(), name))}>
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
                    <div class="ws-browse-foot">
                      <button type="button" class="ws-btn" onClick={() => setBrowse(undefined)}>
                        Back
                      </button>
                      <button type="button" class="ws-btn ws-btn-primary" disabled={listing.loading || !!listing()?.error} onClick={() => pick(dir())} data-testid="workspace-use">
                        <FolderOpen size={14} />
                        Use this folder
                      </button>
                    </div>
                  </div>
                )}
              </Show>
            </div>
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
