/**
 * Session navigation UI (UNI-158): the chat header's ⋯ menu (New session,
 * Resume…, Session tree, Rename), the Resume and Session tree sheets, the
 * message long-press action menu, and the "pi is busy" confirm used by all
 * of them. Visual style matches the rest of chat/board (BoardSheet,
 * AnchoredMenu).
 */
import { createEffect, createMemo, createSignal, For, Show } from 'solid-js';
import {
  Camera,
  Check,
  Clock,
  Copy,
  CornerDownRight,
  FilePlus2,
  Files,
  GitBranch,
  Gauge,
  Globe,
  History,
  Image as ImageIcon,
  Info as InfoIcon,
  ListChecks,
  ListTree,
  Search,
  MessageCircleQuestion,
  Paperclip,
  Pencil,
  Quote,
  RotateCcw,
  Share2,
  Sparkles,
  SquareStack,
  Zap,
  Columns3,
  ListPlus,
  Network,
  Workflow,
  SquarePlus,
} from 'lucide-solid';
import type { Chat, SessionsItem } from '../../lib/chat/store';
import type { Section } from './ControlCentre';
import { BusyError } from '../../lib/chat/store';
import { desktopShell } from '../../lib/layout';
import { modKey } from '../../lib/shell';
import { BoardSheet } from '../board/BoardSheet';
import { AnchoredMenu, MenuRow, MenuSeparator } from '../board/AnchoredMenu';
import { copyText } from '../../lib/board/share';
import '../git/git.css';
import { cachedMenuStat, compactCount, refreshMenuStat, type GitMenuStat } from '../../lib/git';
import { relativeTimeMs } from '../../lib/time';

/** UNI-257: "+120 −34" beside the Git row — last known value at once, then
 *  refreshed from the host every time the menu opens (mount). Nothing when
 *  clean, not a repo, or an older host. */
export function GitStatHint(props: { hostId: string; path: string }) {
  const [stat, setStat] = createSignal<GitMenuStat | null | undefined>(cachedMenuStat(props.hostId, props.path));
  void refreshMenuStat(props.hostId, props.path).then(setStat, () => setStat(null));
  return (
    <Show when={stat()}>
      {(s) => (
        <span class="git-stat chat-menu-git-stat" data-testid="chat-menu-git-stat" aria-label={`${s().add} lines added, ${s().del} removed`}>
          <span class="add">+{compactCount(s().add)}{s().capped ? '+' : ''}</span>
          <span class="del">−{compactCount(s().del)}</span>
        </span>
      )}
    </Show>
  );
}

/** "3m ago", "2h ago", "5d ago" — short, tabular-friendly (ms since epoch, unlike board's ISO relativeTime). */
export { relativeTimeMs } from '../../lib/time';

export type RunRequest = { title: string; detail: string; run: () => Promise<void>; /** The confirm button (default "Stop and continue"). */ confirm?: string };

/** "Stop the current run and …?" confirm — shown when a session-nav request answers `busy`. */
export function RunConfirmSheet(props: { request: RunRequest | null; onClose: () => void }) {
  const [busy, setBusy] = createSignal(false);
  const close = () => !busy() && props.onClose();
  const confirm = async () => {
    const r = props.request;
    if (!r || busy()) return;
    setBusy(true);
    try {
      await r.run();
    } finally {
      setBusy(false);
      props.onClose();
    }
  };
  return (
    <BoardSheet
      open={props.request !== null}
      onClose={close}
      label={props.request?.title ?? 'Stop the current run?'}
      testId="run-confirm-sheet"
      footer={
        <div class="flex gap-3">
          <button type="button" class="pair-secondary flex-1" onClick={close}>
            Cancel
          </button>
          <button type="button" class="pair-primary flex-[1.4]" disabled={busy()} onClick={() => void confirm()}>
            {busy() ? 'Working…' : (props.request?.confirm ?? 'Stop and continue')}
          </button>
        </div>
      }
    >
      <p class="move-why mt-0">{props.request?.detail}</p>
    </BoardSheet>
  );
}

