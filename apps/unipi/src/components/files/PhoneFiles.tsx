/**
 * Files on the phone (phone spec §3.11, PH-02/PH-10/PH-24/PH-37): a tab page,
 * not the desktop tree. The root is **Places** (Home, the PC's projects, a
 * collapsed "Other locations"); a folder is a pushed page (`/files?path=…`,
 * `root=` = the place it was entered from) with a breadcrumb bar and 56/72 px
 * sans rows (name, "12 KB · Yesterday"), an always-visible ⋯ per row and a
 * long-press action sheet. A file opens the viewer at `/files/edit` (a
 * workspace layer above this page); media opens the previewer. Every menu is
 * an action sheet (above every layer). Back walks folders up through the
 * route (A's `phoneParent`), never history.
 */
import { createEffect, createMemo, createResource, createSignal, For, Show, type Component } from 'solid-js';
import { useLocation, useNavigate, useSearchParams } from '@solidjs/router';
import {
  Archive, ChevronDown, ChevronRight, Copy, Download, Ellipsis, Eye, File, FileCode2, FileImage, FileJson, FilePlus2, FileText,
  FileVideo, Folder, FolderOpen, FolderPlus, HardDrive, House, MessageSquarePlus, PencilLine, Search, Trash2, Upload,
} from 'lucide-solid';
import { ActionSheet, Button, IconButton, ListGroup, ListRow, PageScaffold, RowIcon, Sheet, confirmSheet, haptic, type ActionItem } from '../phone';
import { basename, dirname, fsDelete, fsImport, fsList, fsMkdir, fsRename, fsRoots, fsStat, fsWrite, joinPath, pushRecent, type FsEntry } from '../../lib/fs';
import { breadcrumbs, entrySubtitle, fileFamily, folderView, placesFrom, tildify, type FileFamily, type FilesSort } from '../../lib/files/phone';
import { setEditorTarget } from '../../lib/editorTarget';
import { consumeFilesTarget } from '../../lib/filesTarget';
import { isDirectPreview } from '../../lib/media/previewer';
import { downloadWithNotify, progressLabel, transferMessage } from '../../lib/filesTransfer';
import { openPreviewer } from '../../lib/media/previewerState';
import { uploadPicked } from '../../lib/blob';
import { copyText } from '../../lib/board/share';
import { queueMention } from '../../lib/chat/mention';
import { openChatHref, openNewChat } from '../../lib/session/open';
import type { ChatTarget } from '../../lib/session/list';
import { ChatPickSheet } from '../board/ChatPickSheet';
import { hubs } from '../../lib/session/hub';
import { isHomeCwd } from '../../lib/session/selectors';
import { useHostsOptional } from '../../hosts/context';
import { LOCAL_HOST } from '../../lib/host/client';
import { NoHostState, OfflineState } from '../EmptyState';
import { setAddHostOpen } from '../Pairing';
import { FileSearch } from './FileSearch';
import { toasts } from '../../lib/toasts';
import './phoneFiles.css';

type Icon = Component<{ size?: number }>;

const FAMILY_ICON: Record<FileFamily, Icon> = {
  folder: Folder, code: FileCode2, text: FileText, image: FileImage, media: FileVideo, archive: Archive, pdf: FileText, data: FileJson, file: File,
};

const HIDDEN_KEY = 'unipi.files.showHidden';
const SORT_KEY = 'unipi.files.sort';
const readBool = (k: string) => {
  try { return localStorage.getItem(k) === '1'; } catch { return false; }
};
const readSort = (): FilesSort => {
  try { return localStorage.getItem(SORT_KEY) === 'modified' ? 'modified' : 'name'; } catch { return 'name'; }
};
const [showHidden, setShowHiddenRaw] = createSignal(readBool(HIDDEN_KEY));
const [sortBy, setSortByRaw] = createSignal<FilesSort>(readSort());
const setShowHidden = (v: boolean) => {
  setShowHiddenRaw(v);
  try { localStorage.setItem(HIDDEN_KEY, v ? '1' : '0'); } catch { /* not remembered */ }
};
const setSortBy = (v: FilesSort) => {
  setSortByRaw(v);
  try { localStorage.setItem(SORT_KEY, v); } catch { /* not remembered */ }
};

