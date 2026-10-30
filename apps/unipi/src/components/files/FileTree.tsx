/**
 * The Files tree (WP-A20, Z-101): lazy, virtualized tree over `fs_list`
 * with compact folders, dotfiles, git badges + changed filter, gitignore
 * dim, fuzzy/content search, row menu (Rename / Delete / Copy path /
 * Mention in chat), new file/folder, upload by drop, and refresh-on-focus
 * watchers (`lib/files/watch.ts`). Opening a file is delegated up: text →
 * the editor area, media → the preview tab (desktop) or layer (phone).
 */
import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from 'solid-js';
import {
  Copy,
  Ellipsis,
  Eye,
  FilePenLine,
  FilePlus2,
  FileText,
  FolderPlus,
  GitBranch,
  MessageSquarePlus,
  Search,
  Trash2,
  UnfoldVertical,
  Upload,
} from 'lucide-solid';
import { useNavigate } from '@solidjs/router';
import {
  basename,
  dirname,
  fsDelete,
  fsImport,
  fsList,
  fsMkdir,
  fsRename,
  fsStat,
  fsWrite,
  joinPath,
  pushRecent,
} from '../../lib/fs';
import { uploadPicked } from '../../lib/blob';
import { queueMention } from '../../lib/chat/mention';
import { openChat } from '../../lib/session/open';
import type { ChatTarget } from '../../lib/session/list';
import { useHostsOptional } from '../../hosts/context';
import { createDirWatch } from '../../lib/files/watch';
import { emptyModel, flatten, flattenChanged, neededListings, type TreeEntry, type TreeModel } from '../../lib/files/tree';
import { dragHasFiles, filesKey, inTextField } from '../../lib/desktop';
import { desktopShell } from '../../lib/layout';
import { openPreviewer } from '../../lib/media/previewerState';
import { isDirectPreview } from '../../lib/media/previewer';
import { hapticTick } from '../../lib/haptics';
import { useSettings } from '../../settings/context';
import { BoardSheet } from '../board/BoardSheet';
import { ChatPickSheet } from '../board/ChatPickSheet';
import { AnchoredMenu } from '../board/AnchoredMenu';
import { EmptyState } from '../EmptyState';
import { TreeRowView } from './TreeRow';
import { FileSearch } from './FileSearch';
import { VIRTUAL_THRESHOLD, VirtualWindow, useVirtual } from '../../ui/virtual';

const ROW_H = 26;

type MenuAction = 'open' | 'preview' | 'edit-text' | 'rename' | 'delete' | 'copy' | 'mention' | 'new-file' | 'new-dir';

const menuItem = (id: MenuAction, label: string) => ({ id, label });

