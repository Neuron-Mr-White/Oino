/**
 * Files (UNI-155 "files — browser + editor"): host picker + roots,
 * breadcrumb directory listing (dirs first, size/mtime, git badges,
 * .gitignore dim), search (name/content), recent files, a "changed"
 * (git) filter, hidden toggle, pull to refresh. Long-press an entry for
 * Rename / Delete / Copy path / Mention in chat. Tapping a file opens
 * the editor (text) or the previewer (image/PDF/binary, via blob_get).
 */
import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import {
  ChevronRight,
  FolderOpen,
  GitBranch,
  Copy,
  Download,
  EllipsisVertical,
  ExternalLink,
  File as FileIcon,
  FileArchive,
  FileCode,
  FileImage,
  FilePlus,
  FileSpreadsheet,
  FileTerminal,
  FileText,
  FileVideoCamera,
  FolderPlus,
  Folder,
  History,
  MoreVertical,
  Check,
  ChevronDown,
  HardDrive,
  Home,
  MessageSquarePlus,
  Music,
  Pencil,
  Presentation,
  RefreshCw,
  Search,
  Trash2,
  Upload,
  X,
} from 'lucide-solid';
import { useHosts } from '../hosts/context';
import { PageHeader } from '../components/PageHeader';
import { PcChip } from '../components/PcChip';
import { EmptyState, isUnreachable, NoHostState, OfflineState } from '../components/EmptyState';
import { setAddHostOpen } from '../components/Pairing';
import { ErrorBanner } from '../components/ErrorBanner';
import { AnchoredMenu, MenuRow, MenuSeparator } from '../components/board/AnchoredMenu';
import { BoardSheet } from '../components/board/BoardSheet';
import { hapticTick } from '../lib/haptics';
import { useSettings } from '../settings/context';
import { consumeFilesTarget } from '../lib/filesTarget';
import {
  basename,
  dirname,
  joinPath,
  fsDelete,
  fsImport,
  fsList,
  fsMkdir,
  fsRename,
  fsRoots,
  fsSearch,
  fsStat,
  fsWrite,
  formatMtime,
  formatSize,
  pushRecent,
  readRecents,
  DEFAULT_UPLOAD_DIR,
  type FsEntry,
  type RecentFile,
} from '../lib/fs';
import { uploadPicked } from '../lib/blob';
import { setEditorTarget, switchEditorTarget } from '../lib/editorTarget';
import { dragHasFiles, filesKey, inTextField } from '../lib/desktop';
import { can, isTauriDesktop } from '../lib/platform';
import { describeLocal, readLocalFiles, registerDropZone, startNativeDrop } from '../lib/localFiles';
import { openPreviewer } from '../lib/media/previewerState';
import { isDirectPreview } from '../lib/media/previewer';
import { clearReturnToChat, hasMarkedReturnToChat, openChat, openChatState, resumeChatAfterTerminal } from '../lib/session/open';
import { activeRoot, crumbsFor, nextFilesBack, PLACES_INLINE_MAX, placesLayout, rememberFolder, rootLabel, startFolder } from '../lib/filesNav';
import { pushBack } from '../lib/back';
import { desktopShell, menuAnchor } from '../lib/layout';
import { downloadToDevice, openWithExternal, progressLabel, transferMessage, type TransferProgress } from '../lib/filesTransfer';
import { queueMention } from '../lib/chat/mention';
import { ChatPickSheet } from '../components/board/ChatPickSheet';
import type { ChatTarget } from '../lib/session/list';
import { aheadBehind, branchLabel, git, gitTarget, openGit, type GitRepoData } from '../lib/git';
import './files.css';

type SortedEntry = FsEntry & { path: string };

function sortRow(path: string): string {
  return `${path.toLowerCase()}`;
}

/** Typed row icons (UNI-166): every document family gets its own glyph. */
const IMAGE_EXTS = new Set(['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg', 'bmp', 'avif', 'ico']);
const VIDEO_EXTS = new Set(['mp4', 'mkv', 'webm', 'mov', 'avi', 'm4v']);
const AUDIO_EXTS = new Set(['mp3', 'ogg', 'wav', 'flac', 'm4a', 'opus', 'aac']);
const SHEET_EXTS = new Set(['xls', 'xlsx', 'ods', 'csv', 'tsv']);
const SLIDE_EXTS = new Set(['ppt', 'pptx', 'odp', 'key']);
const DOC_EXTS = new Set(['doc', 'docx', 'odt', 'rtf', 'pages', 'pdf']);
const ARCHIVE_EXTS = new Set(['zip', 'tar', 'gz', 'tgz', 'bz2', 'xz', '7z', 'rar', 'apk', 'jar', 'deb', 'rpm', 'iso']);
const CODE_EXTS = new Set([
  'js', 'jsx', 'mjs', 'cjs', 'ts', 'tsx', 'py', 'rs', 'sh', 'bash', 'zsh', 'yml', 'yaml', 'html', 'htm',
  'c', 'cc', 'cpp', 'h', 'hpp', 'json', 'css', 'scss', 'sql', 'go', 'java', 'cs', 'rb', 'toml', 'lua', 'vim',
]);

function entryIcon(entry: SortedEntry) {
  if (entry.kind === 'dir') return <Folder size={18} />;
  const ext = entry.name.split('.').pop()?.toLowerCase() ?? '';
  if (IMAGE_EXTS.has(ext)) return <FileImage size={18} />;
  if (VIDEO_EXTS.has(ext)) return <FileVideoCamera size={18} />;
  if (AUDIO_EXTS.has(ext)) return <Music size={18} />;
  if (SHEET_EXTS.has(ext)) return <FileSpreadsheet size={18} />;
  if (SLIDE_EXTS.has(ext)) return <Presentation size={18} />;
  if (ARCHIVE_EXTS.has(ext)) return <FileArchive size={18} />;
  if (CODE_EXTS.has(ext)) return <FileCode size={18} />;
  if (ext === 'log' || ext === 'logs') return <FileTerminal size={18} />;
  if (DOC_EXTS.has(ext) || ext === 'md' || ext === 'markdown' || ext === 'txt') return <FileText size={18} />;
  return <FileIcon size={18} />;
}

