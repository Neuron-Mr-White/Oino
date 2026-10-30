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
import type { Chat, SessionsItem, TreeNode } from '../../lib/chat/store';
import type { Section } from './ControlCentre';
import { BusyError } from '../../lib/chat/store';
import { desktopShell } from '../../lib/layout';
import { modKey } from '../../lib/shell';
import { BoardSheet } from '../board/BoardSheet';
import { AnchoredMenu, MenuRow, MenuSeparator } from '../board/AnchoredMenu';
import { copyText } from '../../lib/board/share';
import '../git/git.css';
import { cachedMenuStat, compactCount, refreshMenuStat, type GitMenuStat } from '../../lib/git';

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
export function relativeTimeMs(ms: number | undefined, now = Date.now()): string {
  if (!ms) return '';
  const seconds = Math.max(0, Math.round((now - ms) / 1000));
  if (seconds < 45) return 'just now';
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.round(hours / 24)}d ago`;
}

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

/** Session tree sheet: every branch, current path highlighted, current leaf marked. */

/** Max indent levels drawn before the depth is compressed into a badge (UNI-171). */
const TREE_MAX_DEPTH = 6;

/** One row of the flattened tree: pi's /tree layout (UNI-186). */
export type TreeRow = {
  node: TreeNode;
  /** Display indent (levels). */
  depth: number;
  /** One cell per level: '│' gutter, '├' / '└' connector, or '' (blank). */
  cells: string[];
};

/**
 * Port of pi's TUI tree layout (tree-selector.js flattenTree + render,
 * UNI-186): pre-order, the branch holding the current leaf first among
 * siblings; a fork's children step in +1 and so does the first
 * generation after a fork; single-child chains stay flat; several roots
 * hang off a virtual root. Connectors (├ └) mark each fork child and
 * gutters (│) carry a still-open sibling branch down past it.
 */
export function flattenTree(nodes: TreeNode[]): TreeRow[] {
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const children = new Map<string, TreeNode[]>();
  const roots: TreeNode[] = [];
  for (const n of nodes) {
    const parent = n.parentId ? byId.get(n.parentId) : undefined;
    if (!parent) roots.push(n);
    else children.set(parent.id, [...(children.get(parent.id) ?? []), n]);
  }
  // Which subtrees hold the current leaf (iterative post-order, cycle-safe).
  const hasActive = new Map<string, boolean>();
  const order: TreeNode[] = [];
  const seen = new Set<string>();
  const pre = [...roots];
  while (pre.length) {
    const n = pre.pop()!;
    if (seen.has(n.id)) continue;
    seen.add(n.id);
    order.push(n);
    for (const c of children.get(n.id) ?? []) pre.push(c);
  }
  for (let i = order.length - 1; i >= 0; i--) {
    const n = order[i]!;
    hasActive.set(n.id, n.current || (children.get(n.id) ?? []).some((c) => hasActive.get(c.id)));
  }
  const activeFirst = (list: TreeNode[]) => [...list.filter((c) => hasActive.get(c.id)), ...list.filter((c) => !hasActive.get(c.id))];

  type Gutter = { position: number; show: boolean };
  type Item = [TreeNode, number, boolean, boolean, boolean, Gutter[], boolean];
  const multipleRoots = roots.length > 1;
  const stack: Item[] = [];
  const orderedRoots = activeFirst(roots);
  for (let i = orderedRoots.length - 1; i >= 0; i--) {
    stack.push([orderedRoots[i]!, multipleRoots ? 1 : 0, multipleRoots, multipleRoots, i === orderedRoots.length - 1, [], multipleRoots]);
  }
  const out: TreeRow[] = [];
  const emitted = new Set<string>();
  while (stack.length) {
    const [node, indent, justBranched, showConnector, isLast, gutters, virtualRootChild] = stack.pop()!;
    if (emitted.has(node.id)) continue; // malformed (cyclic) host data
    emitted.add(node.id);
    const depth = multipleRoots ? Math.max(0, indent - 1) : indent;
    const connector = showConnector && !virtualRootChild;
    const cells: string[] = [];
    for (let level = 0; level < depth; level++) {
      const gutter = gutters.find((g) => g.position === level);
      if (gutter) cells.push(gutter.show ? '│' : '');
      else if (connector && level === depth - 1) cells.push(isLast ? '└' : '├');
      else cells.push('');
    }
    out.push({ node, depth, cells });
    const kids = activeFirst(children.get(node.id) ?? []);
    const forks = kids.length > 1;
    const childIndent = forks || (justBranched && indent > 0) ? indent + 1 : indent;
    const childGutters = connector ? [...gutters, { position: Math.max(0, depth - 1), show: !isLast }] : gutters;
    for (let i = kids.length - 1; i >= 0; i--) {
      stack.push([kids[i]!, childIndent, forks, forks, i === kids.length - 1, childGutters, false]);
    }
  }
  return out;
}

/** pi /tree filter modes (tree-selector.js filterMode; Ctrl+D/T/U/L/A in the TUI). */
export type TreeFilter = 'default' | 'no-tools' | 'user-only' | 'labeled-only' | 'all';
export const TREE_FILTERS: Array<{ id: TreeFilter; label: string }> = [
  { id: 'default', label: 'Default' },
  { id: 'no-tools', label: 'No tools' },
  { id: 'user-only', label: 'User' },
  { id: 'labeled-only', label: 'Labeled' },
  { id: 'all', label: 'All' },
];
const SETTINGS_KINDS = new Set(['label', 'context_edit', 'custom', 'model_change', 'thinking_level_change', 'session_info']);
const TREE_FILTER_KEY = 'unipi.chat.treeFilter';
export function readTreeFilter(): TreeFilter {
  try {
    const v = localStorage.getItem(TREE_FILTER_KEY);
    return TREE_FILTERS.some((f) => f.id === v) ? (v as TreeFilter) : 'default';
  } catch {
    return 'default';
  }
}
function rememberTreeFilter(mode: TreeFilter): void {
  try {
    localStorage.setItem(TREE_FILTER_KEY, mode);
  } catch {
    // ignore
  }
}

/**
 * pi's /tree filter (UNI-194), then hidden rows' children re-attach to
 * their nearest visible ancestor so the layout (flattenTree) stays a
 * real tree — as pi's recalculateVisualStructure does. Like pi: usage
 * entries never show, an assistant row with no text (tool calls only)
 * hides unless it is the current leaf, and the current leaf always shows.
 */
export function filterTree(nodes: TreeNode[], mode: TreeFilter): TreeNode[] {
  const keep = (n: TreeNode): boolean => {
    if (n.kind === 'usage') return false;
    if (n.current) return true;
    if (n.kind === 'assistant' && !n.preview.trim() && mode !== 'all') return false;
    const settings = SETTINGS_KINDS.has(n.kind);
    switch (mode) {
      case 'user-only':
        return n.kind === 'user';
      case 'no-tools':
        return !settings && n.kind !== 'toolResult';
      case 'labeled-only':
        return n.label !== undefined;
      case 'all':
        return true;
      default:
        return !settings;
    }
  };
  if (mode === 'all') return nodes.filter((n) => n.kind !== 'usage');
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const visible = new Set(nodes.filter(keep).map((n) => n.id));
  const out: TreeNode[] = [];
  for (const n of nodes) {
    if (!visible.has(n.id)) continue;
    let parent = n.parentId ? byId.get(n.parentId) : undefined;
    const guard = new Set<string>();
    while (parent && !visible.has(parent.id) && !guard.has(parent.id)) {
      guard.add(parent.id);
      parent = parent.parentId ? byId.get(parent.parentId) : undefined;
    }
    out.push(parent && visible.has(parent.id) ? (n.parentId === parent.id ? n : { ...n, parentId: parent.id }) : n.parentId === null ? n : { ...n, parentId: null });
  }
  return out;
}

export function TreeSheet(props: {
  open: boolean;
  onClose: () => void;
  chat: Chat;
  onPick: (node: TreeNode, summarize: boolean) => void;
  /** UNI-275: scroll the chat to this entry (on the current branch) with the whole chat loaded. */
  onNavigate?: (node: TreeNode) => void;
}) {
  /** UNI-275: a row on the current branch offers "Go to this point" vs "Start from here". */
  const [rowMenu, setRowMenu] = createSignal<{ node: TreeNode; anchor: DOMRect } | null>(null);
  const [nodes, setNodes] = createSignal<TreeNode[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [summarize, setSummarize] = createSignal(true);
  const [loadError, setLoadError] = createSignal<string | undefined>();
  let listEl: HTMLDivElement | undefined;
  createEffect(() => {
    if (!props.open) return;
    setLoading(true);
    setLoadError(undefined);
    void props.chat.tree().catch((error: unknown) => {
      setLoadError(error instanceof Error ? error.message : String(error));
      return [] as TreeNode[];
    }).then((list) => {
      setNodes(list);
      setLoading(false);
      // Keep the current leaf in view when the sheet opens (UNI-171).
      queueMicrotask(() => listEl?.querySelector<HTMLElement>('[data-current]')?.scrollIntoView?.({ block: 'center' }));
    });
  });
  const [filter, setFilter] = createSignal<TreeFilter>(readTreeFilter());
  const pickFilter = (mode: TreeFilter) => {
    setFilter(mode);
    rememberTreeFilter(mode);
  };
  const rows = createMemo(() => flattenTree(filterTree(nodes(), filter())));
  return (
    <BoardSheet
      open={props.open}
      onClose={props.onClose}
      label="Session tree"
      testId="tree-sheet"
      footer={
        <label class="tree-summarize">
          <input type="checkbox" checked={summarize()} onChange={(e) => setSummarize(e.currentTarget.checked)} />
          Summarize the branch I leave
        </label>
      }
    >
      <div class="tree-filters" role="radiogroup" aria-label="Tree filter">
        <For each={TREE_FILTERS}>
          {(f) => (
            <button type="button" role="radio" class="tree-filter" aria-checked={filter() === f.id} onClick={() => pickFilter(f.id)}>
              {f.label}
            </button>
          )}
        </For>
      </div>
      <Show when={!loading()} fallback={<p class="sheet-label">Loading…</p>}>
        <Show when={rows().length > 0} fallback={<p class="sheet-label">{loadError() ? `Couldn't load the tree: ${loadError()}` : nodes().length === 0 ? 'This session has no entries yet' : 'Nothing matches this filter'}</p>}>
        <div class="model-list tree-list" ref={listEl}>
          <For each={rows()}>
            {(row) => {
              const node = row.node;
              const raw = row.depth;
              // Deep rows keep only their last levels so the text stays reachable.
              const cells = row.cells.slice(-TREE_MAX_DEPTH);
              return (
                <button
                  type="button"
                  class="model-row tree-row"
                  data-on-path={node.onPath ? '' : undefined}
                  data-current={node.current ? '' : undefined}
                  data-depth={raw}
                  aria-current={node.current ? 'true' : undefined}
                  onClick={(e) => {
                    // Off the current branch there is nothing on screen to go to: switch to it.
                    if (!props.onNavigate || !node.onPath || node.current) return props.onPick(node, summarize());
                    setRowMenu({ node, anchor: (e.currentTarget as HTMLElement).getBoundingClientRect() });
                  }}
                >
                  <Show when={cells.length > 0}>
                    <span class="tree-gutter shrink-0" aria-hidden="true">
                      <For each={cells}>{(cell) => <span class="tree-cell" data-cell={cell || undefined} />}</For>
                    </span>
                  </Show>
                  <span class="min-w-0 flex-1 text-left">
                    <span class="block truncate">{node.preview || `(${node.kind})`}</span>
                    <span class="block truncate model-row-sub">
                      {node.kind}
                      {node.label ? ` · ${node.label}` : ''}
                    </span>
                  </span>
                  <Show when={raw > TREE_MAX_DEPTH}>
                    <span class="tree-deep-badge" title={`${raw} branches deep`}>↳{raw}</span>
                  </Show>
                  <Show when={node.current}>
                    <Check size={18} class="board-accent shrink-0" />
                  </Show>
                </button>
              );
            }}
          </For>
        </div>
        </Show>
      </Show>
      <Show when={rowMenu()} keyed>
        {(m) => (
          <AnchoredMenu anchor={m.anchor} width={260} label="Tree point" onDismiss={() => setRowMenu(null)}>
            <MenuRow
              icon={<CornerDownRight size={18} />}
              label="Go to this point"
              onSelect={() => (setRowMenu(null), props.onNavigate?.(m.node))}
            />
            <MenuRow
              icon={<GitBranch size={18} />}
              label="Start a new branch here"
              onSelect={() => (setRowMenu(null), props.onPick(m.node, summarize()))}
            />
          </AnchoredMenu>
        )}
      </Show>
    </BoardSheet>
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