export function FileTree(props: {
  hostId: string;
  /** The tree's starting folder (the project), or the host roots when it is `~`. */
  root: string | undefined;
  roots: () => string[] | undefined;
  /** Text file opened in the editor area. */
  openFile: (path: string, opts?: { line?: number; asText?: boolean }) => void;
  onPreview: (path: string, siblings: string[]) => void;
}) {
  const navigate = useNavigate();
  const [settings] = useSettings();
  const hosts = useHostsOptional();
  const haptic = () => void hapticTick(settings.input.haptics);

  const [model, setModel] = createSignal<TreeModel>(emptyModel());
  const [changedOnly, setChangedOnly] = createSignal(false);
  const [search, setSearch] = createSignal(false);
  const [selected, setSelected] = createSignal<string>();
  const [menu, setMenu] = createSignal<{ path: string; kind: string; anchor: DOMRect } | undefined>();
  const [toolsMenu, setToolsMenu] = createSignal<DOMRect | undefined>();
  const [renameTarget, setRenameTarget] = createSignal<{ path: string; name: string } | undefined>();
  const [deleteTarget, setDeleteTarget] = createSignal<{ path: string; name: string } | undefined>();
  const [newIn, setNewIn] = createSignal<{ dir: string; kind: 'file' | 'dir' } | undefined>();
  const [toast, setToast] = createSignal<string | null>(null);
  const [dropOver, setDropOver] = createSignal(false);
  let uploadInput: HTMLInputElement | undefined;
  let scroller: HTMLDivElement | undefined;

  const watch = createDirWatch({ body: () => scroller?.closest('.pane-tab-body') as HTMLElement | undefined });
  const showToast = (text: string) => {
    setToast(text);
    setTimeout(() => setToast((t) => (t === text ? null : t)), 2500);
  };

  // Roots: the project folder, or the host roots for the `~` scope.
  createEffect(() => {
    const roots = props.root && props.root !== '~' ? [props.root] : props.roots();
    setModel((m) => (m.roots.join('\u{0}') === (roots ?? []).join('\u{0}') ? m : { ...m, roots: roots ?? [] }));
  });

  // Lazy listings: fetch what the view needs, once per dir.
  createEffect(() => {
    for (const path of neededListings(model())) {
      void load(path);
    }
  });
  async function load(path: string, silent = false) {
    setModel((m) => ({ ...m, dirs: new Map(m.dirs).set(path, { status: 'loading' }) }));
    const result = await fsList(props.hostId, path, true);
    setModel((m) => {
      const dirs = new Map(m.dirs);
      if (!result.ok) dirs.set(path, { status: 'error', error: result.error.message });
      else dirs.set(path, { status: 'ready', entries: result.value.entries as TreeEntry[], more: result.value.more });
      return { ...m, dirs };
    });
    if (!result.ok && !silent) showToast(result.error.message);
  }

  // Watchers (Z-101): refresh on focus while any listing is loaded.
  createEffect(() => {
    if (!watch.dirty()) return;
    watch.clear();
    for (const [path, state] of model().dirs) if (state.status === 'ready') void load(path, true);
  });

  const rows = createMemo(() => (changedOnly() ? flattenChanged(model()) : flatten(model())));
  // §13.4: virtualize only past the threshold — small trees render flat
  // (and measurable-less environments like jsdom still show every row).
  const virtual = () => rows().length > VIRTUAL_THRESHOLD;
  const virt = useVirtual({ count: () => rows().length, scroller: () => scroller, estimate: () => ROW_H, key: (i) => rows()[i]!.path, overscan: 10, fixed: true });

  function toggle(row: { path: string; expanded?: boolean; kind: string }) {
    if (row.kind === 'file' || row.kind === 'symlink') return;
    setModel((m) => {
      const expanded = new Set(m.expanded);
      if (expanded.has(row.path)) expanded.delete(row.path);
      else expanded.add(row.path);
      return { ...m, expanded };
    });
  }

  function activate(row: { path: string; kind: string; expanded?: boolean }) {
    haptic();
    setSelected(row.path);
    if (row.kind === 'dir' || row.kind === 'root') return toggle(row);
    openPath(row.path);
  }

  function openPath(path: string, opts?: { line?: number; asText?: boolean }) {
    pushRecent(props.hostId, path);
    if (isDirectPreview(path) && !opts?.asText) {
      const siblings = previewSiblings(path);
      props.onPreview(path, siblings);
      return;
    }
    props.openFile(path, opts);
  }

  /** A folder's previewable siblings (UNI-168 media paging), the tapped file included. */
  function previewSiblings(path: string): string[] {
    const dir = dirname(path);
    const state = model().dirs.get(dir);
    if (state?.status !== 'ready') return [path];
    return (state.entries ?? []).filter((e) => e.kind !== 'dir' && isDirectPreview(e.name)).map((e) => joinPath(dir, e.name));
  }

  // ── Row menu (Rename / Delete / Copy path / Mention in chat, …) ────────
  function runAction(id: MenuAction, row: { path: string; kind: string }) {
    setMenu(undefined);
    switch (id) {
      case 'open':
        return openPath(row.path);
      case 'preview':
        return openPath(row.path);
      case 'edit-text':
        return props.openFile(row.path, { asText: true });
      case 'rename':
        return setRenameTarget({ path: row.path, name: basename(row.path) });
      case 'delete':
        return setDeleteTarget({ path: row.path, name: basename(row.path) });
      case 'copy':
        return void navigator.clipboard?.writeText(row.path).then(
          () => showToast('Path copied'),
          () => showToast('Could not copy'),
        );
      case 'mention':
        return setMentionFor(row.path);
      case 'new-file':
        return setNewIn({ dir: row.path, kind: 'file' });
      case 'new-dir':
        return setNewIn({ dir: row.path, kind: 'dir' });
    }
  }

  const [mentionFor, setMentionFor] = createSignal<string | undefined>();
  function mentionPicked(target: ChatTarget) {
    const path = mentionFor();
    setMentionFor(undefined);
    if (!path) return;
    // The composer (A12) drains this mailbox into an @path chip; until then
    // the draft text is queued for the chat the user picked.
    queueMention(`${target.hostId}:${target.session}`, `@${path} `);
    showToast('Added @path to the chat draft');
    const pcName = hosts?.hosts().find((h) => h.id === target.hostId)?.name ?? 'PC';
    if (desktopShell()) navigate(`/s/${encodeURIComponent(target.hostId)}/${encodeURIComponent(target.session)}`);
    else openChat(target, pcName);
  }

  // ── Mutations (rename / delete / new / upload) ─────────────────────────
  async function doRename(name: string) {
    const target = renameTarget();
    setRenameTarget(undefined);
    if (!target || !name.trim() || name.trim() === target.name) return;
    const result = await fsRename(props.hostId, target.path, joinPath(dirname(target.path), name.trim()));
    if (result.ok) {
      showToast(`Renamed to ${name.trim()}`);
      watch.mutated();
    } else showToast(result.error.message);
  }

  async function doDelete() {
    const target = deleteTarget();
    setDeleteTarget(undefined);
    if (!target) return;
    const result = await fsDelete(props.hostId, target.path, true);
    if (result.ok) {
      showToast(`Deleted ${target.name}`);
      if (selected() === target.path) setSelected(undefined);
      watch.mutated();
    } else showToast(result.error.message);
  }

  async function doNew(name: string, kind: 'file' | 'dir') {
    const target = newIn();
    setNewIn(undefined);
    const trimmed = name.trim();
    if (!target || !trimmed) return;
    if (trimmed.includes('/')) {
      showToast("Names can't contain /");
      return;
    }
    const path = joinPath(target.dir, trimmed);
    if (kind === 'dir') {
      const result = await fsMkdir(props.hostId, path);
      if (result.ok) {
        setModel((m) => ({ ...m, expanded: new Set(m.expanded).add(target.dir) }));
        watch.mutated();
      } else showToast(result.error.message);
      return;
    }
    const existing = await fsStat(props.hostId, path);
    if (existing.ok) {
      showToast(`${trimmed} already exists`);
      return;
    }
    const result = await fsWrite(props.hostId, path, '', undefined, true);
    if (result.ok) {
      watch.mutated();
      openPath(path);
    } else showToast(result.error.message);
  }

  // Upload (Z-101): drop OS files on the tree, or the toolbar button.
  const withSuffix = (name: string, n: number): string => {
    const dot = name.lastIndexOf('.');
    if (dot <= 0) return `${name} (${n})`;
    return `${name.slice(0, dot)} (${n})${name.slice(dot)}`;
  };
  async function importWithRetry(sha256: string, dir: string, name: string): Promise<string | undefined> {
    for (let n = 0; n <= 20; n += 1) {
      const candidate = n === 0 ? name : withSuffix(name, n);
      const result = await fsImport(props.hostId, sha256, joinPath(dir, candidate));
      if (result.ok) return candidate;
      if (result.error.code !== 'E_CONFLICT') {
        showToast(result.error.message);
        return undefined;
      }
    }
    showToast(`Too many files named like ${name}`);
    return undefined;
  }
  const uploadDir = () => model().roots[0];
  async function doUpload(files: File[]) {
    const dir = uploadDir();
    if (!dir || files.length === 0) return;
    let lastName: string | undefined;
    for (const file of files) {
      setToast(`Uploading ${file.name}…`);
      try {
        const put = await uploadPicked(props.hostId, file);
        const saved = await importWithRetry(put.sha256, dir, file.name);
        if (saved) lastName = saved;
      } catch (error) {
        showToast((error as { message?: string })?.message ?? `Could not upload ${file.name}`);
      }
    }
    watch.mutated();
    showToast(lastName ? (files.length > 1 ? `Uploaded ${files.length} files` : `Uploaded ${lastName}`) : 'Upload failed');
  }

  // Keyboard (desktop): arrows / Enter / Backspace-up / F2 / Delete over the virtual list.
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!desktopShell() || inTextField(e.target) || e.defaultPrevented) return;
      if (menu() || toolsMenu() || renameTarget() || deleteTarget() || newIn() || search() || mentionFor()) return;
      if (!(e.target instanceof Element && (e.target === document.body || e.target.closest('.files-tab')))) return;
      const list = rows();
      const at = list.findIndex((r) => r.path === selected());
      const action = filesKey(e, at, list.length);
      if (!action) return;
      e.preventDefault();
      if (action.kind === 'move') {
        const row = list[action.to]!;
        setSelected(row.path);
        scroller?.querySelector(`[data-test-row="${CSS.escape(row.path)}"]`)?.scrollIntoView({ block: 'nearest' });
        return;
      }
      const row = at >= 0 ? list[at] : undefined;
      if (!row) return;
      if (action.kind === 'open') activate(row);
      else if (action.kind === 'rename') setRenameTarget({ path: row.path, name: basename(row.path) });
      else if (action.kind === 'delete') setDeleteTarget({ path: row.path, name: basename(row.path) });
      else if (action.kind === 'up') {
        const parent = dirname(row.path);
        if (model().roots.includes(parent)) setSelected(parent);
        else if (parent && parent !== row.path) setSelected(parent);
      }
    };
    window.addEventListener('keydown', onKey);
    onCleanup(() => window.removeEventListener('keydown', onKey));
  });

  return (
    <div
      class="ftree"
      data-testid="files-tree"
      onDragOver={(e) => {
        if (!dragHasFiles(e.dataTransfer)) return;
        e.preventDefault();
        setDropOver(true);
      }}
      onDragLeave={() => setDropOver(false)}
      onDrop={(e) => {
        if (!dragHasFiles(e.dataTransfer)) return;
        e.preventDefault();
        setDropOver(false);
        const files = Array.from(e.dataTransfer?.files ?? []);
        if (files.length) void doUpload(files);
      }}
    >
      <div class="ftree-bar" role="toolbar" aria-label="Files tools">
        <button type="button" class={`ftree-tool${changedOnly() ? ' on' : ''}`} aria-pressed={changedOnly()} aria-label="Only changed files" data-tip="Only changed files" data-testid="files-changed" onClick={() => setChangedOnly((v) => !v)}>
          <GitBranch size={15} />
        </button>
        <button type="button" class="ftree-tool" aria-label="Search files" data-tip="Search files (name and content)" data-testid="files-search" onClick={() => setSearch(true)}>
          <Search size={15} />
        </button>
        <span class="ftree-bar-fill" />
        <button type="button" class="ftree-tool" aria-label="More files actions" data-tip="New file, new folder, upload" data-testid="files-tools" onClick={(e) => setToolsMenu((e.currentTarget as HTMLElement).getBoundingClientRect())}>
          <Ellipsis size={16} />
        </button>
      </div>

      <div class="ftree-scroll" ref={scroller} role="tree" aria-label="Project files">
        <Show when={rows().length > 0} fallback={
          <EmptyState
            compact
            icon={<FolderPlus size={22} />}
            title={model().roots.length === 0 ? 'No folders to show' : changedOnly() ? 'No changed files' : 'Nothing here'}
            body={changedOnly() ? 'Edit a file and it will show up in this view.' : undefined}
            testId="files-empty"
            action={model().roots.length === 0 ? undefined : (
              <button type="button" class="btn btn-secondary btn-sm" onClick={() => setChangedOnly(false)}>Show all files</button>
            )}
          />
        }>
          <Show
            when={virtual()}
            fallback={
              <For each={rows()}>
                {(row) => <TreeRowView row={row} selected={selected() === row.path} onActivate={activate} onMenu={(r, anchor) => setMenu({ path: r.path, kind: r.kind, anchor })} />}
              </For>
            }
          >
            <VirtualWindow virt={virt} fixed>
              {(index) => {
                const row = rows()[index]!;
                return <TreeRowView row={row} selected={selected() === row.path} onActivate={activate} onMenu={(r, anchor) => setMenu({ path: r.path, kind: r.kind, anchor })} />;
              }}
            </VirtualWindow>
          </Show>
        </Show>
      </div>

      <Show when={dropOver()}>
        <div class="ftree-drop" data-testid="files-drop" aria-hidden="true">
          <Upload size={22} />
          <p>Drop to upload to {basename(uploadDir() ?? '') || 'this project'}</p>
        </div>
      </Show>

      <Show when={toast()}>
        <div class="files-toast" role="status">{toast()}</div>
      </Show>

      <input
        ref={uploadInput}
        type="file"
        multiple
        class="hidden"
        aria-hidden="true"
        tabIndex={-1}
        onChange={(e) => {
          const files = Array.from(e.currentTarget.files ?? []);
          e.currentTarget.value = '';
          if (files.length) void doUpload(files);
        }}
      />

      <Show when={search()}>
        <FileSearch
          hostId={props.hostId}
          root={() => (props.root && props.root !== '~' ? props.root : props.roots()?.[0])}
          onClose={() => setSearch(false)}
          onOpen={(path, line) => {
            setSearch(false);
            openPath(path, { line });
          }}
        />
      </Show>

      <Show when={menu()}>
        {(m) => (
          <AnchoredMenu anchor={m().anchor} width={230} label={`Actions for ${basename(m().path)}`} onDismiss={() => setMenu(undefined)}>
            <MenuItems kind={m().kind} path={m().path} onRun={(id) => runAction(id, { path: m().path, kind: m().kind })} />
          </AnchoredMenu>
        )}
      </Show>

      <Show when={toolsMenu()}>
        {(anchor) => (
          <AnchoredMenu anchor={anchor()} width={230} alignEnd label="Files actions" onDismiss={() => setToolsMenu(undefined)}>
            <div class="board-menu-list" role="none">
              <ToolItem icon={<FilePlus2 size={15} />} label="New file" onClick={() => { setToolsMenu(undefined); setNewIn({ dir: uploadDir() ?? '', kind: 'file' }); }} />
              <ToolItem icon={<FolderPlus size={15} />} label="New folder" onClick={() => { setToolsMenu(undefined); setNewIn({ dir: uploadDir() ?? '', kind: 'dir' }); }} />
              <ToolItem icon={<Upload size={15} />} label="Upload files…" onClick={() => { setToolsMenu(undefined); uploadInput?.click(); }} />
              <ToolItem icon={<UnfoldVertical size={15} />} label="Collapse all" onClick={() => { setToolsMenu(undefined); setModel((m) => ({ ...m, expanded: new Set() })); }} />
            </div>
          </AnchoredMenu>
        )}
      </Show>

      <Show when={renameTarget()}>
        {(t) => <NameSheet label="Rename" initial={t().name} action="Rename" onDone={doRename} onClose={() => setRenameTarget(undefined)} />}
      </Show>
      <Show when={newIn()}>
        {(t) => <NameSheet label={t().kind === 'file' ? 'New file' : 'New folder'} initial="" action="Create" onDone={(name) => void doNew(name, t().kind)} onClose={() => setNewIn(undefined)} />}
      </Show>

      <BoardSheet open={!!deleteTarget()} onClose={() => setDeleteTarget(undefined)} label="Delete" testId="files-delete-sheet">
        <p class="p-2 text-text-muted">Delete {deleteTarget()?.name}? This can't be undone.</p>
        <div class="files-sheet-actions">
          <button type="button" class="files-sheet-btn" onClick={() => setDeleteTarget(undefined)}>Cancel</button>
          <button type="button" class="files-sheet-btn danger" data-testid="files-delete-confirm" onClick={() => void doDelete()}>Delete</button>
        </div>
      </BoardSheet>

      <Show when={mentionFor()}>
        {(path) => (
          <ChatPickSheet
            open
            hostId={props.hostId}
            root={props.root}
            label={`Mention ${basename(path())} in…`}
            onPick={mentionPicked}
            onClose={() => setMentionFor(undefined)}
          />
        )}
      </Show>
    </div>
  );
}