export default function Files() {
  const hosts = useHosts();
  const [settings] = useSettings();
  const navigate = useNavigate();
  const haptic = () => void hapticTick(settings.input.haptics);

  const [hostId, setHostId] = createSignal<string | undefined>(hosts.selectedId());
  // The hosts list resolves asynchronously (list_hosts IPC); once it does,
  // default to the selected host if we don't have one yet.
  createEffect(() => {
    if (!hostId() && hosts.selectedId()) setHostId(hosts.selectedId());
  });
  const [path, setPath] = createSignal<string | undefined>(undefined);
  const [roots, setRoots] = createSignal<string[]>([]);
  const [entries, setEntries] = createSignal<SortedEntry[]>([]);
  const [more, setMore] = createSignal(false);
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<{ message: string; retryable: boolean; code?: string; hint?: string } | undefined>(undefined);
  const [hidden, setHidden] = createSignal(false);
  const [changedOnly, setChangedOnly] = createSignal(false);
  const [refreshing, setRefreshing] = createSignal(false);

  const [searchOpen, setSearchOpen] = createSignal(false);
  const [recentsOpen, setRecentsOpen] = createSignal(false);
  const [hostSheetOpen, setHostSheetOpen] = createSignal(false);
  const [newFolderOpen, setNewFolderOpen] = createSignal(false);
  const [newFileOpen, setNewFileOpen] = createSignal(false);
  const [uploadOpen, setUploadOpen] = createSignal(false);
  const [uploadDest, setUploadDest] = createSignal<string>(DEFAULT_UPLOAD_DIR);
  const [uploading, setUploading] = createSignal<string | null>(null);
  let uploadInput: HTMLInputElement | undefined;
  const [menu, setMenu] = createSignal<{ entry: SortedEntry; anchor: DOMRect } | undefined>(undefined);
  const [mentionPick, setMentionPick] = createSignal<{ entry: SortedEntry; root: string } | undefined>(undefined);
  const [renameTarget, setRenameTarget] = createSignal<SortedEntry | undefined>(undefined);
  const [deleteTarget, setDeleteTarget] = createSignal<SortedEntry | undefined>(undefined);
  const [headerMenu, setHeaderMenu] = createSignal<DOMRect | undefined>(undefined);
  const [toast, setToast] = createSignal<string | null>(null);

  const showToast = (text: string) => {
    setToast(text);
    setTimeout(() => setToast((t) => (t === text ? null : t)), 2500);
  };

  // Consume a pending target (from "Files here" / `/open`) once, on mount.
  const pending = consumeFilesTarget();
  if (pending) {
    setHostId(pending.hostId);
    setPath(pending.path);
  }

  const hostName = (id: string | undefined) => hosts.hosts().find((h) => h.id === id)?.name ?? 'This PC';

  async function loadRoots() {
    const id = hostId();
    if (!id) return;
    const result = await fsRoots(id);
    // The host may have changed while this was in flight: a late answer for
    // the previous PC must not become this PC's Places (and its folders are
    // not this host's folders).
    if (hostId() !== id) return;
    if (result.ok) {
      setRoots(result.value.roots);
      if (!path()) setPath(startFolder(id, result.value.roots));
    } else {
      // Roots belong to the host that answered. Keep the previous PC's list
      // out of this one, and say why instead of showing someone else's
      // folders (a Windows host failing `fs_roots` used to show the Linux
      // PC's /home/… roots).
      setRoots([]);
      if (!path()) {
        // No folder to list yet: show why (offline PC) instead of an empty folder (UNI-218 F56).
        setError({ message: result.error.message, retryable: result.error.retryable, code: result.error.code, hint: result.error.hint });
      }
    }
  }

  async function load() {
    const id = hostId();
    const dir = path();
    if (!id || !dir) return;
    setLoading(true);
    setError(undefined);
    const result = await fsList(id, dir, hidden());
    setLoading(false);
    setRefreshing(false);
    if (result.ok) {
      // Remember where the user is, so the editor/previewer round-trip
      // (and the next app start) reopens this folder (UNI-167). Only on
      // success, so a deleted folder is never persisted.
      rememberFolder(id, dir);
      setEntries(
        result.value.entries
          .map((e) => ({ ...e, path: joinPath(dir, e.name) }))
          .sort((a, b) => {
            const aDir = a.kind === 'dir';
            const bDir = b.kind === 'dir';
            if (aDir !== bDir) return aDir ? -1 : 1;
            return sortRow(a.path).localeCompare(sortRow(b.path));
          }),
      );
      setMore(result.value.more);
    } else {
      setError({ message: result.error.message, retryable: result.error.retryable, code: result.error.code, hint: result.error.hint });
      setEntries([]);
    }
  }

  createEffect(on(hostId, () => {
    // Switching PCs drops the old Places immediately: the roots are per-host
    // and shown before the new host answers.
    setRoots([]);
    setPath(undefined);
    setEntries([]);
  }, { defer: true }));
  createEffect(() => {
    // Always (not only without a path): opened at a session's cwd ("Files
    // here"), Back must know that folder is a root and stop there, instead of
    // walking up into a parent outside the host's scope (e.g. /srv/x → /srv
    // when /srv isn't one of the host's extra roots → error).
    const id = hostId();
    if (id) void loadRoots();
  });
  createEffect(() => {
    hostId();
    path();
    hidden();
    void load();
  });

  // Android/header Back (UNI-167): up one folder while inside a host
  // tree; at a root, back to the chat Files was opened from ("Files
  // here"), else the app home.
  onMount(() =>
    onCleanup(
      pushBack(() => {
        const verdict = nextFilesBack({
          path: path(),
          roots: roots(),
          hasReturnChat: !!openChatState() && hasMarkedReturnToChat(),
        });
        if (verdict.action === 'parent') {
          setPath(verdict.to);
          return;
        }
        if (verdict.action === 'chat') {
          // Consumes the mark synchronously, before the cleanup below.
          resumeChatAfterTerminal();
          navigate('/');
          return;
        }
        navigate('/');
      }),
    ),
  );
  // Leaving Files any other way (bottom nav) drops a stale chat mark,
  // so a later terminal exit doesn't jump to an old chat. Harmless after
  // the back handler: it consumes the mark before this cleanup runs.
  onCleanup(() => clearReturnToChat());

  const visibleEntries = createMemo(() => {
    const list = entries();
    return changedOnly() ? list.filter((e) => e.git) : list;
  });

  // UNI-217: is this folder in a git repo? → branch chip in the toolbar (opens Git).
  // Best-effort: an older host (no `git` op) or any error just hides the chip.
  const [repo, setRepo] = createSignal<GitRepoData | undefined>(undefined);
  let repoSeq = 0;
  async function loadRepo() {
    const id = hostId();
    const dir = path();
    const seq = ++repoSeq;
    if (!id || !dir) return setRepo(undefined);
    const result = await git.repo(id, dir);
    if (seq !== repoSeq) return;
    setRepo(result.ok && result.value?.repo ? result.value : undefined);
  }
  createEffect(on([hostId, path], () => void loadRepo()));
  // Coming back from the Git view: branch / ahead-behind may have changed.
  createEffect(on(gitTarget, (t, prev) => { if (!t && prev) void loadRepo(); }, { defer: true }));
  const repoDirty = () => entries().some((e) => e.git);
  function openGitHere() {
    const id = hostId();
    const dir = path();
    if (!id || !dir) return;
    haptic();
    openGit({ hostId: id, path: dir, from: 'files' });
  }

  const crumbs = createMemo(() => crumbsFor(path(), roots()));
  // Places (UNI-220 roots): a few short ones inline in the toolbar, more in one menu pill (UNI-218).
  const places = createMemo(() => {
    const list = roots();
    if (list.length === 1 && path() && !activeRoot(path(), list)) return 'inline' as const;
    return placesLayout(list, desktopShell() ? PLACES_INLINE_MAX : 2);
  });
  const [placesMenu, setPlacesMenu] = createSignal<DOMRect | undefined>(undefined);

  function openEntry(entry: SortedEntry) {
    haptic();
    if (entry.kind === 'dir') {
      setPath(entry.path);
      return;
    }
    const id = hostId();
    if (!id) return;
    pushRecent(id, entry.path);
    // Media files page through their folder siblings (UNI-168); text
    // stays in the editor.
    if (isDirectPreview(entry.path)) {
      // Symlinks count too (dbx.png → usr/share/…): filtering to 'file'
      // only dropped the tapped link, indexOf gave -1 and the pager opened
      // the folder's first file instead (UNI-196).
      const siblings = visibleEntries().filter((e) => e.kind !== 'dir' && isDirectPreview(e.path)).map((e) => e.path);
      const index = siblings.indexOf(entry.path);
      openPreviewer(index < 0 ? { kind: 'list', hostId: id, paths: [entry.path], index: 0 } : { kind: 'list', hostId: id, paths: siblings, index });
      return;
    }
    // Desktop: the editor sits next to the list; switching files asks first if it holds edits.
    if (!switchEditorTarget({ hostId: id, path: entry.path })) return;
    navigate('/files/edit');
  }

  // UNI-191: "Edit as text" from the row menu — opens even a binary file
  // in the editor, lossily decoded, read-only until "Edit anyway".
  function editAsText(entry: SortedEntry) {
    const id = hostId();
    if (!id) return;
    setEditorTarget({ hostId: id, path: entry.path, asText: true });
    navigate('/files/edit');
  }

  function openRecent(recent: RecentFile) {
    setHostId(recent.hostId);
    setEditorTarget({ hostId: recent.hostId, path: recent.path });
    setRecentsOpen(false);
    navigate('/files/edit');
  }

  async function doDelete(entry: SortedEntry) {
    const id = hostId();
    if (!id) return;
    const result = await fsDelete(id, entry.path, true);
    if (result.ok) {
      showToast(`Deleted ${entry.name}`);
      void load();
    } else {
      showToast(result.error.message);
    }
    setDeleteTarget(undefined);
  }

  async function doRename(entry: SortedEntry, name: string) {
    const id = hostId();
    if (!id || !name.trim() || name.trim() === entry.name) {
      setRenameTarget(undefined);
      return;
    }
    const to = joinPath(dirname(entry.path), name.trim());
    const result = await fsRename(id, entry.path, to);
    if (result.ok) {
      showToast(`Renamed to ${name.trim()}`);
      void load();
    } else {
      showToast(result.error.message);
    }
    setRenameTarget(undefined);
  }

  async function doMkdir(name: string) {
    const id = hostId();
    const dir = path();
    if (!id || !dir || !name.trim()) {
      setNewFolderOpen(false);
      return;
    }
    const result = await fsMkdir(id, joinPath(dir, name.trim()));
    if (result.ok) void load();
    else showToast(result.error.message);
    setNewFolderOpen(false);
  }

  // UNI-198: "New file" — any name (incl. dotfiles, extensionless),
  // created empty then handed straight to the editor to fill in and save.
  async function doNewFile(name: string) {
    const id = hostId();
    const dir = path();
    const trimmed = name.trim();
    if (!id || !dir || !trimmed) {
      setNewFileOpen(false);
      return;
    }
    if (trimmed.includes('/')) {
      showToast("File name can't contain /");
      return;
    }
    const target = joinPath(dir, trimmed);
    // fs_write with create:true happily overwrites an existing file (no
    // base_hash given), so check first rather than clobber it.
    const existing = await fsStat(id, target);
    if (existing.ok) {
      showToast(`${trimmed} already exists`);
      return;
    }
    const result = await fsWrite(id, target, '', undefined, true);
    if (result.ok) {
      setNewFileOpen(false);
      setEditorTarget({ hostId: id, path: target });
      navigate('/files/edit');
    } else {
      showToast(result.error.message);
    }
  }

  // UNI-199: "Upload" — put the bytes via the blob channel, then
  // fs_import them to the chosen destination. A name clash (E_CONFLICT)
  // gets " (1)", " (2)"… appended before the extension and retries.
  function withSuffix(name: string, n: number): string {
    const dot = name.lastIndexOf('.');
    if (dot <= 0) return name + ' (' + n + ')';
    return name.slice(0, dot) + ' (' + n + ')' + name.slice(dot);
  }

  async function importWithRetry(id: string, sha256: string, dir: string, name: string): Promise<string | undefined> {
    for (let n = 0; n <= 20; n += 1) {
      const candidate = n === 0 ? name : withSuffix(name, n);
      const target = joinPath(dir, candidate);
      const result = await fsImport(id, sha256, target);
      if (result.ok) return candidate;
      if (result.error.code !== 'E_CONFLICT') {
        showToast(result.error.message);
        return undefined;
      }
    }
    showToast('Too many files named like ' + name);
    return undefined;
  }

  async function doUpload(files: File[]) {
    const id = hostId();
    const dir = uploadDest();
    setUploadOpen(false);
    if (!id || !dir || files.length === 0) return;
    let lastName: string | undefined;
    for (const file of files) {
      setUploading(file.name);
      try {
        // UNI-278: big files are streamed (no whole-file copy in the WebView).
        const put = await uploadPicked(id, file, (p) =>
          setUploading(file.name + ' (' + Math.round((p.sent / Math.max(p.total, 1)) * 100) + '%)'),
        );
        const saved = await importWithRetry(id, put.sha256, dir, file.name);
        if (saved) lastName = saved;
      } catch (error) {
        showToast((error as { message?: string })?.message ?? ("Could not upload " + file.name));
      }
    }
    setUploading(null);
    if (!lastName) return;
    if (dir === path()) {
      void load();
      showToast(files.length > 1 ? ('Uploaded ' + files.length + ' files') : ('Uploaded ' + lastName));
    } else {
      setToast('Uploaded to ' + basename(dir));
      // The toast has no action slot; navigate the browser there — a
      // simple "Open folder" that works even without a dedicated button.
      setPath(dir);
    }
  }

  function pickUploadFiles() {
    setUploadOpen(false);
    uploadInput?.click();
  }

  function copyPath(entry: SortedEntry) {
    void navigator.clipboard?.writeText(entry.path).then(
      () => showToast('Path copied'),
      () => showToast('Could not copy'),
    );
  }

  // UNI-169: get a host file onto the phone / into another app.
  async function transferFile(entry: SortedEntry, op: 'download' | 'openWith') {
    const id = hostId();
    if (!id) return;
    // UNI-225: streamed to the device (no preview size cap) with live progress in the toast.
    const verb = op === 'download' ? 'Downloading' : 'Opening';
    setToast(progressLabel(verb, undefined));
    const onProgress = (p: TransferProgress) => setToast(progressLabel(verb, p));
    const result = op === 'download'
      ? await downloadToDevice(id, entry.path, onProgress)
      : await openWithExternal(id, entry.path, onProgress);
    showToast(transferMessage(result, entry.name, desktopShell()));
  }

  // UNI-193: "Mention in chat" asks which chat — Files can be the home
  // for several chats, so dropping straight into whichever chat happened
  // to be open in the background was often the wrong one.
  function mentionInChat(entry: SortedEntry) {
    setMentionPick({ entry, root: dirname(entry.path) || path() || '/' });
  }

  function pickMentionChat(target: ChatTarget) {
    const pick = mentionPick();
    setMentionPick(undefined);
    if (!pick) return;
    queueMention(`${target.hostId}:${target.session}`, `@${pick.entry.path} `);
    openChat(target, hostName(target.hostId));
  }

  // ── UNI-117 desktop: keyboard selection, drop files from the OS to upload ──
  const [selected, setSelected] = createSignal(-1);
  createEffect(on(entries, () => setSelected(-1)));
  let listEl: HTMLDivElement | undefined;
  const focusRow = (i: number) => {
    setSelected(i);
    const row = listEl?.querySelectorAll<HTMLElement>('[data-testid=files-row]')[i];
    row?.focus({ preventScroll: true });
    row?.scrollIntoView?.({ block: 'nearest' });
  };
  // Window-level so Backspace works in an empty folder too; only while focus is in the Files
  // screen (or nowhere) and no dialog/menu/editor has it.
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target instanceof Element ? e.target : null;
      const free = !t || t === document.body || !!t.closest('.files-screen');
      if (!free || t?.closest?.('.sheet, [role=menu], .palette')) return;
      onListKey(e);
    };
    window.addEventListener('keydown', onKey);
    onCleanup(() => window.removeEventListener('keydown', onKey));
  });
  const onListKey = (e: KeyboardEvent) => {
    if (!desktopShell() || inTextField(e.target) || e.defaultPrevented) return;
    if (renameTarget() || deleteTarget() || menu() || headerMenu()) return;
    const list = visibleEntries();
    const action = filesKey(e, selected(), list.length);
    if (!action) return;
    e.preventDefault();
    e.stopPropagation();
    const entry = list[selected()];
    if (action.kind === 'move') focusRow(action.to);
    else if (action.kind === 'open' && entry) openEntry(entry);
    else if (action.kind === 'rename' && entry) setRenameTarget(entry);
    else if (action.kind === 'delete' && entry) setDeleteTarget(entry);
    else if (action.kind === 'up') {
      // Like Back, but never leaves the tree: a root (home, a session's cwd) is the top.
      const verdict = nextFilesBack({ path: path(), roots: roots(), hasReturnChat: false });
      if (verdict.action === 'parent') setPath(verdict.to);
    }
  };
  const [dropOver, setDropOver] = createSignal(false);
  let dropDepth = 0;
  const canDrop = () => desktopShell() && can('fileDragDrop') && !!path();
  const onDragEnter = (e: DragEvent) => {
    if (!canDrop() || !dragHasFiles(e.dataTransfer)) return;
    e.preventDefault();
    dropDepth += 1;
    setDropOver(true);
  };
  const onDragOver = (e: DragEvent) => {
    if (!canDrop() || !dragHasFiles(e.dataTransfer)) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy';
  };
  const onDragLeave = () => {
    dropDepth = Math.max(0, dropDepth - 1);
    if (dropDepth === 0) setDropOver(false);
  };
  const onDrop = (e: DragEvent) => {
    dropDepth = 0;
    setDropOver(false);
    if (!canDrop() || !dragHasFiles(e.dataTransfer)) return;
    e.preventDefault();
    const files = [...(e.dataTransfer?.files ?? [])];
    if (!files.length) return;
    // Dropped on the folder you're looking at: upload right there.
    setUploadDest(path()!);
    void doUpload(files);
  };

  // UNI-256: the desktop app gets dropped files as paths (WebKitGTK gives the page no Files).
  let screenEl: HTMLDivElement | undefined;
  if (isTauriDesktop()) {
    void startNativeDrop();
    onCleanup(
      registerDropZone({
        el: () => screenEl,
        enabled: canDrop,
        hover: setDropOver,
        drop: (paths) => {
          void describeLocal(paths)
            .then((found) => readLocalFiles(found.filter((f) => !f.isDir)))
            .then((files) => {
              if (!files.length) return showToast('Folders can’t be uploaded — drop the files inside');
              setUploadDest(path()!);
              return doUpload(files);
            })
            .catch((error) => showToast(String((error as { message?: string })?.message ?? error)));
        },
      }),
    );
  }

  let touchStartY = 0;
  let pulling = false;
  function onTouchStart(e: TouchEvent) {
    if ((e.currentTarget as HTMLElement).scrollTop <= 0) {
      touchStartY = e.touches[0]?.clientY ?? 0;
      pulling = true;
    }
  }
  function onTouchMove(e: TouchEvent) {
    if (!pulling) return;
    const dy = (e.touches[0]?.clientY ?? 0) - touchStartY;
    if (dy > 80 && !refreshing()) {
      setRefreshing(true);
      void load();
    }
  }
  function onTouchEnd() {
    pulling = false;
  }

  return (
    <Show
      when={hosts.hosts().length > 0}
      fallback={
        <div class="files-screen page-screen">
          <PageHeader title="Files" />
          <NoHostState what="its files" onAdd={() => setAddHostOpen(true)} />
        </div>
      }
    >
    <div class="files-screen" ref={screenEl} data-drop-over={dropOver() ? '' : undefined} onDragEnter={onDragEnter} onDragOver={onDragOver} onDragLeave={onDragLeave} onDrop={onDrop}>
      <Show when={dropOver()}>
        <div class="files-drop" data-testid="files-drop" aria-hidden="true">
          <span class="chat-drop-icon">
            <Upload size={26} />
          </span>
          <p class="chat-drop-title">Drop to upload</p>
          <p class="chat-drop-hint truncate">to {path()}</p>
        </div>
      </Show>
      <PageHeader
        title="Files"
        lead={<PcChip name={hostName(hostId())} state={isUnreachable(error()?.code) ? 'error' : hostId() ? 'ok' : 'checking'} label="Choose a PC" onClick={() => setHostSheetOpen(true)} />}
        tools={
          <>
            <button type="button" class="ghost-icon-button" aria-label="Search" onClick={() => setSearchOpen(true)}>
              <Search size={19} />
            </button>
            <button
              type="button"
              class="ghost-icon-button"
              aria-label="Files menu"
              aria-expanded={!!headerMenu()}
              onClick={(e) => setHeaderMenu(e.currentTarget.getBoundingClientRect())}
            >
              <MoreVertical size={19} />
            </button>
          </>
        }
      />

      <Show when={!isUnreachable(error()?.code)}>
      <div class="files-host-row" role="toolbar" aria-label="Filters">
        {/* Places: $HOME, the host's extra roots (/tmp, /mnt…, UNI-220), session folders. */}
        <Show when={places() === 'inline'}>
          <div class="files-places" role="group" aria-label="Places">
            <For each={roots()}>
              {(root) => (
                <button
                  type="button"
                  class="files-pill files-root-pill"
                  classList={{ active: activeRoot(path(), roots()) === root }}
                  aria-current={activeRoot(path(), roots()) === root ? 'true' : undefined}
                  title={root}
                  onClick={() => setPath(root)}
                >
                  {rootLabel(root, roots()[0])}
                </button>
              )}
            </For>
          </div>
        </Show>
        <Show when={places() === 'menu'}>
          <div class="files-places" role="group" aria-label="Places">
            <button
              type="button"
              class="files-pill files-root-pill files-places-menu"
              aria-haspopup="menu"
              aria-expanded={!!placesMenu()}
              title={activeRoot(path(), roots()) ?? path()}
              onClick={(e) => setPlacesMenu(e.currentTarget.getBoundingClientRect())}
            >
              <HardDrive size={15} />
              <span class="truncate">{(() => { const r = activeRoot(path(), roots()); return r ? rootLabel(r, roots()[0]) : 'Places'; })()}</span>
              <ChevronDown size={14} />
            </button>
          </div>
        </Show>
        <Show when={places() !== 'none'}>
          <span class="files-toolbar-sep" aria-hidden="true" />
        </Show>
        <button type="button" class="files-pill" classList={{ active: hidden() }} aria-pressed={hidden()} onClick={() => setHidden((v) => !v)}>
          Hidden
        </button>
        <button type="button" class="files-pill" classList={{ active: changedOnly() }} aria-pressed={changedOnly()} onClick={() => setChangedOnly((v) => !v)}>
          Changed
        </button>
        <Show when={repo()}>
          {(r) => (
            <button
              type="button"
              class="files-pill files-git-chip"
              data-testid="files-git-chip"
              aria-label={`Git: ${branchLabel(r())}${aheadBehind(r()) ? ` ${aheadBehind(r())}` : ''}. Open Git`}
              title={`${r().root ?? ''} · ${branchLabel(r())}`}
              onClick={openGitHere}
            >
              <GitBranch size={15} />
              <span class="truncate">{branchLabel(r())}</span>
              <Show when={aheadBehind(r())}>
                <span class="git-chip-track">{aheadBehind(r())}</span>
              </Show>
              <Show when={repoDirty()}>
                <span class="git-chip-dirty" aria-hidden="true" />
              </Show>
            </button>
          )}
        </Show>
        <input
          ref={uploadInput}
          type="file"
          multiple
          class="files-upload-input"
          onChange={(e) => {
            const files = Array.from(e.currentTarget.files ?? []);
            e.currentTarget.value = '';
            if (files.length > 0) void doUpload(files);
          }}
        />
      </div>

      {/* At a root the Places pill already names it: no one-crumb row under it. */}
      <Show when={crumbs().length > 1 || places() === 'none'}>
      <nav class="files-breadcrumb" aria-label="Breadcrumb">
        <For each={crumbs()}>
          {(crumb, i) => (
            <>
              <button
                type="button"
                class="files-crumb"
                data-root={crumb.root ? '' : undefined}
                title={crumb.path}
                aria-current={i() === crumbs().length - 1 ? 'true' : undefined}
                onClick={() => setPath(crumb.path)}
              >
                {crumb.label}
              </button>
              <Show when={i() < crumbs().length - 1}>
                <ChevronRight size={14} class="files-crumb-sep" />
              </Show>
            </>
          )}
        </For>
      </nav>
      </Show>
      </Show>

      <Show when={placesMenu()}>
        {(anchor) => (
          <AnchoredMenu anchor={anchor()} label="Places" width={300} onDismiss={() => setPlacesMenu(undefined)}>
            <For each={roots()}>
              {(root) => (
                <MenuRow
                  icon={activeRoot(path(), roots()) === root ? <Check size={18} /> : <HardDrive size={18} />}
                  label={rootLabel(root, roots()[0])}
                  hint={rootLabel(root, roots()[0]) === root ? undefined : <span class="files-place-path">{root}</span>}
                  onSelect={() => { setPlacesMenu(undefined); setPath(root); }}
                />
              )}
            </For>
          </AnchoredMenu>
        )}
      </Show>

      <div class="files-list" data-testid="files-list" ref={listEl} onTouchStart={onTouchStart} onTouchMove={onTouchMove} onTouchEnd={onTouchEnd}>
        <Show when={refreshing()}>
          <div class="files-refresh-spinner"><RefreshCw size={16} class="spin" /></div>
        </Show>
        <Show when={error()}>
          {(e) => (
            <Show
              when={isUnreachable(e().code)}
              fallback={<ErrorBanner error={{ code: 'E_INTERNAL', message: e().message, retryable: e().retryable }} retry={() => void load()} />}
            >
              <OfflineState pc={hostName(hostId())} message={e().message} hint={e().hint} onRetry={() => (path() ? void load() : void loadRoots())} />
            </Show>
          )}
        </Show>
        <Show when={!loading() && !error() && visibleEntries().length === 0}>
          <EmptyState compact icon={<FolderOpen size={22} />} title={changedOnly() ? 'Nothing changed here' : 'This folder is empty'} body={changedOnly() ? 'No git changes in this folder.' : desktopShell() ? 'Drop files here or use ⋯ → Upload.' : 'Use ⋯ → Upload to add files.'} />
        </Show>
        <For each={visibleEntries()}>
          {(entry, i) => (
            <div
              role="button"
              tabIndex={0}
              class="files-row"
              data-testid="files-row"
              classList={{ dim: entry.ignored }}
              data-selected={desktopShell() && selected() === i() ? '' : undefined}
              onClick={() => {
                setSelected(i());
                openEntry(entry);
              }}
              onFocus={() => setSelected(i())}
              onKeyDown={(e) => {
                if (desktopShell()) return; // the list's handler (arrows/Enter/F2/Del)
                if (e.key === 'Enter' || e.key === ' ') openEntry(entry);
              }}
              onContextMenu={(e) => {
                e.preventDefault();
                setSelected(i());
                setMenu({ entry, anchor: menuAnchor(e, e.currentTarget as HTMLElement) });
              }}
            >
              <span class="files-row-icon">{entryIcon(entry)}</span>
              <span class="min-w-0 flex-1 text-left">
                <span class="files-row-name truncate">{entry.name}</span>
                <span class="files-row-meta">
                  {entry.kind !== 'dir' && <span>{formatSize(entry.size)}</span>}
                  <span>{formatMtime(entry.mtime)}</span>
                </span>
              </span>
              <Show when={entry.git}>
                <span class="files-git-badge" data-git={entry.git}>{entry.git}</span>
              </Show>
              <button
                type="button"
                class="ghost-icon-button files-row-menu"
                aria-label={`Actions for ${entry.name}`}
                onClick={(e) => {
                  e.stopPropagation();
                  setMenu({ entry, anchor: menuAnchor(e, e.currentTarget as HTMLElement) });
                }}
              >
                <EllipsisVertical size={16} />
              </button>
            </div>
          )}
        </For>
        <Show when={more()}>
          <p class="files-more-note">Showing the first entries — narrow with search for more.</p>
        </Show>
      </div>

      <Show when={headerMenu()}>
        {(anchor) => (
          <AnchoredMenu anchor={anchor()} label="Files menu" onDismiss={() => setHeaderMenu(undefined)}>
            <MenuRow icon={<History size={18} />} label="Recent files" onSelect={() => { setHeaderMenu(undefined); setRecentsOpen(true); }} />
            <Show when={repo()}>
              <MenuRow icon={<GitBranch size={18} />} label="Git…" onSelect={() => { setHeaderMenu(undefined); openGitHere(); }} />
            </Show>
            <MenuSeparator />
            <MenuRow icon={<FilePlus size={18} />} label="New file…" onSelect={() => { setHeaderMenu(undefined); setNewFileOpen(true); }} />
            <MenuRow icon={<FolderPlus size={18} />} label="New folder…" onSelect={() => { setHeaderMenu(undefined); setNewFolderOpen(true); }} />
            <MenuRow icon={<Upload size={18} />} label="Upload…" onSelect={() => { setHeaderMenu(undefined); setUploadDest(DEFAULT_UPLOAD_DIR); setUploadOpen(true); }} />
          </AnchoredMenu>
        )}
      </Show>

      <Show when={menu()}>
        {(m) => (
          <AnchoredMenu anchor={m().anchor} label="File actions" onDismiss={() => setMenu(undefined)}>
            <Show when={m().entry.kind !== 'dir'}>
              <MenuRow icon={<Download size={18} />} label={desktopShell() ? 'Download' : 'Download to phone'} onSelect={() => { const e = m().entry; setMenu(undefined); void transferFile(e, 'download'); }} />
              <MenuRow icon={<ExternalLink size={18} />} label="Open with…" onSelect={() => { const e = m().entry; setMenu(undefined); void transferFile(e, 'openWith'); }} />
              <MenuRow icon={<FileText size={18} />} label="Edit as text" onSelect={() => { const e = m().entry; setMenu(undefined); editAsText(e); }} />
              <MenuSeparator />
            </Show>
            <MenuRow icon={<Pencil size={18} />} label="Rename…" onSelect={() => { const e = m().entry; setMenu(undefined); setRenameTarget(e); }} />
            <MenuRow icon={<Copy size={18} />} label="Copy path" onSelect={() => { const e = m().entry; setMenu(undefined); copyPath(e); }} />
            <MenuRow icon={<MessageSquarePlus size={18} />} label="Mention in chat" onSelect={() => { const e = m().entry; setMenu(undefined); mentionInChat(e); }} />
            <MenuSeparator />
            <MenuRow icon={<Trash2 size={18} />} label="Delete…" onSelect={() => { const e = m().entry; setMenu(undefined); setDeleteTarget(e); }} />
          </AnchoredMenu>
        )}
      </Show>

      <HostSheet open={hostSheetOpen()} onClose={() => setHostSheetOpen(false)} hostIds={hosts.hosts().map((h) => h.id)} names={hosts.hosts()} current={hostId()} onPick={(id) => (setHostId(id), setPath(undefined), setHostSheetOpen(false))} />
      <ChatPickSheet
        open={!!mentionPick()}
        hostId={hostId()}
        root={mentionPick()?.root}
        label={mentionPick() ? `Mention ${mentionPick()!.entry.name} in…` : 'Mention in…'}
        onPick={pickMentionChat}
        onClose={() => setMentionPick(undefined)}
      />

      <SearchSheet open={searchOpen()} onClose={() => setSearchOpen(false)} hostId={hostId()} root={path()} onPick={(p) => (setPath(dirname(p)), setSearchOpen(false))} onOpenFile={(p) => {
        const id = hostId();
        if (!id) return;
        pushRecent(id, p);
        setEditorTarget({ hostId: id, path: p });
        setSearchOpen(false);
        navigate('/files/edit');
      }} />

      <RecentsSheet open={recentsOpen()} onClose={() => setRecentsOpen(false)} onPick={openRecent} />

      <RenameSheet entry={renameTarget()} onClose={() => setRenameTarget(undefined)} onSave={doRename} />
      <DeleteConfirmSheet entry={deleteTarget()} onClose={() => setDeleteTarget(undefined)} onConfirm={doDelete} />
      <NewFolderSheet open={newFolderOpen()} onClose={() => setNewFolderOpen(false)} onCreate={doMkdir} />
      <NewFileSheet open={newFileOpen()} onClose={() => setNewFileOpen(false)} onCreate={doNewFile} />
      <UploadSheet
        open={uploadOpen()}
        onClose={() => setUploadOpen(false)}
        currentPath={path()}
        dest={uploadDest()}
        onPick={setUploadDest}
        onChooseFiles={pickUploadFiles}
      />

      <Show when={uploading()}>
        <div class="files-toast" role="status">Uploading {uploading()}…</div>
      </Show>
      <Show when={!uploading() && toast()}>
        <div class="files-toast" role="status">{toast()}</div>
      </Show>
    </div>
    </Show>
  );
}

