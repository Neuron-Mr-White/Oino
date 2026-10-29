/**
 * The open chat (UNI-113): a full-screen layer over the app (like the
 * terminal). Header (back, title, live dot, Terminal switch), transcript
 * with follow-the-tail scrolling + jump button, running clock row, then the
 * composer — or a pi dialog in its place.
 */
import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { createVirtualizer } from '@tanstack/solid-virtual';
import { ArrowDown, ChevronLeft, CircleAlert, Copy, Ellipsis, EllipsisVertical, Globe, History, Quote, RefreshCw, RotateCcw, SquareStack, SquareTerminal } from 'lucide-solid';
import { useNavigate } from '@solidjs/router';
import { useSettings } from '../../settings/context';
import { hapticTick, type HapticKind } from '../../lib/haptics';
import { pushBack } from '../../lib/back';
import { buildRows, formatDuration, liveActivity, rowIndexForEntry, textOf, toolResultOwners, type Row } from '../../lib/chat/model';
import { SearchSheet } from './SearchSheet';
import type { SearchHit } from '../../lib/chat/store';
import { TERMINAL_ONLY } from '../../lib/chat/commands';
import { chatVisible, closeChat, hideChat, markReturnToChat, openChatState } from '../../lib/chat/open';
import { CloseChatSheet } from './CloseChatSheet';
import { closeChatTarget, setCloseChatTarget } from '../../lib/chat/closeTarget';
import { errorWords, folderName, hideChatFromList, newChatHere, targetFromNew } from '../../lib/chat/newChat';
import { setTermTarget } from '../../lib/termTarget';
import { setFilesTarget } from '../../lib/filesTarget';
import { openGit } from '../../lib/git';
import { addTaskFromChat, openBoardFromChat } from '../../lib/chat/boardPortal';
import { consumeMentionFor } from '../../lib/chat/mention';
import { copyText, openExternal } from '../../lib/board/share';
import { RowView } from './Transcript';
import { desktopShell, menuAnchor } from '../../lib/layout';
import { Composer, type ComposerApi } from './Composer';
import { adjacentIndex, dragHasFiles, inTextField } from '../../lib/desktop';
import { chatGroups, chatLists, needsYouFirst, WORKING_WORD } from '../../lib/chat/list';
import { openChat } from '../../lib/chat/open';
import { can, isTauriDesktop } from '../../lib/platform';
import { describeLocal, registerDropZone, startNativeDrop } from '../../lib/localFiles';
import { Upload } from 'lucide-solid';
import { BtwSheet } from './BtwSheet';
import { DialogStack } from './DialogCard';
import { FileRefMenu } from './FileRefMenu';
import { BusyError, type SessionsItem, type TreeGoRestore, type TreeNode } from '../../lib/chat/store';
import { imageRefFile } from './ImageChip';
import { ChatHeaderMenu, MessageMenu, RenameSheet, ResumeSheet, RunConfirmSheet, TreeSheet, WelcomeState, type RunRequest } from './SessionMenus';
import { ProgressSheet, progressActive } from './ProgressSheet';
import { chatModeView, ModeChip, ModePopover } from './ModeChip';
import { progressSheetOpen, setProgressSheetOpen } from '../../lib/chat/progress';
import { DreamSheet, dreamHint } from './DreamSheet';
import { dreamSheetOpen, setDreamSheetOpen } from '../../lib/chat/dream';
import { BrowserSheet, BrowserSidePane } from './BrowserSide';
import { browserPane, toggleBrowser } from '../../lib/browserPane';
import { modKey } from '../../lib/shell';
import { ControlCentreSheet, FooterStrip, FooterTray, ModelPickerSheet, modelHeaderLabel, runningWorkCount, type Section } from './ControlCentre';

/** Within this many px of the bottom counts as "following". */
const FOLLOW = 32;

/** UNI-207: elements inside a message row a tap must not toggle selection
 * for — links, code, buttons, image chips, task refs — so those keep
 * their own tap behaviour instead of opening/closing the action bar. */
const TAP_IGNORE_SELECTOR = 'a[href], code, button, [data-copy], [data-wrap-toggle], .chat-image-chip, .task-ref';
/** UNI-290: put a jumped-back-to message into the composer — straight away when the draft is
 *  empty (whitespace only counts as empty), else only after "Replace your draft?". */
export function restoreIntoDraft(draft: string, apply: () => void, ask: (request: RunRequest) => void): void {
  if (!draft.trim()) return apply();
  ask({ title: 'Replace your draft?', detail: 'The message you jumped back to goes into the composer, replacing what you have typed.', confirm: 'Replace', run: async () => apply() });
}

export function isMsgTapIgnored(el: Element): boolean {
  return el.closest(TAP_IGNORE_SELECTOR) !== null;
}