function toast(text: string) {
  toasts().push(text);
}

/** `/files?path=<dir>&root=<place>` for a folder. */
export function folderHref(path: string, root?: string): string {
  return `/files?path=${encodeURIComponent(path)}${root ? `&root=${encodeURIComponent(root)}` : ''}`;
}

export function PhoneFiles() {
  const hosts = useHostsOptional();
  const navigate = useNavigate();
  const location = useLocation();
  const [search] = useSearchParams<{ path?: string; root?: string; from?: string }>();
  const hostId = () => hosts?.selectedId() ?? hosts?.hosts()[0]?.id ?? LOCAL_HOST;
  const noHost = () => !!hosts && hosts.hosts().length === 0 && !hosts.loading();

  // "Files here" from a chat (B's ⋯ → Files) names a folder: open it as a pushed folder.
  createEffect(() => {
    const target = consumeFilesTarget();
    if (!target) return;
    if (hosts && target.hostId !== hostId() && hosts.hosts().some((h) => h.id === target.hostId)) hosts.selectHost(target.hostId);
    navigate(folderHref(target.path, target.path), { replace: location.pathname === '/files' && !search.path });
  });

  // On the viewer route the page underneath shows the folder the file came from.
  const path = () => (location.pathname === '/files/edit' ? search.from : search.path) || undefined;

  return (
    <Show when={!noHost()} fallback={<PageScaffold title="Files" testId="files-page"><NoHostState what="its files" onAdd={() => setAddHostOpen(true)} /></PageScaffold>}>
      <Show when={path()} keyed fallback={<PlacesPage hostId={hostId()} />}>
        {(p) => <FolderPage hostId={hostId()} path={p} root={search.root || undefined} />}
      </Show>
    </Show>
  );
}

/** The ⋯ menu items every Files page shares (New / Upload / hidden files / sort). */
function filesMenu(opts: { dir?: string; onNew: (kind: 'file' | 'dir') => void; onUpload: () => void }): ActionItem[][] {
  const create: ActionItem[] = opts.dir
    ? [
        { id: 'new-file', label: 'New file', icon: FilePlus2, onSelect: () => opts.onNew('file') },
        { id: 'new-dir', label: 'New folder', icon: FolderPlus, onSelect: () => opts.onNew('dir') },
        { id: 'upload', label: 'Upload from phone', icon: Upload, onSelect: opts.onUpload },
      ]
    : [];
  const view: ActionItem[] = [
    { id: 'hidden', label: 'Show hidden files', icon: Eye, checked: showHidden(), onSelect: () => setShowHidden(!showHidden()) },
    { id: 'sort-name', label: 'Sort by name', checked: sortBy() === 'name', onSelect: () => setSortBy('name') },
    { id: 'sort-modified', label: 'Sort by date modified', checked: sortBy() === 'modified', onSelect: () => setSortBy('modified') },
  ];
  return create.length ? [create, view] : [view];
}