function MenuItems(props: { kind: string; path: string; onRun: (id: MenuAction) => void }) {
  const isDir = () => props.kind === 'dir' || props.kind === 'root';
  const media = () => !isDir() && isDirectPreview(props.path);
  return (
    <div class="board-menu-list" role="none">
      <Show when={isDir()}>
        <ToolItem icon={<FilePlus2 size={15} />} label="New file" onClick={() => props.onRun('new-file')} />
        <ToolItem icon={<FolderPlus size={15} />} label="New folder" onClick={() => props.onRun('new-dir')} />
      </Show>
      <Show when={media()}>
        <ToolItem icon={<Eye size={15} />} label="Preview" onClick={() => props.onRun('preview')} />
      </Show>
      <Show when={!isDir() && !media()}>
        <ToolItem icon={<FileText size={15} />} label="Edit as text" onClick={() => props.onRun('edit-text')} />
      </Show>
      <ToolItem icon={<FilePenLine size={15} />} label="Rename…" onClick={() => props.onRun('rename')} />
      <ToolItem icon={<Copy size={15} />} label="Copy path" onClick={() => props.onRun('copy')} />
      <ToolItem icon={<MessageSquarePlus size={15} />} label="Mention in chat" testId="files-mention" onClick={() => props.onRun('mention')} />
      <ToolItem icon={<Trash2 size={15} />} label="Delete…" danger onClick={() => props.onRun('delete')} />
    </div>
  );
}