/** First-paint height guesses per row kind (real sizes are measured). */
function estimate(row: Row | undefined): number {
  if (!row) return 40;
  switch (row.kind) {
    case 'user':
      return 48 + Math.min(12, Math.ceil(row.text.length / 38)) * 22;
    case 'prose':
      return 30 + Math.ceil(row.text.length / 42) * 24 + (row.text.match(/```/g)?.length ?? 0) * 40;
    case 'process':
      return 42;
    case 'turn_end':
      return 26;
    case 'compaction':
      return 42;
    default:
      return 50;
  }
}

/** Top of the transcript: loads older history, or says where it starts. */
function HistoryEdge(props: { loading: boolean; more: boolean; onLoad: () => void }) {
  return (
    <div class="history-edge">
      <Show
        when={props.more}
        fallback={<span class="history-edge-start">Start of the conversation</span>}
      >
        <button type="button" class="history-edge-btn" disabled={props.loading} onClick={() => props.onLoad()}>
          {props.loading ? 'Loading earlier messages…' : 'Load earlier messages'}
        </button>
      </Show>
    </div>
  );
}

/**
 * UNI-206: when the transcript's last row is already a live process group
 * or live prose (its own header shows "Running · <cmd>" / shimmers while
 * streaming), this tail row would repeat the exact same line. `compact`
 * drops the title/detail and keeps only the dot + elapsed timer ("Working
 * 1m 27s") so there is still a visible, ticking "pi is alive" indicator
 * without the duplicate.
 */
export function RunningRow(props: { since?: number; label: { title: string; detail?: string }; compact?: boolean }) {
  const [now, setNow] = createSignal(Date.now());
  const t = setInterval(() => setNow(Date.now()), 1000);
  onCleanup(() => clearInterval(t));
  return (
    <div class="running-row" aria-live="polite" data-compact={props.compact ? '' : undefined}>
      <span class="running-dot" />
      <Show when={!props.compact} fallback={<span class="shimmer running-title">Working</span>}>
        <span class="shimmer running-title">{props.label.title}</span>
        <Show when={props.label.detail}>
          <span class="running-detail truncate">{props.label.detail}</span>
        </Show>
      </Show>
      <Show when={props.since}>
        <span class="tabular-nums running-time">{formatDuration(Math.max(0, now() - props.since!))}</span>
      </Show>
    </div>
  );
}

export function ChatLayer() {
  const [settings] = useSettings();
  const navigate = useNavigate();
  const haptic = (kind: HapticKind = 'tap') => void hapticTick(settings.input.haptics, kind);
  const open = openChatState;
  // UNI-201: the Model picker (single/Fusion + thinking + model list) and
  // the Session sheet (Context/Running/Info) are two separate sheets —
  // only one is ever open at a time.
  const [modelOpen, setModelOpen] = createSignal(false);
  const [ccOpen, setCcOpen] = createSignal(false);
  const [ccSection, setCcSection] = createSignal<Section>('context');
  // UNI-222: the long-horizon Progress sheet (header ⋯ → Progress, palette).
  const progressOpen = progressSheetOpen;
  const setProgressOpen = setProgressSheetOpen;
  const openModelPicker = () => {
    setTrayOpen(false);
    setModelOpen(true);
  };
  const openControlCentre = (section: Section) => {
    setTrayOpen(false);
    setCcSection(section);
    setCcOpen(true);
  };
  // The subheader strip's inline tray (UNI-188): tap to expand/collapse,
  // or scroll the transcript to collapse it (no modal, stays over the chat).
  const [trayOpen, setTrayOpen] = createSignal(false);
  const [drafts, setDrafts] = createSignal<Record<string, string>>({});
  const [following, setFollowing] = createSignal(true);
  const [toast, setToast] = createSignal<string | null>(null);

  const chat = () => open()?.chat;
  const s = () => chat()?.state;

  // ── UNI-117 desktop: drop files to attach, keyboard ────────────────
  let composerApi: ComposerApi | undefined;
  const [dragOver, setDragOver] = createSignal(false);
  let dragDepth = 0;
  const canDrop = () => desktopShell() && can('fileDragDrop') && !s()?.dialogs.length;
  const onDragEnter = (e: DragEvent) => {
    if (!canDrop() || !dragHasFiles(e.dataTransfer)) return;
    e.preventDefault();
    dragDepth += 1;
    setDragOver(true);
  };
  const onDragOver = (e: DragEvent) => {
    if (!canDrop() || !dragHasFiles(e.dataTransfer)) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy';
  };
  const onDragLeave = () => {
    dragDepth = Math.max(0, dragDepth - 1);
    if (dragDepth === 0) setDragOver(false);
  };
  const onDrop = (e: DragEvent) => {
    dragDepth = 0;
    setDragOver(false);
    if (!canDrop() || !dragHasFiles(e.dataTransfer)) return;
    e.preventDefault();
    const files = [...(e.dataTransfer?.files ?? [])];
    if (files.length) {
      composerApi?.attachFiles(files);
      composerApi?.focus();
    }
  };
  // UNI-256: in the desktop app WebKitGTK gives a file-manager drop no `File`s — Tauri's window
  // drag & drop reports the paths instead (lib/localFiles.ts routes them to the zone under the pointer).
  let layerEl: HTMLDivElement | undefined;
  if (isTauriDesktop()) {
    void startNativeDrop();
    onCleanup(
      registerDropZone({
        el: () => (chatVisible() ? layerEl : undefined),
        enabled: () => canDrop() && !!composerApi,
        hover: (over) => setDragOver(over),
        drop: (paths) => {
          void describeLocal(paths)
            .then((files) => {
              if (!files.length) return;
              composerApi?.attachLocal(files);
              composerApi?.focus();
            })
            .catch((error) => setToast(String((error as { message?: string })?.message ?? error)));
        },
      }),
    );
  }
  /** Alt+↑/↓: the previous/next chat of this PC's list (same order as the Chat screen). */
  const switchChat = (delta: 1 | -1) => {
    const o = open();
    if (!o) return;
    const data = chatLists().get(o.target.hostId)?.data;
    const all = chatGroups(o.target.hostId, data, data?.protected ?? []).flatMap((g) => needsYouFirst(g.items)).filter((t) => t.pid !== null);
    const at = all.findIndex((t) => t.pid === o.target.pid);
    const next = adjacentIndex(all.length, at, delta);
    if (next === undefined || next === at) return;
    openChat(all[next]!, o.pcName);
  };
  createEffect(() => {
    if (!chatVisible() || !desktopShell()) return;
    const onKey = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey;
      if (mod && !e.altKey && !e.shiftKey && e.key.toLowerCase() === 'l') {
        e.preventDefault();
        composerApi?.focus();
        return;
      }
      if (e.altKey && !mod && (e.key === 'ArrowUp' || e.key === 'ArrowDown')) {
        if (inTextField(e.target) && !(e.target as HTMLElement).closest('.chat-field')) return;
        e.preventDefault();
        switchChat(e.key === 'ArrowUp' ? -1 : 1);
      }
    };
    window.addEventListener('keydown', onKey);
    onCleanup(() => window.removeEventListener('keydown', onKey));
  });
  // ── UNI-296: the PC's browser beside the chat (pane on the desktop, sheet on the phone) ──
  const pane = browserPane();
  const canBrowser = () => can('portForward');
  const paneShown = () => chatVisible() && desktopShell() && canBrowser() && pane.open() && !!open();
  const openBrowser = () => {
    if (!canBrowser()) return;
    haptic();
    toggleBrowser(desktopShell());
  };
  createEffect(() => {
    if (!chatVisible()) return;
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.shiftKey && !e.altKey && e.key.toLowerCase() === 'b') {
        e.preventDefault();
        openBrowser();
      }
    };
    window.addEventListener('keydown', onKey);
    onCleanup(() => window.removeEventListener('keydown', onKey));
  });
  // Leaving the chat closes the phone's sheet.
  createEffect(() => {
    if (!chatVisible()) pane.setSheet(false);
  });
  const key = () => (open() ? `${open()!.target.hostId}:${open()!.target.pid}` : '');
  const draft = () => drafts()[key()] ?? '';
  const setDraft = (text: string) => setDrafts((d) => ({ ...d, [key()]: text }));

  // "Mention in chat" from the Files screen (UNI-155): fold any pending
  // `@path` mention for this chat into its draft.
  createEffect(() => {
    const k = key();
    if (!k) return;
    const mention = consumeMentionFor(k);
    if (mention) setDraft(draft() ? `${draft()} ${mention}` : mention);
  });

  // Session navigation (UNI-158): header menu, resume/tree sheets, rename,
  // the "pi is busy" confirm, and the long-press message action menu.
  const [headerMenu, setHeaderMenu] = createSignal<DOMRect | null>(null);
  // UNI-269: the working-mode popover (from the header chip, or ⋯ → Working mode).
  const [modeMenu, setModeMenu] = createSignal<DOMRect | null>(null);
  const [resumeOpen, setResumeOpen] = createSignal(false);
  const [treeOpen, setTreeOpen] = createSignal(false);
  const [renameOpen, setRenameOpen] = createSignal(false);
  // Side question sheet (UNI-159): the "+" menu, or typed /btw, /unipi:btw.
  const [btwOpen, setBtwOpen] = createSignal(false);
  const [btwQuestion, setBtwQuestion] = createSignal<string | undefined>(undefined);
  const openBtw = (question?: string) => {
    setBtwQuestion(question);
    setBtwOpen(true);
  };
  const [runConfirm, setRunConfirm] = createSignal<RunRequest | null>(null);
  const [msgMenu, setMsgMenu] = createSignal<{ anchor: DOMRect; text: string; entryId?: string } | null>(null);
  // UNI-207: long-press is native text selection only (no onContextMenu on
  // rows — it used to fight Android's selection gesture). A tap on a
  // message (not a link/code/button/image chip, and with no active text
  // selection) toggles a compact action bar under that row instead; only
  // one row at a time, cleared by scrolling or tapping elsewhere.
  const [selectedKey, setSelectedKey] = createSignal<string | undefined>(undefined);

  /** Runs a session-nav op; on BusyError, offers "Stop the current run and …?". */
  const withBusyConfirm = (title: string, detail: string, run: (force?: boolean) => Promise<void>) => {
    void run().catch((error: unknown) => {
      if (error instanceof BusyError) setRunConfirm({ title, detail, run: () => run(true) });
      else setToast(String((error as { message?: string })?.message ?? error));
    });
  };

  // A separate pi beside this one (same herdr workspace, same folder) — the
  // fast path; "New session" restarts THIS pi's conversation instead.
  const doNewHere = async () => {
    const cur = open();
    if (!cur) return;
    const cwd = s()?.session?.cwd ?? cur.target.cwdFull;
    setToast('Starting a new chat…');
    const res = await newChatHere(cur.target, cwd);
    if (!res.ok) {
      setToast(`Couldn't start a chat here: ${errorWords(res.error).text}`);
      setTimeout(() => setToast(null), 4000);
      return;
    }
    setToast(null);
    openChat(targetFromNew(cur.target.hostId, res.value, folderName(res.value.cwd ?? cwd ?? cur.target.workspaceLabel), cur.target.workspaceLabel), cur.pcName);
  };
  const doNew = () => withBusyConfirm('Start a new session?', 'pi is still working on something. Stop it and start a new session?', (force) => chat()!.sessionNew(force));
  // UNI-308: /reload — "Reloading…" while the op is in flight, "Reloaded" once pi's
  // reconnect (its extensions/skills/prompts re-read) comes back through.
  const doReload = () => {
    setToast('Reloading…');
    void chat()!
      .reload()
      .then(
        () => setToast('Reloaded'),
        (error: unknown) => {
          if (error instanceof BusyError) setRunConfirm({ title: 'Reload now?', detail: 'pi is still working on something. Stop it and reload?', run: () => chat()!.reload(true).then(() => void setToast('Reloaded')) });
          else setToast(String((error as { message?: string })?.message ?? error));
        },
      );
    setTimeout(() => setToast((t) => (t === 'Reloading…' || t === 'Reloaded' ? null : t)), 3000);
  };
  const doResume = (item: SessionsItem) =>
    withBusyConfirm('Resume this session?', 'pi is still working on something. Stop it and resume the other session?', (force) => chat()!.sessionResume(item.path, force).then(() => void setResumeOpen(false)));
  // UNI-290: a jump to a user message puts that message back in the composer (pi's /tree does the
  // same in the TUI editor). An empty draft is simply replaced; anything typed asks first.
  const [replaceConfirm, setReplaceConfirm] = createSignal<RunRequest | null>(null);
  const restoreDraft = (r: TreeGoRestore) => {
    const c = chat();
    if (!c) return;
    const apply = () => {
      const images = r.images.map((image, i) => imageRefFile(c, image, `image-${i + 1}.${(image.mime.split('/')[1] ?? 'png').replace(/[^a-z0-9]/gi, '') || 'png'}`).catch(() => undefined));
      if (composerApi && !s()?.dialogs.length) composerApi.restore(r.text, images);
      else setDraft(r.text);
    };
    restoreIntoDraft(draft(), apply, setReplaceConfirm);
  };
  const doTreeGo = (node: TreeNode, summarize: boolean) =>
    withBusyConfirm('Jump to this point?', 'pi is still working on something. Stop it and jump there?', (force) =>
      chat()!
        .treeGo(node.id, summarize, force)
        .then((restore) => {
          setTreeOpen(false);
          if (restore) restoreDraft(restore);
        }),
    );
  const doFork = (entryId: string) => withBusyConfirm('Fork from here?', 'pi is still working on something. Stop it and fork a new session from here?', (force) => chat()!.sessionFork(entryId, force));

  const rows = createMemo<Row[]>(() => {
    const st = s();
    if (!st) return [];
    const built = buildRows(st.entries, st.streaming, st.tools, st.run.running, new Set(st.seams));
    // Optimistic bubbles for prompts pi has not delivered yet (idle sends).
    // "Sending…" only until pi has it (acked); then a plain bubble until the entry lands.
    for (const p of st.pending) if (p.mode === 'prompt') built.push({ kind: 'user', key: `pending-${p.ref}`, text: p.text, images: [], pending: !p.acked && !p.accepted });
    // Outbox bubbles (UNI-159): not sent at all yet (connection isn't live) — "Waiting for connection", never "Sending…".
    for (const o of st.outbox) built.push({ kind: 'user', key: `outbox-${o.id}`, text: o.text, images: [], pending: true, waiting: true });
    return built;
  });

  /** The last turn failed (an `error` row with no user message after it) and pi is idle: show Retry when /unipi:continue is available. */
  const lastErrorKey = createMemo(() => {
    const list = rows();
    for (let i = list.length - 1; i >= 0; i--) {
      const r = list[i]!;
      if (r.kind === 'user') return undefined;
      if (r.kind === 'error') return r.key;
    }
    return undefined;
  });
  const canRetry = createMemo(() => {
    const st = s();
    if (!st || st.run.running || !lastErrorKey()) return false;
    return st.commands.some((c) => c.name === 'unipi:continue');
  });

  // Android Back closes the chat (back to the list).
  createEffect(() => {
    if (!chatVisible()) return;
    onCleanup(pushBack(() => hideChat()));
  });

  // ── virtualized transcript, anchored to the end ────────────────────
  // Only the rows near the viewport are in the DOM (long sessions stay
  // light). New rows follow the tail while you are at the bottom; older
  // history paged in on scroll-up keeps your place (no jump).
  const [scrollEl, setScrollEl] = createSignal<HTMLDivElement>();
  const virt = createVirtualizer({
    get count() {
      return rows().length;
    },
    get getScrollElement() {
      // Reactive: the scroller mounts after this runs, and is only attached
      // while the layer is visible (a hidden layer measures 0 px, which the
      // virtualizer would keep).
      const el = chatVisible() ? scrollEl() : undefined;
      return () => el ?? null;
    },
    estimateSize: (i) => estimate(rows()[i]),
    getItemKey: (i) => rows()[i]?.key ?? i,
    overscan: 6,
    anchorTo: 'end',
    followOnAppend: true,
    scrollEndThreshold: FOLLOW,
    paddingStart: 14,
    paddingEnd: 18,
  });
  // DOM-based: the running row below the virtual list counts too.
  const atBottom = () => {
    const el = scrollEl();
    return !el || el.scrollHeight - el.scrollTop - el.clientHeight <= FOLLOW;
  };
  const toBottom = (smooth = false) => {
    const el = scrollEl();
    virt.scrollToEnd({ behavior: smooth ? 'smooth' : 'auto' });
    // A second pass once fresh rows are measured, down to the real end
    // (the running row sits below the virtual list).
    requestAnimationFrame(() => el?.scrollTo({ top: el.scrollHeight, behavior: 'auto' }));
  };
  // ── UNI-275: search + navigate to a point ──────────────────────────
  const [searchOpen, setSearchOpen] = createSignal(false);
  /** Row key flashed after a jump (cleared after the highlight fades). */
  const [flashKey, setFlashKey] = createSignal<string | undefined>();
  /** Scroll to session entry `id`, paging older history in up to it — the whole chat stays loaded. */
  const navigateToEntry = async (id: string): Promise<boolean> => {
    const c = chat();
    if (!c) return false;
    const ok = await c.navigateTo(id);
    if (!ok) {
      setToast("That point isn't in this branch any more.");
      setTimeout(() => setToast(null), 3000);
      return false;
    }
    setFollowing(false);
    // Let the paged-in rows render and measure, then jump (twice: estimates settle).
    await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
    const idx = () => rowIndexForEntry(rows(), id, toolResultOwners(s()?.entries ?? []));
    if (idx() < 0) return false;
    virt.scrollToIndex(idx(), { align: 'center' });
    setTimeout(() => idx() >= 0 && virt.scrollToIndex(idx(), { align: 'center' }), 180);
    setFlashKey(rows()[idx()]?.key);
    setTimeout(() => setFlashKey(undefined), 2200);
    return true;
  };
  const pickSearchHit = (hit: SearchHit) => {
    haptic('tick');
    setSearchOpen(false);
    void navigateToEntry(hit.id);
  };
  // Desktop: Ctrl/⌘+F opens chat search while the chat is in front.
  createEffect(() => {
    if (!chatVisible()) return;
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === 'f') {
        e.preventDefault();
        setSearchOpen(true);
      }
    };
    window.addEventListener('keydown', onKey);
    onCleanup(() => window.removeEventListener('keydown', onKey));
  });

  let userScrolling = false;
  const onScroll = () => {
    const el = scrollEl();
    if (!el) return;
    if (trayOpen()) setTrayOpen(false);
    if (selectedKey()) setSelectedKey(undefined);
    // Only your own scrolling decides following (measurement scrolls by the
    // virtualizer settle themselves); reaching the bottom re-engages it.
    // A finger or wheel decides; so does landing near the top (that is the
    // reader going back, even when the last touch already ended).
    if (userScrolling || el.scrollTop < el.clientHeight) setFollowing(atBottom());
    else if (atBottom()) setFollowing(true);
    // Near the top: page in older history.
    if (el.scrollTop < 600 && s()?.hasOlder && !s()?.loadingOlder) chat()?.loadOlder();
  };
  // A page landed while you rest near the top: keep paging (no scroll event fires).
  createEffect(
    on(
      () => s()?.loadingOlder,
      (loading, was) => {
        if (was && !loading) setTimeout(onScroll, 120);
      },
    ),
  );
  // UNI-256: the composer growing (a long paste, a chip row) shrinks the transcript from below
  // without any scroll event — the last lines slid under the composer. Following: stay at the end.
  createEffect(() => {
    const el = scrollEl();
    if (!el || typeof ResizeObserver === 'undefined') return;
    let height = el.clientHeight;
    const ro = new ResizeObserver(() => {
      const next = el.clientHeight;
      if (next < height && following()) el.scrollTop = el.scrollHeight;
      height = next;
    });
    ro.observe(el);
    onCleanup(() => ro.disconnect());
  });
  // Streaming growth inside the last row (no new row): keep the tail in view.
  createEffect(
    on([rows, () => s()?.dialogs.length, () => s()?.run.running, () => s()?.run.waiting], () => {
      if (following()) requestAnimationFrame(() => toBottom());
    }),
  );
  createEffect(
    on([key, () => chat()?.generation()], () => {
      setFollowing(true);
      requestAnimationFrame(() => requestAnimationFrame(() => toBottom()));
    }),
  );
  // Your own send always re-engages following.
  createEffect(
    on(
      () => s()?.pending.length ?? 0,
      (n, prev) => {
        if (prev !== undefined && n > prev) {
          setFollowing(true);
          requestAnimationFrame(() => toBottom(true));
        }
      },
    ),
  );

  // Errors from pi → a short toast.
  createEffect(
    on(
      () => s()?.error?.at,
      (at) => {
        if (!at) return;
        setToast(s()!.error!.message);
        const t = setTimeout(() => setToast(null), 4000);
        onCleanup(() => clearTimeout(t));
      },
    ),
  );

  // pi's ui.notify (a command's "needs an argument", "saved", …) → a toast.
  createEffect(
    on(
      () => s()?.notice?.at,
      (at) => {
        const n = s()?.notice;
        if (!at || !n?.text.trim()) return;
        // UNI-276: a command's reply also lands in the chat (unipi-command-output,
        // pushed right after) — no toast when it's there; wait a beat to see.
        let hide: ReturnType<typeof setTimeout> | undefined;
        const show = setTimeout(() => {
          const inChat = (s()?.entries ?? []).slice(-12).some((e) => {
            const x = e as { type?: string; customType?: string; data?: { text?: unknown } };
            return x.type === 'custom' && x.customType === 'unipi-command-output' && String(x.data?.text ?? '').trim() === n.text.trim().slice(0, 4000);
          });
          if (inChat) return;
          setToast(n.text.split('\n').slice(0, 4).join('\n'));
          hide = setTimeout(() => setToast(null), n.level === 'error' || n.level === 'warning' ? 5000 : 3000);
        }, 600);
        onCleanup(() => (clearTimeout(show), hide && clearTimeout(hide)));
      },
    ),
  );

  // UNI-202/211: an abort pulled pi's own queued messages back — fold them
  // into the draft, joined by blank lines, same as the TUI's own editor
  // restore. Ahead of whatever the user already had typed, same order.
  createEffect(
    on(
      () => s()?.restored?.at,
      (at) => {
        const r = s()?.restored;
        if (!at || !r?.texts.length) return;
        const joined = r.texts.join('\n\n');
        setDraft(draft() ? `${joined}\n\n${draft()}` : joined);
        chat()?.clearRestored();
      },
    ),
  );

  // Copy buttons in code blocks; links open outside.
  const onClick = (e: MouseEvent) => {
    const el = e.target as HTMLElement;
    const copy = el.closest<HTMLButtonElement>('[data-copy]');
    if (copy) {
      const code = copy.closest('.cb')?.querySelector('code')?.textContent ?? '';
      void copyText(code).then(() => {
        copy.textContent = 'Copied';
        haptic('tick');
        setTimeout(() => (copy.textContent = 'Copy'), 1200);
      });
      return;
    }
    const wrapToggle = el.closest<HTMLButtonElement>('[data-wrap-toggle]');
    if (wrapToggle) {
      wrapToggle.closest('.cb')?.classList.toggle('wrapped');
      return;
    }
    const a = el.closest<HTMLAnchorElement>('a[href]');
    if (a) {
      e.preventDefault();
      void openExternal(a.href);
    }
  };

  const toTerminal = () => {
    const o = open();
    if (!o) return;
    haptic();
    setTermTarget({
      hostId: o.target.hostId,
      target: { mux: 'herdr', session: o.target.session, workspace_id: o.target.workspaceId, tab_id: o.target.tabId, pane_id: o.target.paneId },
      label: o.target.workspaceLabel,
      sublabel: o.target.session,
    });
    hideChat();
    markReturnToChat();
    navigate('/term');
  };

  const onCommand = (name: string, args?: string): boolean => {
    if (name === 'files') {
      haptic();
      const cwd = s()?.session?.cwd;
      if (cwd) setFilesTarget({ hostId: open()!.target.hostId, path: args?.trim() || cwd });
      hideChat();
      markReturnToChat();
      navigate('/files');
      return true;
    }
    if (name === 'open' && args) {
      haptic();
      const cwd = s()?.session?.cwd;
      const path = args.trim().startsWith('/') ? args.trim() : cwd ? `${cwd}/${args.trim()}` : args.trim();
      setFilesTarget({ hostId: open()!.target.hostId, path });
      hideChat();
      markReturnToChat();
      navigate('/files');
      return true;
    }
    if (name === 'model' || name === 'thinking') {
      openModelPicker();
      return true;
    }
    if (name === 'compact') {
      haptic();
      const instructions = args?.trim() || undefined;
      chat()?.compact(instructions);
      setToast(instructions ? `Compacting… (${instructions})` : 'Compacting…');
      setTimeout(() => setToast(null), 2500);
      return true;
    }
    if (name === 'new') {
      haptic();
      doNew();
      return true;
    }
    if (name === 'resume') {
      haptic();
      setResumeOpen(true);
      return true;
    }
    if (name === 'tree') {
      haptic();
      setTreeOpen(true);
      return true;
    }
    if (name === 'fork') {
      haptic();
      const leaf = s()?.entries.at(-1)?.id;
      if (leaf) doFork(leaf);
      else setToast('Nothing to fork from yet.');
      return true;
    }
    // UNI-276: the app has its own Progress view — the TUI overlay can't show here.
    if (name === 'unipi:visualize-progress') {
      haptic();
      setProgressOpen(true);
      return true;
    }
    // UNI-276: pi built-ins the app can do itself (a same-named extension command wins).
    const overridden = s()?.commands.some((c) => c.name === name && c.source !== 'builtin');
    // UNI-308: /reload — must never fall through to a plain prompt.
    if (!overridden && name === 'reload') {
      haptic();
      doReload();
      return true;
    }
    if (!overridden && name === 'name') {
      haptic();
      if (args?.trim()) void chat()?.renameSession(args.trim());
      else setRenameOpen(true);
      return true;
    }
    if (!overridden && name === 'copy') {
      haptic();
      const last = [...(s()?.entries ?? [])].reverse().find((e) => (e as { message?: { role?: string } }).message?.role === 'assistant');
      const text = last ? textOf(((last as { message: { content: unknown } }).message.content) as never) : '';
      if (!text.trim()) setToast('No reply to copy yet.');
      else void navigator.clipboard?.writeText(text).then(() => setToast('Copied the last reply'), () => setToast("Couldn't copy"));
      setTimeout(() => setToast(null), 2500);
      return true;
    }
    if (!overridden && name === 'session') {
      haptic();
      openControlCentre('info');
      return true;
    }
    if (!overridden && TERMINAL_ONLY.has(name)) {
      setToast(`/${name} opens a screen only the terminal can show`);
      setTimeout(() => setToast(null), 3000);
      return true;
    }
    return false;
  };

  const runningLabel = () => {
    const st = s();
    return st ? liveActivity(st, st.session?.cwd) : { title: '' };
  };
  // UNI-206: the last transcript row already shows "Running · <cmd>" live
  // (a live ProcessGroup header or streaming prose) — the tail RunningRow
  // below the transcript must not repeat it.
  const lastRowLive = createMemo(() => {
    const list = rows();
    const last = list[list.length - 1];
    if (!last) return false;
    return (last.kind === 'process' && last.live) || (last.kind === 'prose' && last.live);
  });
  const runningCount = () => (chat() ? runningWorkCount(chat()!) : 0);
  const toggleTray = () => {
    haptic('tick');
    setTrayOpen((v) => !v);
  };

  return (
    <Show when={open()}>
      <div
        class="chat-layer"
        ref={layerEl}
        hidden={!chatVisible()}
        data-testid="chat-layer"
        data-drag-over={dragOver() ? '' : undefined}
        onDragEnter={onDragEnter}
        onDragOver={onDragOver}
        onDragLeave={onDragLeave}
        onDrop={onDrop}
      >
        <Show when={dragOver()}>
          <div class="chat-drop" data-testid="chat-drop" aria-hidden="true">
            <span class="chat-drop-icon">
              <Upload size={26} />
            </span>
            <p class="chat-drop-title">Drop to attach</p>
            <p class="chat-drop-hint">Images go in inline; other files upload to {open()!.pcName || 'the PC'} and are mentioned in your message.</p>
          </div>
        </Show>
        <header class="chat-head detail-head">
          <button type="button" class="ghost-icon-button" aria-label="Back to chats" onClick={() => hideChat()}>
            <ChevronLeft size={24} />
          </button>
          <button type="button" class="chat-head-title chat-head-title-btn" onClick={openModelPicker}>
            <span class="chat-head-name truncate">{s()?.session?.name || open()!.target.title}</span>
            <span class="chat-head-sub truncate">
              <span class="pc-dot" data-state={s()?.conn === 'live' ? 'ok' : s()?.conn === 'connecting' ? 'checking' : 'error'} data-live={s()?.conn === 'live' ? 'live' : undefined} />
              {s()?.fusionStatus ? modelHeaderLabel(chat()!) : `${open()!.pcName} · ${open()!.target.workspaceLabel}`}
            </span>
          </button>
          <Show when={s()}>
            <ModeChip chat={chat()!} onOpen={setModeMenu} />
          </Show>
          {/* Desktop (UNI-218 F42): the session strip lives in the header, so the transcript starts right under it. */}
          <Show when={s() && desktopShell()}>
            <FooterStrip chat={chat()!} open={trayOpen()} onToggle={toggleTray} />
          </Show>
          <button
            type="button"
            class="chat-head-menu"
            aria-label="Chat actions"
            aria-expanded={headerMenu() !== null}
            data-running={runningCount() > 0 ? '' : undefined}
            onClick={(e) => setHeaderMenu((e.currentTarget as HTMLElement).getBoundingClientRect())}
          >
            <EllipsisVertical size={20} />
            <Show when={runningCount() > 0}>
              <span class="chat-head-menu-dot" aria-hidden="true" />
            </Show>
          </button>
          <Show when={desktopShell() && canBrowser()}>
            <button
              type="button"
              class="chat-head-menu chat-browser-toggle"
              aria-label={pane.open() ? 'Hide the browser' : 'Show the browser'}
              aria-pressed={pane.open()}
              title={`Browser (${modKey()} Shift B)`}
              data-testid="chat-browser-toggle"
              onClick={openBrowser}
            >
              <Globe size={19} />
            </button>
          </Show>
          <button type="button" class="chat-term-switch" aria-label="Open in terminal" onClick={toTerminal}>
            <SquareTerminal size={18} />
          </button>
        </header>
        <Show when={s()}>
          <Show when={!desktopShell()}>
            <FooterStrip chat={chat()!} open={trayOpen()} onToggle={toggleTray} />
          </Show>
          <FooterTray chat={chat()!} open={trayOpen()} onOpenModel={openModelPicker} onOpenSection={(section) => (setTrayOpen(false), openControlCentre(section))} />
        </Show>

        <Show when={s()?.conn === 'error' || s()?.conn === 'closed'}>
          <div class="chat-banner" role="alert">
            <CircleAlert size={18} class="shrink-0" />
            <span class="min-w-0 flex-1">{s()?.connMessage ?? (s()?.conn === 'closed' ? 'The pi session ended.' : 'Lost the connection.')}</span>
            <button type="button" class="chat-banner-action" onClick={() => chat()?.reconnect()}>
              <RefreshCw size={14} /> Retry
            </button>
          </div>
        </Show>

        <div
          class="chat-scroll"
          ref={setScrollEl}
          onScroll={onScroll}
          onTouchStart={() => (userScrolling = true)}
          onTouchEnd={() => setTimeout(() => (userScrolling = false), 600)}
          onWheel={() => {
            userScrolling = true;
            setTimeout(() => (userScrolling = false), 400);
          }}
          onClick={onClick}
        >
          <Show when={s()?.conn === 'connecting' && !s()?.entries.length}>
            <div class="chat-col">
              <div class="chat-skeleton">
                <span />
                <span />
                <span />
              </div>
            </div>
          </Show>
          <Show when={s()?.conn === 'live' && s()?.entries.length === 0 && !s()?.pending.length && !s()?.run.running}>
            <WelcomeState chat={chat()!} onResume={doResume} onOpenResume={() => setResumeOpen(true)} />
          </Show>
          <div class="chat-virtual" style={{ height: `${virt.getTotalSize()}px` }}>
            <For each={virt.getVirtualItems()}>
              {(item) => {
                const row = () => rows()[item.index];
                // The virtual list reuses this node by position: when a page
                // lands, another row moves into it. Measure again whenever the
                // row changes, even if the height happens to stay the same
                // (otherwise the new row keeps its estimate and rows overlap).
                let node: HTMLDivElement | undefined;
                createEffect(on(() => row()?.key, () => queueMicrotask(() => node?.isConnected && virt.measureElement(node))));
                // UNI-207: selection applies to user/prose rows only (the
                // same rows the old long-press menu covered).
                const canSelect = () => row()?.kind === 'user' || row()?.kind === 'prose';
                const entryIdFor = (): string | undefined => {
                  const r = row();
                  if (!r) return undefined;
                  if (r.kind === 'user') return r.command ? r.command.entryId : r.key.startsWith('pending-') ? undefined : r.key;
                  if (r.kind === 'prose') {
                    const id = r.key.slice(0, r.key.lastIndexOf('-'));
                    return s()?.entries.some((e) => e.id === id) ? id : undefined;
                  }
                  return undefined;
                };
                const textFor = (): string => {
                  const r = row();
                  return r && (r.kind === 'user' || r.kind === 'prose') ? r.text : '';
                };
                const isSelected = () => !!row() && selectedKey() === row()!.key;
                let touchStart: { x: number; y: number } | undefined;
                const onTouchStartRow = (e: TouchEvent) => {
                  if (!canSelect()) return;
                  touchStart = { x: e.touches[0]!.clientX, y: e.touches[0]!.clientY };
                };
                const onTouchEndRow = (e: TouchEvent) => {
                  if (!canSelect() || !touchStart) return;
                  const dx = (e.changedTouches[0]?.clientX ?? touchStart.x) - touchStart.x;
                  const dy = (e.changedTouches[0]?.clientY ?? touchStart.y) - touchStart.y;
                  touchStart = undefined;
                  // Swipe right (mostly horizontal, far enough) = Quote.
                  if (dx > 64 && Math.abs(dy) < 40) {
                    haptic('tick');
                    setDraft(draft() ? `${draft()}\n> ${textFor()}\n` : `> ${textFor()}\n`);
                  }
                };
                // Tap toggles selection: not a link/code/button/image chip,
                // and no active text selection (a finished long-press select
                // leaves `getSelection().isCollapsed === false`, which a tap
                // on the same spot must not clear into the action bar).
                const onClickRow = (e: MouseEvent) => {
                  if (!canSelect() || desktopShell()) return;
                  if (isMsgTapIgnored(e.target as HTMLElement)) return;
                  if (!(window.getSelection()?.isCollapsed ?? true)) return;
                  const k = row()!.key;
                  setSelectedKey((cur) => (cur === k ? undefined : k));
                };
                const actionBar = (hover: boolean) => (
                  <div class="msg-action-bar" data-testid="msg-action-bar" data-hover={hover ? '' : undefined} onClick={(e) => e.stopPropagation()}>
                                <button
                                  type="button"
                                  class="msg-action-btn"
                                  aria-label="Copy"
                                  onClick={() => {
                                    haptic('tick');
                                    void copyText(textFor());
                                    setSelectedKey(undefined);
                                  }}
                                >
                                  <Copy size={16} />
                                </button>
                                <button
                                  type="button"
                                  class="msg-action-btn"
                                  aria-label="Quote"
                                  onClick={() => {
                                    haptic('tick');
                                    setDraft(draft() ? `${draft()}\n> ${textFor()}\n` : `> ${textFor()}\n`);
                                    setSelectedKey(undefined);
                                  }}
                                >
                                  <Quote size={16} />
                                </button>
                                <Show when={entryIdFor()}>
                                  {(id) => (
                                    <button
                                      type="button"
                                      class="msg-action-btn"
                                      aria-label="Start from here"
                                      onClick={() => {
                                        setSelectedKey(undefined);
                                        doTreeGo({ id: id() } as TreeNode, false);
                                      }}
                                    >
                                      <History size={16} />
                                    </button>
                                  )}
                                </Show>
                                <Show when={entryIdFor()}>
                                  {(id) => (
                                    <button
                                      type="button"
                                      class="msg-action-btn"
                                      aria-label="Fork from here"
                                      onClick={() => {
                                        setSelectedKey(undefined);
                                        doFork(id());
                                      }}
                                    >
                                      <SquareStack size={16} />
                                    </button>
                                  )}
                                </Show>
                                <Show when={entryIdFor() && canRetry()}>
                                  <button
                                    type="button"
                                    class="msg-action-btn"
                                    aria-label="Retry"
                                    onClick={() => {
                                      setSelectedKey(undefined);
                                      haptic();
                                      chat()?.send('/unipi:continue');
                                    }}
                                  >
                                    <RotateCcw size={16} />
                                  </button>
                                </Show>
                                <button
                                  type="button"
                                  class="msg-action-btn"
                                  aria-label="More"
                                  onClick={(e) => {
                                    const anchor = (e.currentTarget as HTMLElement).getBoundingClientRect();
                                    setSelectedKey(undefined);
                                    setMsgMenu({ anchor, text: textFor(), entryId: entryIdFor() });
                                  }}
                                >
                                  <Ellipsis size={16} />
                                </button>
                              </div>
                );
                // UNI-117 desktop: right-click a message = its action menu at the cursor (links,
                // file refs and code keep their own menus / native one).
                const onContextMenuRow = (e: MouseEvent) => {
                  if (!desktopShell() || !canSelect()) return;
                  const t = e.target as HTMLElement;
                  if (t.closest('a[href], [data-file-ref], .task-ref, img, button')) return;
                  if (!(window.getSelection()?.isCollapsed ?? true)) return; // let the native menu copy a selection
                  e.preventDefault();
                  setMsgMenu({ anchor: menuAnchor(e, e.currentTarget as HTMLElement), text: textFor(), entryId: entryIdFor() });
                };
                return (
                  <div class="chat-vrow" data-index={item.index} ref={(el) => (node = el)} style={{ top: `${item.start}px` }}>
                    <div class="chat-col">
                      <Show when={item.index === 0}>
                        <HistoryEdge loading={!!s()?.loadingOlder} more={!!s()?.hasOlder} onLoad={() => chat()?.loadOlder()} />
                      </Show>
                      <Show when={row()}>
                        {(r) => (
                          <div
                            class="chat-row-touch"
                            data-selected={isSelected() ? '' : undefined}
                            data-flash={flashKey() === r().key ? '' : undefined}
                            onClick={onClickRow}
                            onContextMenu={onContextMenuRow}
                            onTouchStart={onTouchStartRow}
                            onTouchEnd={onTouchEndRow}
                          >
                            <RowView
                              row={r()}
                              cwd={s()?.session?.cwd}
                              chat={chat()!}
                              onRemoveWaiting={(key) => chat()?.removeOutboxed(key.replace('outbox-', ''))}
                            />
                            <Show when={r().kind === 'error' && r().key === lastErrorKey() && canRetry()}>
                              <button
                                type="button"
                                class="chat-retry-btn"
                                onClick={() => {
                                  haptic();
                                  chat()?.send('/unipi:continue');
                                }}
                              >
                                <RotateCcw size={13} /> Retry
                              </button>
                            </Show>
                            <Show when={isSelected()}>{actionBar(false)}</Show>
                            {/* UNI-117 desktop: the same actions on hover (no tap-to-select). */}
                            <Show when={desktopShell() && canSelect() && !isSelected()}>{actionBar(true)}</Show>
                          </div>
                        )}
                      </Show>
                    </div>
                  </div>
                );
              }}
            </For>
          </div>
          <Show when={s()?.run.running}>
            <div class="chat-col chat-col-tail">
              <RunningRow since={s()?.runStartedAt} label={runningLabel()} compact={lastRowLive()} />
            </div>
          </Show>
          {/* UNI-221: pi is idle but wake-capable work is pending (bg task with
              a wake, background subagent, fusion handoff) — not finished. */}
          <Show when={!s()?.run.running && s()?.run.waiting}>
            <div class="chat-col chat-col-tail" data-pending-work="">
              <RunningRow since={s()?.waitingSince} label={{ title: WORKING_WORD, detail: s()?.run.waiting }} />
            </div>
          </Show>
        </div>

        <Show when={!following()}>
          <button
            type="button"
            class="jump-btn"
            aria-label="Jump to the latest"
            onClick={() => {
              setFollowing(true);
              toBottom(true);
            }}
          >
            <ArrowDown size={18} />
          </button>
        </Show>
        <Show when={toast()}>
          <div class="chat-toast" role="status">
            {toast()}
          </div>
        </Show>

        <div class="chat-bottom">
          <Show
            when={s()?.dialogs.length}
            fallback={
              <Composer
                chat={chat()!}
                draft={draft()}
                setDraft={setDraft}
                onHaptic={haptic}
                onModel={openModelPicker}
                onCommand={onCommand}
                onUnknownCommand={(name) => {
                  setToast(`Unknown command /${name}`);
                  setTimeout(() => setToast(null), 3000);
                }}
                onSideQuestion={openBtw}
                onNewSession={doNew}
                onAddTask={() => addTaskFromChat(open()!.target.hostId, s()?.session?.cwd)}
                onResume={() => setResumeOpen(true)}
                api={(a) => (composerApi = a)}
              />
            }
          >
            <DialogStack dialogs={s()!.dialogs} onAnswer={(id, v) => chat()?.answer(id, v)} onHaptic={() => haptic('tick')} />
          </Show>
        </div>
      </div>
      <ModelPickerSheet open={modelOpen()} onClose={() => setModelOpen(false)} chat={chat()!} onHaptic={() => haptic('tick')} />
      <ProgressSheet open={progressOpen()} onClose={() => setProgressOpen(false)} chat={chat()!} />
      <DreamSheet open={dreamSheetOpen()} onClose={() => setDreamSheetOpen(false)} chat={chat()!} />
      <ControlCentreSheet open={ccOpen()} onClose={() => setCcOpen(false)} chat={chat()!} onHaptic={() => haptic('tick')} initialSection={ccSection()} />
      <Show when={headerMenu()} keyed>
        {(anchor) => (
          <ChatHeaderMenu
            anchor={anchor}
            onDismiss={() => setHeaderMenu(null)}
            onNew={doNew}
            onNewHere={() => void doNewHere()}
            onResume={() => setResumeOpen(true)}
            onTree={() => setTreeOpen(true)}
            onSearch={() => setSearchOpen(true)}
            onRename={() => setRenameOpen(true)}
            canRename
            onOpenSection={(section) => openControlCentre(section)}
            runningCount={runningCount()}
            onProgress={() => setProgressOpen(true)}
            progressLive={progressActive(chat())}
            onMode={() => setModeMenu(anchor)}
            modeHint={chatModeView(chat())?.label ?? 'Plain'}
            onDream={() => setDreamSheetOpen(true)}
            dreamHint={dreamHint(chat())}
            onBoard={() => openBoardFromChat()}
            onAddTask={() => addTaskFromChat(open()!.target.hostId, s()?.session?.cwd)}
            onFilesHere={
              s()?.session?.cwd
                ? () => {
                    setFilesTarget({ hostId: open()!.target.hostId, path: s()!.session!.cwd });
                    hideChat();
                    markReturnToChat();
                    navigate('/files');
                  }
                : undefined
            }
            onGit={s()?.session?.cwd ? () => openGit({ hostId: open()!.target.hostId, path: s()!.session!.cwd!, from: 'chat' }) : undefined}
            gitStat={s()?.session?.cwd ? { hostId: open()!.target.hostId, path: s()!.session!.cwd! } : undefined}
            onCloseChat={() => setCloseChatTarget(open()!.target)}
            onBrowser={canBrowser() ? openBrowser : undefined}
            browserOpen={desktopShell() ? pane.open() : pane.sheet()}
          />
        )}
      </Show>
      <Show when={modeMenu()} keyed>
        {(anchor) => <ModePopover anchor={anchor} chat={chat()!} onDismiss={() => setModeMenu(null)} onProgress={() => setProgressOpen(true)} />}
      </Show>
      <Show when={open()}>
        {(o) => (
          <>
            <BrowserSidePane show={paneShown()} hostId={o().target.hostId} hostName={o().pcName} />
            <BrowserSheet open={pane.sheet() && chatVisible() && canBrowser()} hostId={o().target.hostId} hostName={o().pcName} onClose={() => pane.setSheet(false)} />
          </>
        )}
      </Show>
      <FileRefMenu />
      <ResumeSheet open={resumeOpen()} onClose={() => setResumeOpen(false)} chat={chat()!} onPick={doResume} />
      <TreeSheet open={treeOpen()} onClose={() => setTreeOpen(false)} chat={chat()!} onPick={doTreeGo} onNavigate={(node) => (setTreeOpen(false), void navigateToEntry(node.id))} />
      <SearchSheet open={searchOpen()} onClose={() => setSearchOpen(false)} chat={chat()!} onPick={pickSearchHit} />
      <RenameSheet
        open={renameOpen()}
        onClose={() => setRenameOpen(false)}
        name={s()?.session?.name ?? ''}
        onSave={(name) => void chat()?.renameSession(name)}
      />
      <RunConfirmSheet request={runConfirm()} onClose={() => setRunConfirm(null)} />
      <RunConfirmSheet request={replaceConfirm()} onClose={() => setReplaceConfirm(null)} />
      <CloseChatSheet
        target={closeChatTarget()}
        pcName={open()?.pcName ?? ''}
        running={!!s()?.run.running}
        onClose={() => setCloseChatTarget(null)}
        onDone={(choice) => {
          const t = closeChatTarget();
          setCloseChatTarget(null);
          if (!t) return;
          if (choice === 'hide') hideChatFromList(t.hostId, t);
          closeChat();
          void chatLists().load(t.hostId);
        }}
      />
      <BtwSheet
        open={btwOpen()}
        onClose={() => setBtwOpen(false)}
        chat={chat()!}
        initialQuestion={btwQuestion()}
        onAskInMainChat={(text) => setDraft(draft() ? `${draft()}\n${text}` : text)}
      />
      {/* keyed (UNI-226): `m` is the value, so an item that reads it after dismissing can't hit a stale accessor. */}
      <Show when={msgMenu()} keyed>
        {(m) => (
          <MessageMenu
            anchor={m.anchor}
            onDismiss={() => setMsgMenu(null)}
            text={m.text}
            entryId={m.entryId}
            onHaptic={() => haptic('tick')}
            onQuote={(text) => setDraft(draft() ? `${draft()}\n> ${text}\n` : `> ${text}\n`)}
            onStartFrom={m.entryId ? () => doTreeGo({ id: m.entryId! } as TreeNode, false) : undefined}
            onForkFrom={m.entryId ? () => doFork(m.entryId!) : undefined}
            onRetry={m.entryId && canRetry() ? () => chat()?.send('/unipi:continue') : undefined}
          />
        )}
      </Show>
    </Show>
  );
}
