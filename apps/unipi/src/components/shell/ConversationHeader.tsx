/**
 * Conversation header (Z-2, WP-A3): 56 px. Left: the session title (fade mask
 * on overflow, full title in the tooltip) over "project · PC". Title click
 * does **not** open the model picker (Z-70). Right: the chips slot (mode chip,
 * A14/A16), ⋯ chat-scoped actions, terminal toggle (Cmd/Ctrl+J) and side-pane
 * toggle (Cmd/Ctrl+Alt+B) — icon-only like ZCode's header.
 *
 * ⋯ actions come from one registry; the built-ins (copy id / path / log path)
 * are here, the rest are plugged in by their WPs (rename/fork/export/reveal:
 * A5, session info: A17) and stay hidden until then:
 *
 *   registerChatAction('rename', { run: (ctx) => openRename(ctx) });
 */
import { createEffect, createSignal, For, Show, type JSX } from 'solid-js';
import { ArrowLeft, Brain, Copy, CopyPlus, Ellipsis, FileText, Folder, FolderInput, FolderLock, FolderOpen, GitFork, Info, PanelLeftOpen, PanelRight, Pencil, Search, Share, SquarePen, SquareTerminal } from 'lucide-solid';
import { AnchoredMenu, MenuRow, MenuSeparator } from '../board/AnchoredMenu';
import { observeTitleOverflow, syncTitleOverflow } from '../sidebar/SessionRow';
import { copyText } from '../../lib/board/share';
import { layout } from '../../lib/layout';
import { modKey } from '../../lib/shell';
import { openFind } from '../../lib/find';
import { can } from '../../lib/platform';
import { ShellNeedsYouBell } from '../NeedsYouBell';
import { setNeedsYouOpen } from '../../lib/needsYouOpen';
import type { IconComponent } from '../../lib/panes/kinds';

export type ChatActionId = 'rename' | 'fork' | 'duplicate' | 'export' | 'move-workspace' | 'copy-id' | 'copy-path' | 'copy-log' | 'reveal' | 'memory-consolidate' | 'info';

export type ChatActionCtx = { hostId: string; sessionId: string; title: string; cwd?: string; file?: string };

export type ChatAction = { run: (ctx: ChatActionCtx) => void; when?: (ctx: ChatActionCtx) => boolean };

/** The ⋯ menu, in order (Z-2). `group` draws a separator between groups. */
export const CHAT_ACTIONS: Array<{ id: ChatActionId; label: string; icon: IconComponent; group: number }> = [
  { id: 'rename', label: 'Rename…', icon: Pencil, group: 0 },
  { id: 'fork', label: 'Fork', icon: GitFork, group: 0 },
  { id: 'duplicate', label: 'Duplicate', icon: CopyPlus, group: 0 },
  { id: 'export', label: 'Export…', icon: Share, group: 0 },
  // A No-workspace (scratch) chat only: one way to a real folder.
  { id: 'move-workspace', label: 'Move to a workspace…', icon: FolderInput, group: 0 },
  { id: 'copy-id', label: 'Copy session id', icon: Copy, group: 1 },
  { id: 'copy-path', label: 'Copy path', icon: Folder, group: 1 },
  { id: 'copy-log', label: 'Copy log path', icon: FileText, group: 1 },
  { id: 'reveal', label: 'Reveal folder', icon: FolderOpen, group: 1 },
  // Owner 2026-11-03: moved out of Settings › Memory — it writes this chat's memories.
  { id: 'memory-consolidate', label: 'Consolidate memories', icon: Brain, group: 2 },
  { id: 'info', label: 'Session info', icon: Info, group: 2 },
];

const BUILTIN: Partial<Record<ChatActionId, ChatAction>> = {
  'copy-id': { run: (ctx) => void copyText(ctx.sessionId) },
  'copy-path': { run: (ctx) => void copyText(ctx.cwd ?? ''), when: (ctx) => !!ctx.cwd },
  'copy-log': { run: (ctx) => void copyText(ctx.file ?? ''), when: (ctx) => !!ctx.file },
};