function ToolItem(props: { icon: JSX.Element; label: string; onClick: () => void; testId?: string; danger?: boolean }) {
  return (
    <button type="button" role="menuitem" class="board-menu-item" data-testid={props.testId} onClick={props.onClick} data-danger={props.danger ? '' : undefined}>
      <span class="board-menu-icon" aria-hidden="true">{props.icon}</span>
      <span>{props.label}</span>
    </button>
  );
}

/** A small name prompt (rename / new file / new folder). */
function NameSheet(props: { label: string; initial: string; action: string; onDone: (name: string) => void; onClose: () => void }) {
  const [name, setName] = createSignal(props.initial);
  return (
    <BoardSheet open onClose={props.onClose} label={props.label} testId="files-name-sheet">
      <input
        class="files-name-input"
        data-testid="files-name-input"
        value={name()}
        autofocus
        onInput={(e) => setName(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            e.preventDefault();
            props.onDone(name());
          }
          if (e.key === 'Escape') props.onClose();
        }}
      />
      <div class="files-sheet-actions">
        <button type="button" class="files-sheet-btn" onClick={props.onClose}>Cancel</button>
        <button type="button" class="files-sheet-btn primary" data-testid="files-name-save" onClick={() => props.onDone(name())}>{props.action}</button>
      </div>
    </BoardSheet>
  );
}

/** Chat picker for "Mention in chat" reuses `ChatPickSheet`. */
export { FileSearch };