function HostSheet(props: { open: boolean; onClose: () => void; hostIds: string[]; names: { id: string; name: string }[]; current?: string; onPick: (id: string) => void }) {
  return (
    <BoardSheet open={props.open} onClose={props.onClose} label="Choose a PC" testId="files-host-sheet">
      <div class="settings-group rounded-card bg-surface-2/60">
        <For each={props.names}>
          {(host) => (
            <button type="button" class="picker-row" aria-current={props.current === host.id ? 'true' : undefined} onClick={() => props.onPick(host.id)}>
              <span class="settings-icon"><Home size={16} /></span>
              <span class="min-w-0 flex-1 text-left truncate">{host.name}</span>
            </button>
          )}
        </For>
        <Show when={props.names.length === 0}>
          <p class="p-4 text-text-muted text-sm">Pair a PC first (Settings → Add PC).</p>
        </Show>
      </div>
    </BoardSheet>
  );
}

function SearchSheet(props: {
  open: boolean;
  onClose: () => void;
  hostId: string | undefined;
  root: string | undefined;
  onPick: (path: string) => void;
  onOpenFile: (path: string) => void;
}) {
  const [query, setQuery] = createSignal('');
  const [kind, setKind] = createSignal<'name' | 'content'>('name');
  const [results, setResults] = createSignal<{ path: string; line?: number; preview?: string }[]>([]);
  const [searching, setSearching] = createSignal(false);

  async function run() {
    const id = props.hostId;
    const root = props.root;
    const q = query().trim();
    if (!id || !root || !q) {
      setResults([]);
      return;
    }
    setSearching(true);
    const result = await fsSearch(id, root, q, kind());
    setSearching(false);
    if (result.ok) setResults(result.value.results);
  }

  let timer: ReturnType<typeof setTimeout> | undefined;
  createEffect(() => {
    query();
    kind();
    if (!props.open) return;
    clearTimeout(timer);
    timer = setTimeout(() => void run(), 250);
  });
  onCleanup(() => clearTimeout(timer));

  return (
    <BoardSheet open={props.open} onClose={props.onClose} label="Search files" testId="files-search-sheet">
      <div class="files-search-row">
        <input
          type="text"
          class="files-search-input"
          placeholder="Search by name or content…"
          value={query()}
          onInput={(e) => setQuery(e.currentTarget.value)}
          autofocus
        />
        <Show when={query()}>
          <button type="button" class="ghost-icon-button" aria-label="Clear" onClick={() => setQuery('')}>
            <X size={18} />
          </button>
        </Show>
      </div>
      <div class="files-search-tabs">
        <button type="button" class="files-pill" classList={{ active: kind() === 'name' }} onClick={() => setKind('name')}>
          Name
        </button>
        <button type="button" class="files-pill" classList={{ active: kind() === 'content' }} onClick={() => setKind('content')}>
          Content
        </button>
      </div>
      <div class="files-search-results" data-testid="files-search-results">
        <Show when={searching()}>
          <p class="p-3 text-text-muted text-sm">Searching…</p>
        </Show>
        <For each={results()}>
          {(r) => (
            <button type="button" class="files-search-result" onClick={() => props.onOpenFile(r.path)}>
              <span class="truncate">{r.path}</span>
              <Show when={r.preview}>
                <span class="files-search-preview truncate">{r.line ? `:${r.line} ` : ''}{r.preview}</span>
              </Show>
            </button>
          )}
        </For>
      </div>
    </BoardSheet>
  );
}

