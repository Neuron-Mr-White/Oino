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
import { createSignal, For, Show, type JSX } from 'solid-js';
import { ArrowLeft, Copy, Ellipsis, FileText, Folder, FolderOpen, GitFork, Info, PanelLeftOpen, PanelRight, Pencil, Share, SquarePen, SquareTerminal } from 'lucide-solid';
import { AnchoredMenu, MenuRow, MenuSeparator } from '../board/AnchoredMenu';
import { copyText } from '../../lib/board/share';
import { layout } from '../../lib/layout';
import { modKey } from '../../lib/shell';
import type { IconComponent } from '../../lib/panes/kinds';

export type ChatActionId = 'rename' | 'fork' | 'export' | 'copy-id' | 'copy-path' | 'copy-log' | 'reveal' | 'info';

export type ChatActionCtx = { hostId: string; sessionId: string; title: string; cwd?: string; file?: string };

export type ChatAction = { run: (ctx: ChatActionCtx) => void; when?: (ctx: ChatActionCtx) => boolean };

/** The ⋯ menu, in order (Z-2). `group` draws a separator between groups. */
export const CHAT_ACTIONS: Array<{ id: ChatActionId; label: string; icon: IconComponent; group: number }> = [
  { id: 'rename', label: 'Rename…', icon: Pencil, group: 0 },
  { id: 'fork', label: 'Fork', icon: GitFork, group: 0 },
  { id: 'export', label: 'Export…', icon: Share, group: 0 },
  { id: 'copy-id', label: 'Copy session id', icon: Copy, group: 1 },
  { id: 'copy-path', label: 'Copy path', icon: Folder, group: 1 },
  { id: 'copy-log', label: 'Copy log path', icon: FileText, group: 1 },
  { id: 'reveal', label: 'Reveal folder', icon: FolderOpen, group: 1 },
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
  /** The session the ⋯ menu acts on (none in a draft). */
  ctx?: ChatActionCtx;
  /** Mode chip etc. (A14 / A16). */
  chips?: JSX.Element;
  /** Phone: back to the list. */
  onBack?: () => void;
  onNewChat: () => void;
  desktop: boolean;
}) {
  const [menu, setMenu] = createSignal<DOMRect | undefined>();
  const rows = () => (props.ctx ? chatActionsFor(props.ctx) : []);
  const sub = () => [props.project, props.hostName].filter(Boolean).join(' · ');
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
        <h1 class="conv-title" title={props.title} data-testid="conv-title">
          {props.title}
        </h1>
        <Show when={sub()}>
          <p class="conv-sub" title={props.cwd}>
            <Folder size={12} />
            <span>{sub()}</span>
          </p>
        </Show>
      </div>
      <div class="conv-head-tools">
        {props.chips}
        <Show when={rows().length > 0}>
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
      <Show when={menu() && props.ctx}>
        <AnchoredMenu anchor={menu()!} alignEnd width={240} label="Chat actions" onDismiss={() => setMenu(undefined)}>
          <For each={rows()}>
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