/** Root: Places (§3.11, PH-24) — Home, projects, Other locations (collapsed). */
function PlacesPage(props: { hostId: string }) {
  const navigate = useNavigate();
  const hosts = useHostsOptional();
  const [roots, { refetch }] = createResource(() => props.hostId, (id) => fsRoots(id));
  const [otherOpen, setOtherOpen] = createSignal(false);
  const [menuOpen, setMenuOpen] = createSignal(false);
  const [searching, setSearching] = createSignal(false);
  const projects = createMemo(() => {
    const cards = Object.values(hubs().ensure(props.hostId).state.cards).filter((c) => c.source !== 'cron' && c.cwd && !isHomeCwd(c.cwd));
    cards.sort((a, b) => (b.last_activity ?? b.updated_at ?? 0) - (a.last_activity ?? a.updated_at ?? 0));
    return cards.map((c) => c.cwd as string);
  });
  const places = () => {
    const r = roots();
    return r?.ok ? placesFrom(r.value.roots, projects()) : undefined;
  };
  const pcName = () => hosts?.hosts().find((h) => h.id === props.hostId)?.name;
  const open = (p: string) => navigate(folderHref(p, p));
  return (
    <PageScaffold
      title="Files"
      testId="files-page"
      onRefresh={async () => void (await refetch())}
      tools={
        <>
          <IconButton icon={Search} label="Search files" onClick={() => setSearching(true)} testId="files-search" />
          <IconButton icon={Ellipsis} label="More files actions" onClick={() => setMenuOpen(true)} testId="files-tools" />
        </>
      }
    >
      <div class="phfiles" data-testid="files-places">
        <Show when={roots()?.ok === false}>
          <OfflineState compact pc={pcName()} message={roots()?.ok === false ? (roots() as { error: { message: string } }).error.message : undefined} onRetry={() => void refetch()} />
        </Show>
        <Show when={places()}>
          {(pl) => (
            <>
              <ListGroup>
                <Show when={pl().home}>
                  {(home) => <ListRow title="Home" subtitle={tildify(home().path, home().path)} leading={<RowIcon icon={House} tint="accent" />} chevron onClick={() => open(home().path)} testId="files-place-home" />}
                </Show>
              </ListGroup>
              <Show when={pl().projects.length > 0}>
                <ListGroup title="Projects">
                  <For each={pl().projects}>
                    {(p) => <ListRow title={p.label} subtitle={p.detail} leading={<RowIcon icon={FolderOpen} />} chevron onClick={() => open(p.path)} testId="files-place-project" />}
                  </For>
                </ListGroup>
              </Show>
              <Show when={pl().other.length > 0}>
                <ListGroup>
                  <ListRow
                    title="Other locations"
                    subtitle={pl().other.map((o) => o.path).join(' · ')}
                    leading={<RowIcon icon={HardDrive} />}
                    trailing={otherOpen() ? <ChevronDown size={20} /> : <ChevronRight size={20} />}
                    onClick={() => setOtherOpen(!otherOpen())}
                    testId="files-other-toggle"
                  />
                  <Show when={otherOpen()}>
                    <For each={pl().other}>
                      {(o) => <ListRow title={o.path} leading={<RowIcon icon={HardDrive} />} chevron onClick={() => open(o.path)} testId="files-place-other" />}
                    </For>
                  </Show>
                </ListGroup>
              </Show>
            </>
          )}
        </Show>
      </div>
      <ActionSheet open={menuOpen()} onClose={() => setMenuOpen(false)} title="Files" sections={filesMenu({ onNew: () => undefined, onUpload: () => undefined })} testId="files-tools-sheet" />
      <Show when={searching()}>
        <SearchSheet hostId={props.hostId} root={places()?.home?.path} onClose={() => setSearching(false)} />
      </Show>
    </PageScaffold>
  );
}

/** Search across a folder (name / content) in a full sheet; a hit opens the viewer. */
function SearchSheet(props: { hostId: string; root: string | undefined; onClose: () => void }) {
  const navigate = useNavigate();
  return (
    <Sheet open onClose={props.onClose} label="Search files" title="Search files" detents={['full']} testId="files-search-sheet">
      <FileSearch
        hostId={props.hostId}
        root={() => props.root}
        onClose={props.onClose}
        onOpen={(path, line) => {
          props.onClose();
          openFileAt(props.hostId, path, navigate, line);
        }}
      />
    </Sheet>
  );
}

/** Opens a file: media → previewer layer, everything else → the viewer route (read-only first). */
function openFileAt(hostId: string, path: string, navigate: (to: string) => void, line?: number, siblings?: string[]) {
  pushRecent(hostId, path);
  if (isDirectPreview(path)) {
    const list = siblings?.length ? siblings : [path];
    openPreviewer({ kind: 'list', hostId, paths: list, index: Math.max(0, list.indexOf(path)) });
    return;
  }
  setEditorTarget({ hostId, path, line });
  navigate(`/files/edit?from=${encodeURIComponent(dirname(path))}`);
}