function RecentsSheet(props: { open: boolean; onClose: () => void; onPick: (recent: RecentFile) => void }) {
  const recents = createMemo(() => (props.open ? readRecents() : []));
  return (
    <BoardSheet open={props.open} onClose={props.onClose} label="Recent files" testId="files-recents-sheet">
      <div class="settings-group rounded-card bg-surface-2/60">
        <For each={recents()}>
          {(r) => (
            <button type="button" class="picker-row" onClick={() => props.onPick(r)}>
              <span class="settings-icon"><FileIcon size={16} /></span>
              <span class="min-w-0 flex-1 text-left truncate">{r.path}</span>
            </button>
          )}
        </For>
        <Show when={recents().length === 0}>
          <p class="p-4 text-text-muted text-sm">No files opened yet.</p>
        </Show>
      </div>
    </BoardSheet>
  );
}

/** A name form submits once until its request settles (Enter twice, or Enter then a click). */
export function submitOnce(): (run: () => void | Promise<void>) => void {
  let busy = false;
  return (run) => {
    if (busy) return;
    busy = true;
    void Promise.resolve()
      .then(run)
      .finally(() => {
        busy = false;
      });
  };
}

function RenameSheet(props: { entry: SortedEntry | undefined; onClose: () => void; onSave: (entry: SortedEntry, name: string) => void | Promise<void> }) {
  const [name, setName] = createSignal('');
  const once = submitOnce();
  createEffect(() => setName(props.entry?.name ?? ''));
  return (
    <BoardSheet open={!!props.entry} onClose={props.onClose} label="Rename" testId="files-rename-sheet">
      <form class="files-name-form" onSubmit={(e) => { e.preventDefault(); const entry = props.entry; if (entry) once(() => props.onSave(entry, name())); }}>
        <input type="text" class="files-search-input" aria-label="New name" value={name()} onInput={(e) => setName(e.currentTarget.value)} autofocus />
        <div class="files-sheet-actions">
          <button type="button" class="files-sheet-btn" onClick={props.onClose}>Cancel</button>
          <button type="submit" class="files-sheet-btn primary">Save</button>
        </div>
      </form>
    </BoardSheet>
  );
}