const [actions, setActions] = createSignal<Partial<Record<ChatActionId, ChatAction>>>({ ...BUILTIN });

/** Plugs a ⋯ action in (replacing the current one). Returns the unplug. */
export function registerChatAction(id: ChatActionId, action: ChatAction): () => void {
  const prev = actions()[id];
  setActions((cur) => ({ ...cur, [id]: action }));
  return () =>
    setActions((cur) => {
      if (cur[id] !== action) return cur;
      const next = { ...cur };
      if (prev) next[id] = prev;
      else delete next[id];
      return next;
    });
}

/** The ⋯ rows available for this session, in menu order. */
export function chatActionsFor(ctx: ChatActionCtx): Array<(typeof CHAT_ACTIONS)[number] & { action: ChatAction }> {
  const all = actions();
  return CHAT_ACTIONS.flatMap((row) => {
    const action = all[row.id];
    return action && (action.when?.(ctx) ?? true) ? [{ ...row, action }] : [];
  });
}

/** Sidebar reopen + New chat, shown at the header's start while the sidebar is collapsed (ZCode). */
export function SidebarReveal(props: { onNewChat: () => void }) {
  return (
    <div class="sb-reveal" data-testid="sidebar-reveal">
      <button type="button" class="shell-icon-btn" aria-label="Show sidebar" title={`Show sidebar (${modKey()} B)`} onClick={() => layout().toggleSidebar(true)}>
        <PanelLeftOpen size={17} />
      </button>
      <button type="button" class="shell-icon-btn" aria-label="New chat" title="New chat" onClick={() => props.onNewChat()}>
        <SquarePen size={16} />
      </button>
      <Show when={can('hosts')}>
        <ShellNeedsYouBell onOpen={() => setNeedsYouOpen(true)} />
      </Show>
    </div>
  );
}