/** The chat header's ⋯ menu. */
export function ChatHeaderMenu(props: {
  anchor: DOMRect;
  onDismiss: () => void;
  onNew: () => void;
  /** A separate session beside this one, in the same folder. */
  onNewHere?: () => void;
  onTree: () => void;
  /** UNI-275: search this chat's text. */
  onSearch?: () => void;
  onRename: () => void;
  canRename: boolean;
  /** "Files here" (UNI-155): opens the Files screen at this chat's cwd, when known. */
  onFilesHere?: () => void;
  /** UNI-217: opens the Git view for this chat's cwd, when known. */
  onGit?: () => void;
  /** UNI-257: where to read the Git row's +/- from (the chat's PC + cwd). */
  gitStat?: { hostId: string; path: string };
  /** UNI-181 portal: jump to the board / add a task there. */
  onBoard?: () => void;
  onAddTask?: () => void;
  /** UNI-188: Running / Context / Session info open the Control Centre at that section; `runningCount` draws the Running row's "(n)" hint. */
  onOpenSection?: (section: Section) => void;
  runningCount?: number;
  /** UNI-222: opens the long-horizon Progress sheet; `progressLive` draws a "live" hint while a run is going. */
  onProgress?: () => void;
  progressLive?: boolean;
  /** UNI-269: opens the working-mode popover; `modeHint` = the current mode's name ("Plain" when none). */
  onMode?: () => void;
  modeHint?: string;
  /** UNI-296: the PC's browser beside the chat (pane / sheet); `browserOpen` marks it shown. */
  onBrowser?: () => void;
  browserOpen?: boolean;
}) {
  // Built once per open (MenuRow reads `hint` twice; a getter would mount it twice).
  const gitStat = props.gitStat;
  const gitHint = gitStat ? <GitStatHint hostId={gitStat.hostId} path={gitStat.path} /> : undefined;
  return (
    <AnchoredMenu anchor={props.anchor} alignEnd width={240} label="Chat actions" onDismiss={props.onDismiss}>
      <Show when={props.onOpenSection}>
        <>
            <MenuRow
              icon={<ListChecks size={18} />}
              label="Running"
              hint={(props.runningCount ?? 0) > 0 ? <span class="board-menu-note">{props.runningCount}</span> : undefined}
              onSelect={() => (props.onDismiss(), props.onOpenSection?.('running'))}
            />
            <MenuRow icon={<Gauge size={18} />} label="Context" onSelect={() => (props.onDismiss(), props.onOpenSection?.('context'))} />
            <MenuRow icon={<InfoIcon size={18} />} label="Session info" onSelect={() => (props.onDismiss(), props.onOpenSection?.('info'))} />
            <Show when={props.onMode}>
              <MenuRow
                icon={<Workflow size={18} />}
                label="Working mode"
                hint={props.modeHint ? <span class="board-menu-note">{props.modeHint}</span> : undefined}
                onSelect={() => (props.onDismiss(), props.onMode?.())}
              />
            </Show>
            <Show when={props.onProgress}>
              <MenuRow
                icon={<Network size={18} />}
                label="Progress"
                hint={props.progressLive ? <span class="board-menu-note">live</span> : undefined}
                onSelect={() => (props.onDismiss(), props.onProgress?.())}
              />
            </Show>
            <MenuSeparator />
        </>
      </Show>
      <MenuRow icon={<FilePlus2 size={18} />} label="New session" onSelect={() => (props.onDismiss(), props.onNew())} />
      <Show when={props.onNewHere}>
        <MenuRow icon={<SquarePlus size={18} />} label="New chat in this folder" onSelect={() => (props.onDismiss(), props.onNewHere?.())} />
      </Show>
      <Show when={props.onSearch}>
        <MenuRow icon={<Search size={18} />} label="Search in chat" onSelect={() => (props.onDismiss(), props.onSearch?.())} />
      </Show>
      <MenuRow icon={<ListTree size={18} />} label="Session tree" onSelect={() => (props.onDismiss(), props.onTree())} />
      <Show when={props.onFilesHere}>
        <MenuRow icon={<Files size={18} />} label="Files here" onSelect={() => (props.onDismiss(), props.onFilesHere?.())} />
      </Show>
      <Show when={props.onBrowser}>
        <MenuRow
          icon={<Globe size={18} />}
          label={props.browserOpen ? 'Hide browser' : 'Browser'}
          hint={desktopShell() ? <span class="board-menu-note">{modKey()} ⇧ B</span> : undefined}
          onSelect={() => (props.onDismiss(), props.onBrowser?.())}
        />
      </Show>
      <Show when={props.onGit}>
        <MenuRow
          icon={<GitBranch size={18} />}
          label="Git"
          hint={gitHint}
          onSelect={() => (props.onDismiss(), props.onGit?.())}
        />
      </Show>
      <Show when={props.onBoard}>
        <MenuSeparator />
        <MenuRow icon={<Columns3 size={18} />} label="Kanboard" onSelect={() => (props.onDismiss(), props.onBoard?.())} />
        <MenuRow icon={<ListPlus size={18} />} label="Add task…" onSelect={() => (props.onDismiss(), props.onAddTask?.())} />
      </Show>
      <Show when={props.canRename}>
        <MenuSeparator />
        <MenuRow icon={<Pencil size={18} />} label="Rename…" onSelect={() => (props.onDismiss(), props.onRename())} />
      </Show>
    </AnchoredMenu>
  );
}