function DeleteConfirmSheet(props: { entry: SortedEntry | undefined; onClose: () => void; onConfirm: (entry: SortedEntry) => void }) {
  return (
    <BoardSheet open={!!props.entry} onClose={props.onClose} label="Delete" testId="files-delete-sheet">
      <p class="p-2">
        Delete <strong>{props.entry?.name}</strong>
        {props.entry?.kind === 'dir' ? ' and everything inside it' : ''}? This can't be undone.
      </p>
      <div class="files-sheet-actions">
        <button type="button" class="files-sheet-btn" onClick={props.onClose}>Cancel</button>
        <button type="button" class="files-sheet-btn danger" onClick={() => props.entry && props.onConfirm(props.entry)}>Delete</button>
      </div>
    </BoardSheet>
  );
}

function NewFolderSheet(props: { open: boolean; onClose: () => void; onCreate: (name: string) => void | Promise<void> }) {
  const [name, setName] = createSignal('');
  const once = submitOnce();
  createEffect(() => {
    if (props.open) setName('');
  });
  return (
    <BoardSheet open={props.open} onClose={props.onClose} label="New folder" testId="files-mkdir-sheet">
      <form class="files-name-form" onSubmit={(e) => { e.preventDefault(); once(() => props.onCreate(name())); }}>
        <input type="text" class="files-search-input" placeholder="Folder name" aria-label="Folder name" value={name()} onInput={(e) => setName(e.currentTarget.value)} autofocus />
        <div class="files-sheet-actions">
          <button type="button" class="files-sheet-btn" onClick={props.onClose}>Cancel</button>
          <button type="submit" class="files-sheet-btn primary">Create</button>
        </div>
      </form>
    </BoardSheet>
  );
}