/** A folder (pushed page): breadcrumb bar + list (§3.11). */
function FolderPage(props: { hostId: string; path: string; root?: string }) {
  const navigate = useNavigate();
  const [listing, { refetch }] = createResource(
    () => ({ host: props.hostId, path: props.path, hidden: showHidden() }),
    (k) => fsList(k.host, k.path, k.hidden),
  );
  const [roots] = createResource(() => props.hostId, (id) => fsRoots(id));
  const home = () => {
    const r = roots();
    return r?.ok ? r.value.roots[0] : undefined;
  };
  const entries = () => {
    const l = listing();
    return l?.ok ? folderView(l.value.entries, { hidden: showHidden(), sort: sortBy() }) : [];
  };
  const crumbs = () => breadcrumbs(props.path, props.root, home());
  const title = () => {
    const h = home();
    if (h && props.path === h.replace(/\/+$/, '')) return 'Home';
    return basename(props.path) || props.path;
  };
  const parentLabel = () => tildify(dirname(props.path), home());
  /** The header Back: the parent folder inside the place it was entered from, else Places. */
  const backHref = () => {
    const at = props.path.replace(/\/+$/, '') || '/';
    if (at === '/' || (props.root && at === props.root.replace(/\/+$/, ''))) return '/files';
    const up = dirname(at);
    return up && up !== at ? folderHref(up, props.root) : '/files';
  };
  const [menuFor, setMenuFor] = createSignal<FsEntry>();
  const [toolsOpen, setToolsOpen] = createSignal(false);
  const [nameAsk, setNameAsk] = createSignal<{ kind: 'rename' | 'new-file' | 'new-dir'; entry?: FsEntry }>();
  const [searching, setSearching] = createSignal(false);
  let uploadInput: HTMLInputElement | undefined;
  const now = Date.now();
  const pathOf = (e: FsEntry) => joinPath(props.path, e.name);
  const previewable = () => entries().filter((e) => e.kind !== 'dir' && isDirectPreview(e.name)).map(pathOf);

  const activate = (e: FsEntry) => {
    if (e.kind === 'dir') navigate(folderHref(pathOf(e), props.root ?? props.path));
    else openFileAt(props.hostId, pathOf(e), navigate, undefined, previewable());
  };

  async function del(e: FsEntry) {
    const ok = await confirmSheet({ title: `Delete ${e.name}?`, body: e.kind === 'dir' ? 'The folder and everything in it is deleted from the PC.' : 'The file is deleted from the PC.', confirm: 'Delete', tone: 'danger' });
    if (!ok) return;
    const result = await fsDelete(props.hostId, pathOf(e), true);
    if (result.ok) {
      toast(`Deleted ${e.name}`);
      void refetch();
    } else toast(result.error.message);
  }

  function mention(e: FsEntry) {
    const path = pathOf(e);
    queueMention(`draft:${props.hostId}`, `@${path} `);
    toast(`Added @${e.name} to a new chat`);
    openNewChat(props.hostId, e.kind === 'dir' ? path : props.path);
  }

  // "Add to chat" (owner spec 2026-11-03): an `@path` chip in a chat the user picks.
  const [addFor, setAddFor] = createSignal<FsEntry>();
  function addPicked(target: ChatTarget) {
    const e = addFor();
    setAddFor(undefined);
    if (!e) return;
    queueMention(`${target.hostId}:${target.session}`, `@${pathOf(e)} `);
    toast(`Added @${e.name} to the chat`);
    navigate(openChatHref(target.hostId, target.session));
  }

  const rowMenu = (e: FsEntry): ActionItem[][] => [
    [
      { id: 'open', label: e.kind === 'dir' ? 'Open' : isDirectPreview(e.name) ? 'Preview' : 'Open', icon: e.kind === 'dir' ? FolderOpen : Eye, onSelect: () => activate(e) },
      ...(e.kind !== 'dir' && isDirectPreview(e.name)
        ? [{ id: 'text', label: 'Open as text', icon: FileText, onSelect: () => { setEditorTarget({ hostId: props.hostId, path: pathOf(e), asText: true }); navigate(`/files/edit?from=${encodeURIComponent(props.path)}`); } }]
        : []),
      ...(e.kind !== 'dir'
        ? [{
            id: 'download',
            label: 'Download',
            icon: Download,
            onSelect: () => {
              toast(progressLabel('Downloading', undefined));
              void downloadWithNotify(props.hostId, pathOf(e), (p) => toast(progressLabel('Downloading', p))).then((r) => toast(transferMessage(r, e.name, false)));
            },
          }]
        : []),
      { id: 'add', label: 'Add to chat…', icon: MessageSquarePlus, onSelect: () => setAddFor(e) },
      { id: 'chat', label: 'Open in a new chat', icon: MessageSquarePlus, onSelect: () => mention(e) },
    ],
    [
      { id: 'rename', label: 'Rename', icon: PencilLine, onSelect: () => setNameAsk({ kind: 'rename', entry: e }) },
      { id: 'copy', label: 'Copy path', icon: Copy, onSelect: () => void copyText(pathOf(e)).then((ok) => toast(ok ? 'Path copied' : 'Could not copy')) },
      { id: 'delete', label: 'Delete', icon: Trash2, tone: 'danger', onSelect: () => void del(e) },
    ],
  ];

  async function submitName(name: string) {
    const ask = nameAsk();
    setNameAsk(undefined);
    const trimmed = name.trim();
    if (!ask || !trimmed) return;
    if (trimmed.includes('/')) return toast("Names can't contain /");
    if (ask.kind === 'rename' && ask.entry) {
      if (trimmed === ask.entry.name) return;
      const result = await fsRename(props.hostId, pathOf(ask.entry), joinPath(props.path, trimmed));
      if (result.ok) toast(`Renamed to ${trimmed}`);
      else toast(result.error.message);
    } else if (ask.kind === 'new-dir') {
      const result = await fsMkdir(props.hostId, joinPath(props.path, trimmed));
      if (!result.ok) toast(result.error.message);
    } else {
      const path = joinPath(props.path, trimmed);
      if ((await fsStat(props.hostId, path)).ok) return toast(`${trimmed} already exists`);
      const result = await fsWrite(props.hostId, path, '', undefined, true);
      if (!result.ok) return toast(result.error.message);
      void refetch();
      return openFileAt(props.hostId, path, navigate);
    }
    void refetch();
  }

  async function upload(files: File[]) {
    let saved = 0;
    for (const file of files) {
      try {
        const put = await uploadPicked(props.hostId, file);
        let name = file.name;
        for (let n = 1; n <= 20; n += 1) {
          const result = await fsImport(props.hostId, put.sha256, joinPath(props.path, name));
          if (result.ok) {
            saved += 1;
            break;
          }
          if (result.error.code !== 'E_CONFLICT') {
            toast(result.error.message);
            break;
          }
          const dot = file.name.lastIndexOf('.');
          name = dot > 0 ? `${file.name.slice(0, dot)} (${n})${file.name.slice(dot)}` : `${file.name} (${n})`;
        }
      } catch (error) {
        toast((error as { message?: string })?.message ?? `Could not upload ${file.name}`);
      }
    }
    if (saved) {
      haptic('success');
      toast(saved > 1 ? `Uploaded ${saved} files` : 'Uploaded');
    }
    void refetch();
  }

  return (
    <PageScaffold
      title={title()}
      subtitle={<>{parentLabel()}</>}
      back={backHref()}
      testId="files-folder"
      onRefresh={async () => void (await refetch())}
      tools={
        <>
          <IconButton icon={Search} label="Search in this folder" onClick={() => setSearching(true)} testId="files-search" />
          <IconButton icon={Ellipsis} label="More files actions" onClick={() => setToolsOpen(true)} testId="files-tools" />
        </>
      }
    >
      <div class="phfiles" data-testid="files-folder-list">
        <nav class="phfiles-crumbs" aria-label="Folder path">
          <For each={crumbs()}>
            {(c, i) => (
              <>
                <Show when={i() > 0}>
                  <ChevronRight size={14} class="phfiles-crumb-sep" aria-hidden="true" />
                </Show>
                <button
                  type="button"
                  class="phfiles-crumb"
                  aria-current={i() === crumbs().length - 1 ? 'page' : undefined}
                  onClick={() => i() < crumbs().length - 1 && navigate(folderHref(c.path, props.root), { replace: true })}
                >
                  {c.label}
                </button>
              </>
            )}
          </For>
        </nav>
        <Show when={listing.loading && !listing.latest}>
          <ListGroup>
            <For each={[0, 1, 2, 3, 4]}>{() => <div class="phfiles-skeleton" aria-hidden="true" />}</For>
          </ListGroup>
        </Show>
        <Show when={listing()?.ok === false}>
          <OfflineState compact message={(listing() as { error: { message: string } }).error.message} onRetry={() => void refetch()} />
        </Show>
        <Show when={listing()?.ok && entries().length === 0}>
          <p class="phfiles-empty" role="status">{showHidden() ? 'This folder is empty.' : 'No files here. Hidden files are not shown.'}</p>
        </Show>
        <ListGroup>
          <For each={entries()}>
            {(e) => (
              <ListRow
                title={<span class="phfiles-name">{e.name}</span>}
                subtitle={entrySubtitle(e, now)}
                leading={<RowIcon icon={FAMILY_ICON[fileFamily(e)]} tint={e.kind === 'dir' ? 'accent' : 'neutral'} />}
                trailing={<IconButton icon={Ellipsis} label={`Actions for ${e.name}`} onClick={(ev) => (ev.stopPropagation(), setMenuFor(e))} testId="files-row-menu" />}
                dim={e.ignored}
                onClick={() => activate(e)}
                onLongPress={() => setMenuFor(e)}
                testId="files-row"
              />
            )}
          </For>
        </ListGroup>
      </div>
      <input ref={uploadInput} type="file" multiple class="hidden" aria-hidden="true" onChange={(ev) => {
        const files = Array.from(ev.currentTarget.files ?? []);
        ev.currentTarget.value = '';
        if (files.length) void upload(files);
      }} />
      <ActionSheet
        open={!!menuFor()}
        onClose={() => setMenuFor(undefined)}
        title={menuFor()?.name}
        subtitle={menuFor() ? entrySubtitle(menuFor()!, now) : undefined}
        sections={menuFor() ? rowMenu(menuFor()!) : []}
        testId="files-row-sheet"
      />
      <ActionSheet
        open={toolsOpen()}
        onClose={() => setToolsOpen(false)}
        title={title()}
        sections={filesMenu({ dir: props.path, onNew: (kind) => setNameAsk({ kind: kind === 'dir' ? 'new-dir' : 'new-file' }), onUpload: () => uploadInput?.click() })}
        testId="files-tools-sheet"
      />
      <Show when={addFor()} keyed>
        {(e) => (
          <ChatPickSheet
            open
            hostId={props.hostId}
            root={props.root}
            label={`Add ${e.name} to…`}
            onPick={addPicked}
            onNewChat={() => {
              setAddFor(undefined);
              mention(e);
            }}
            onClose={() => setAddFor(undefined)}
          />
        )}
      </Show>
      <Show when={nameAsk()} keyed>
        {(ask) => (
          <NameSheet
            label={ask.kind === 'rename' ? 'Rename' : ask.kind === 'new-dir' ? 'New folder' : 'New file'}
            initial={ask.entry?.name ?? ''}
            action={ask.kind === 'rename' ? 'Rename' : 'Create'}
            onDone={(name) => void submitName(name)}
            onClose={() => setNameAsk(undefined)}
          />
        )}
      </Show>
      <Show when={searching()}>
        <SearchSheet hostId={props.hostId} root={props.path} onClose={() => setSearching(false)} />
      </Show>
    </PageScaffold>
  );
}