/** Long-press message action menu: Copy, Quote, Start from here, Fork from here, Retry, Share. */
export function MessageMenu(props: {
  anchor: DOMRect;
  onDismiss: () => void;
  text: string;
  entryId?: string;
  onQuote: (text: string) => void;
  onStartFrom?: () => void;
  onForkFrom?: () => void;
  onRetry?: () => void;
  onHaptic: () => void;
}) {
  const share = async (text: string) => {
    const nav = navigator as Navigator & { share?: (data: { text: string }) => Promise<void> };
    if (typeof nav.share === 'function') {
      try {
        await nav.share({ text });
        return;
      } catch {
        // fall through to copy
      }
    }
    void copyText(text);
  };
  // UNI-226: snapshot everything BEFORE onDismiss(). The caller renders this
  // menu inside `<Show when={msgMenu()}>{(m) => …}`: dismissing unmounts it, and
  // reading a prop afterwards (props.text → m().text) threw "Stale read from
  // <Show>", so every item of this full menu did nothing (the icon action bar,
  // which reads its own row, kept working).
  const pick = (run: (snap: { text: string; startFrom?: () => void; forkFrom?: () => void; retry?: () => void }) => void) => () => {
    const snap = { text: props.text, startFrom: props.onStartFrom, forkFrom: props.onForkFrom, retry: props.onRetry };
    props.onDismiss();
    run(snap);
  };
  return (
    <AnchoredMenu anchor={props.anchor} width={224} label="Message actions" onDismiss={props.onDismiss}>
      <MenuRow
        icon={<Copy size={18} />}
        label="Copy"
        onSelect={pick((snap) => {
          props.onHaptic();
          void copyText(snap.text);
        })}
      />
      <MenuRow
        icon={<Quote size={18} />}
        label="Quote"
        onSelect={pick((snap) => {
          props.onHaptic();
          props.onQuote(snap.text);
        })}
      />
      <Show when={props.onStartFrom}>
        <MenuRow icon={<History size={18} />} label="Start from here" onSelect={pick((snap) => snap.startFrom?.())} />
      </Show>
      <Show when={props.onForkFrom}>
        <MenuRow icon={<SquareStack size={18} />} label="Fork from here" onSelect={pick((snap) => snap.forkFrom?.())} />
      </Show>
      <Show when={props.onRetry}>
        <MenuRow icon={<RotateCcw size={18} />} label="Retry" onSelect={pick((snap) => snap.retry?.())} />
      </Show>
      <MenuRow icon={<Share2 size={18} />} label="Share" onSelect={pick((snap) => void share(snap.text))} />
    </AnchoredMenu>
  );
}

/** Rename prompt (tiny inline sheet — a single text field + Save). */
export function RenameSheet(props: { open: boolean; onClose: () => void; name: string; onSave: (name: string) => void }) {
  const [value, setValue] = createSignal(props.name);
  createEffect(() => {
    if (props.open) setValue(props.name);
  });
  return (
    <BoardSheet
      open={props.open}
      onClose={props.onClose}
      label="Rename session"
      testId="rename-sheet"
      footer={
        <button
          type="button"
          class="pair-primary w-full"
          disabled={!value().trim()}
          onClick={() => {
            props.onSave(value().trim());
            props.onClose();
          }}
        >
          Save
        </button>
      }
    >
      <input class="model-search" placeholder="Session name" value={value()} onInput={(e) => setValue(e.currentTarget.value)} autofocus />
    </BoardSheet>
  );
}