function NewFileSheet(props: { open: boolean; onClose: () => void; onCreate: (name: string) => void | Promise<void> }) {
  const [name, setName] = createSignal('');
  const once = submitOnce();
  createEffect(() => {
    if (props.open) setName('');
  });
  return (
    <BoardSheet open={props.open} onClose={props.onClose} label="New file" testId="files-new-file-sheet">
      <form class="files-name-form" onSubmit={(e) => { e.preventDefault(); once(() => props.onCreate(name())); }}>
        <input type="text" class="files-search-input" placeholder="File name" aria-label="File name" value={name()} onInput={(e) => setName(e.currentTarget.value)} autofocus />
        <div class="files-sheet-actions">
          <button type="button" class="files-sheet-btn" onClick={props.onClose}>Cancel</button>
          <button type="submit" class="files-sheet-btn primary">Create</button>
        </div>
      </form>
    </BoardSheet>
  );
}

/** `~/.unipi/uploads` in either form (the browser shows absolute paths). */
function isUploadDir(path: string): boolean {
  return path.replace(/\/+$/, '') === DEFAULT_UPLOAD_DIR || /^\/home\/[^/]+\/\.unipi\/uploads\/?$/.test(path) || /^\/root\/\.unipi\/uploads\/?$/.test(path);
}