export function ConversationHeader(props: {
  title: string;
  /** Project folder (full path) and its short form. */
  cwd?: string;
  project?: string;
  /** Shown for a paired PC (not This computer). */
  hostName?: string;
  /** A No-workspace chat (its private scratch folder): the folder line offers "Move to a workspace…". */
  scratch?: boolean;
  /** The session the ⋯ menu acts on (none in a draft). */
  ctx?: ChatActionCtx;
  /** Mode chip etc. (A14 / A16). */
  chips?: JSX.Element;
  /** U10: the phone ⋯ menu's "Panes" group (sidePane/bottomPane entries). */
  extraMenu?: (close: () => void) => JSX.Element;
  /** Phone: back to the list. */
  onBack?: () => void;
  onNewChat: () => void;
  desktop: boolean;
}) {
  const [menu, setMenu] = createSignal<DOMRect | undefined>();
  const [folderMenu, setFolderMenu] = createSignal<DOMRect | undefined>();
  const rows = () => (props.ctx ? chatActionsFor(props.ctx) : []);
  /** The folder line's rows on a scratch chat: Move to a workspace… / Reveal scratch folder. */
  const folderRows = () => (props.scratch ? rows().filter((r) => r.id === 'move-workspace' || r.id === 'reveal') : []);
  const hasExtra = () => !!props.extraMenu;
  const sub = () => [props.project, props.hostName].filter(Boolean).join(' · ');
  let titleEl: HTMLHeadingElement | undefined;
  createEffect(() => {
    void props.title;
    if (titleEl && typeof ResizeObserver !== 'undefined') queueMicrotask(() => titleEl && syncTitleOverflow(titleEl));
  });
  return (
    <header class="conv-head" data-testid="conv-head">
      <Show when={props.onBack}>
        <button type="button" class="ghost-icon-button" aria-label="Back" onClick={() => props.onBack?.()}>
          <ArrowLeft size={20} />
        </button>
      </Show>
      <Show when={props.desktop && !layout().sidebarOpen() && !layout().fullView()}>
        <SidebarReveal onNewChat={props.onNewChat} />
      </Show>
      <div class="conv-head-text">
        <h1
          ref={(el) => {
            titleEl = el;
            observeTitleOverflow(el);
          }}
          class="conv-title"
          title={props.title}
          data-testid="conv-title"
        >
          {props.title}
        </h1>
        <Show when={sub()}>
          <Show
            when={folderRows().length > 0}
            fallback={
              <p class="conv-sub" title={props.cwd}>
                <Show when={props.scratch} fallback={<Folder size={12} />}>
                  <FolderLock size={12} />
                </Show>
                <span>{sub()}</span>
              </p>
            }
          >
            <button
              type="button"
              class="conv-sub conv-sub-btn"
              title={`Runs in a private scratch folder: ${props.cwd ?? ''}`}
              aria-haspopup="menu"
              aria-expanded={!!folderMenu()}
              onClick={(e) => setFolderMenu(e.currentTarget.getBoundingClientRect())}
              data-testid="conv-folder"
            >
              <FolderLock size={12} />
              <span>{sub()}</span>
            </button>
          </Show>
        </Show>
      </div>
      <div class="conv-head-tools">
        {props.chips}
        <Show when={rows().length > 0 || hasExtra()}>
          <button type="button" class="shell-icon-btn" aria-label="Chat actions" aria-haspopup="menu" aria-expanded={!!menu()} onClick={(e) => setMenu(e.currentTarget.getBoundingClientRect())} data-testid="conv-more">
            <Ellipsis size={18} />
          </button>
        </Show>
        <Show when={props.desktop}>
          <button
            type="button"
            class="shell-icon-btn"
            aria-label="Terminal panel"
            aria-pressed={layout().bottomOpen()}
            title={`Terminal panel (${modKey()} J)`}
            onClick={() => layout().toggleBottom()}
            data-testid="toggle-bottom"
          >
            <SquareTerminal size={17} />
          </button>
          <button
            type="button"
            class="shell-icon-btn"
            aria-label="Side pane"
            aria-pressed={layout().sideOpen()}
            title={`Side pane (${modKey()} Alt B)`}
            onClick={() => layout().toggleSide()}
            data-testid="toggle-side"
          >
            <PanelRight size={17} />
          </button>
        </Show>
      </div>
      <Show when={folderMenu()}>
        <AnchoredMenu anchor={folderMenu()!} width={260} label="Workspace" onDismiss={() => setFolderMenu(undefined)}>
          <For each={folderRows()}>
            {(row) => {
              const Icon = row.id === 'reveal' ? FolderOpen : row.icon;
              return <MenuRow icon={<Icon size={18} />} label={row.id === 'reveal' ? 'Reveal scratch folder' : row.label} onSelect={() => (setFolderMenu(undefined), row.action.run(props.ctx!))} />;
            }}
          </For>
        </AnchoredMenu>
      </Show>
      {/* ctx-gated rows hide in a draft; the phone's ⋯ Panes group (A29)
          still opens there (Files/Git don't need a session). */}
      <Show when={menu()}>
        <AnchoredMenu anchor={menu()!} alignEnd width={240} label="Chat actions" onDismiss={() => setMenu(undefined)}>
          {/* Find is a menu entry too (Z-79): the phone has no Cmd/Ctrl+F. */}
          <Show when={props.ctx}>
            <MenuRow icon={<Search size={18} />} label="Find in chat" onSelect={() => (setMenu(undefined), openFind())} />
          </Show>
          <Show when={props.extraMenu}>{(lead) => <>{lead()(() => setMenu(undefined))}</>}</Show>          <For each={rows()}>
            {(row, i) => {
              const Icon = row.icon;
              return (
                <>
                  <Show when={i() > 0 && rows()[i() - 1]!.group !== row.group}>
                    <MenuSeparator />
                  </Show>
                  <MenuRow icon={<Icon size={18} />} label={row.label} onSelect={() => (setMenu(undefined), row.action.run(props.ctx!))} />
                </>
              );
            }}
          </For>
        </AnchoredMenu>
      </Show>
    </header>
  );
}