/** Welcome state for a brand-new, empty session: up to 5 recent sessions to resume. */
export function WelcomeState(props: { chat: Chat; onResume: (item: SessionsItem) => void }) {
  const [recent, setRecent] = createSignal<SessionsItem[]>([]);
  createEffect(() => {
    void props.chat.sessions('cwd').then(({ items }) => setRecent(items.filter((i) => !i.current).slice(0, 5)));
  });
  return (
    <div class="chat-welcome">
      <Sparkles size={28} class="chat-welcome-icon" />
      <p class="chat-welcome-title">New session</p>
      <p class="chat-welcome-sub">Say what you need, or pick up where you left off.</p>
      <Show when={recent().length > 0}>
        <div class="model-list chat-welcome-list">
          <For each={recent()}>
            {(item) => (
              <button type="button" class="model-row resume-row" onClick={() => props.onResume(item)}>
                <span class="min-w-0 flex-1 text-left">
                  <span class="block truncate">{item.name || item.firstMessage || '(empty session)'}</span>
                  <span class="block truncate model-row-sub">
                    {relativeTimeMs(item.modified)} · {item.messageCount} {item.messageCount === 1 ? 'message' : 'messages'}
                  </span>
                </span>
              </button>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}

/** The composer "+" menu: Photo, Camera, File, Side question, New session
 *  (resuming lives in the sessions sidebar, Z-19). */
export function PlusMenu(props: {
  anchor: DOMRect;
  onDismiss: () => void;
  onPhoto: () => void;
  onCamera: () => void;
  onFile: () => void;
  onSideQuestion: () => void;
  onNewSession: () => void;
  /** UNI-181: create a board task from the chat. */
  onAddTask?: () => void;
}) {
  return (
    <AnchoredMenu anchor={props.anchor} width={232} label="Add" onDismiss={props.onDismiss}>
      <MenuRow icon={<ImageIcon size={18} />} label="Photo" onSelect={() => (props.onDismiss(), props.onPhoto())} />
      <MenuRow icon={<Camera size={18} />} label="Camera" onSelect={() => (props.onDismiss(), props.onCamera())} />
      <MenuRow icon={<Paperclip size={18} />} label="File" hint={<span class="board-menu-note">1 GB max</span>} onSelect={() => (props.onDismiss(), props.onFile())} />
      <MenuSeparator />
      <MenuRow icon={<MessageCircleQuestion size={18} />} label="Side question" onSelect={() => (props.onDismiss(), props.onSideQuestion())} />
      <Show when={props.onAddTask}>
        <MenuRow icon={<ListPlus size={18} />} label="Board task…" onSelect={() => (props.onDismiss(), props.onAddTask?.())} />
      </Show>
      <MenuSeparator />
      <MenuRow icon={<FilePlus2 size={18} />} label="New session" onSelect={() => (props.onDismiss(), props.onNewSession())} />
    </AnchoredMenu>
  );
}

/** The Send chevron menu (UNI-159): Steer / Send now / After it ends. Long-press on
 *  Send opens the same menu (replacing the old long-press = follow-up gesture). */
export function SendModeMenu(props: {
  anchor: DOMRect;
  onDismiss: () => void;
  running: boolean;
  onPick: (mode: 'steer' | 'now' | 'after') => void;
}) {
  return (
    <AnchoredMenu anchor={props.anchor} alignEnd width={272} label="Send options" onDismiss={props.onDismiss}>
      <Show when={props.running}>
        <MenuRow icon={<CornerDownRight size={18} />} label="Steer" hint={<span class="board-menu-note">next boundary</span>} onSelect={() => (props.onDismiss(), props.onPick('steer'))} />
        <MenuRow icon={<Zap size={18} />} label="Send now" hint={<span class="board-menu-note">stops run</span>} onSelect={() => (props.onDismiss(), props.onPick('now'))} />
      </Show>
      <MenuRow icon={<Clock size={18} />} label="After it ends" hint={<span class="board-menu-note">queue</span>} onSelect={() => (props.onDismiss(), props.onPick('after'))} />
    </AnchoredMenu>
  );
}

export { BusyError };