function UploadSheet(props: {
  open: boolean;
  onClose: () => void;
  currentPath: string | undefined;
  dest: string;
  onPick: (dest: string) => void;
  onChooseFiles: () => void;
}) {
  const Row = (p: { value: string; title: string; sub: string; icon: JSX.Element }) => (
    <button type="button" role="radio" class="files-upload-dest-row" aria-checked={props.dest === p.value ? 'true' : 'false'} onClick={() => props.onPick(p.value)}>
      <span class="files-upload-radio" aria-hidden="true" />
      {p.icon}
      <span class="min-w-0 flex-1 text-left">
        <span class="block truncate">{p.title}</span>
        <span class="files-upload-sub truncate">{p.sub}</span>
      </span>
    </button>
  );
  return (
    <BoardSheet open={props.open} onClose={props.onClose} label="Upload to…" testId="files-upload-sheet">
      <div class="files-upload-dest" role="radiogroup" aria-label="Upload destination">
        {Row({ value: DEFAULT_UPLOAD_DIR, title: 'Shared uploads folder', sub: DEFAULT_UPLOAD_DIR, icon: <Upload size={16} /> })}
        <Show when={props.currentPath && !isUploadDir(props.currentPath) ? props.currentPath : undefined}>{(here) => Row({ value: here(), title: 'This folder', sub: here(), icon: <Folder size={16} /> })}</Show>
      </div>
      <div class="files-sheet-actions">
        <button type="button" class="files-sheet-btn" onClick={props.onClose}>Cancel</button>
        <button type="button" class="files-sheet-btn primary" onClick={props.onChooseFiles}>
          Choose files → {props.dest === DEFAULT_UPLOAD_DIR ? 'uploads' : 'this folder'}
        </button>
      </div>
    </BoardSheet>
  );
}