/** Name prompt (rename / new file / new folder): a form sheet, Cancel + primary both 48 (PH-39). */
function NameSheet(props: { label: string; initial: string; action: string; onDone: (name: string) => void; onClose: () => void }) {
  const [name, setName] = createSignal(props.initial);
  return (
    <Sheet
      open
      onClose={props.onClose}
      label={props.label}
      title={props.label}
      guarded={() => name() !== props.initial}
      testId="files-name-sheet"
      footer={
        <div class="phfiles-sheet-actions">
          <Button kind="secondary" block onClick={props.onClose}>Cancel</Button>
          <Button kind="primary" block disabled={!name().trim()} onClick={() => props.onDone(name())} testId="files-name-save">{props.action}</Button>
        </div>
      }
    >
      <input
        class="phfiles-name-input"
        data-testid="files-name-input"
        aria-label="Name"
        value={name()}
        autocapitalize="off"
        autocorrect="off"
        spellcheck={false}
        ref={(el) => setTimeout(() => {
          el.focus();
          const dot = props.initial.lastIndexOf('.');
          el.setSelectionRange(0, dot > 0 ? dot : props.initial.length);
        }, 60)}
        onInput={(e) => setName(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            e.preventDefault();
            props.onDone(name());
          }
        }}
      />
    </Sheet>
  );
}
