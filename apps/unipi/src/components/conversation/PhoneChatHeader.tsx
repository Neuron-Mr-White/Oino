/**
 * The phone chat header (phone spec §3.4, PH-25 / PH-18): one 56 px row —
 * Back · title over "<project> · <PC>" (running → "Working…") · ⋯. The mode
 * button, the skills strip and the git chip of the desktop header leave it:
 * everything else lives in the ⋯ action sheet, in three sections:
 *
 *   Open    Terminal · Files · Git changes · Side chat · Conversation tree
 *   Chat    Mode · Working mode (long-horizon, when on) · Context & info · Search in chat
 *   Manage  Rename · Fork · Share / export · Archive
 *
 * Desktop-only rows (Panes label, Duplicate, Copy id/path/log path, Reveal,
 * Move workspace) are not offered; the session id lives in Context & info.
 */
import { createEffect, createMemo, createResource, createSignal, getOwner, on, onCleanup, Show } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import {
  Archive,
  ArrowLeft,
  Ellipsis,
  Folder,
  GitBranch,
  GitFork,
  Info,
  ListTree,
  MessageCircleQuestion,
  Pencil,
  Search,
  Share,
  ShieldCheck,
  SquareTerminal,
  Workflow,
} from 'lucide-solid';
import { sessions } from '../../lib/session/store';
import { hubs } from '../../lib/session/hub';
import { isRunning } from '../../lib/session/selectors';
import { archiveSession } from '../../lib/session/actions';
import { git, compactCount } from '../../lib/git';
import { openFind } from '../../lib/find';
import { setFilesTarget } from '../../lib/filesTarget';
import { composerDraft, setComposerText } from '../../lib/composer/drafts';
import { focusComposer } from '../../lib/timeline/actions';
import { lhSummary, LH_EXTENSION_ID, LH_PROGRESS_TOPIC, LH_STATE_TOPIC, parseLhProgress, parseLhState } from '../../lib/status/lh';
import { parseExtensions } from '../session/modalData';
import { STARTER_HINT, STARTER_ORDER, withStarter } from '../../lib/status/modeStarter';
import { openStatusSection } from '../status/StatusPane';
import { openBtwTab, openTreeTab } from '../panes/tabs/sessionTabs';
import { openPhoneTerminal } from '../panes/tabs/terminalPhone';
import { openSessionModal } from '../session/modalTarget';
import { statusSegments } from '../shell/StatusStrip';
import { chatActionsFor, type ChatActionCtx } from '../shell/ConversationHeader';
import { openModeSheet, permissionModeLabel } from '../composer/PermissionControl';
import { ActionSheet, IconButton, type ActionItem } from '../phone';
import './phoneChat.css';

/** "17 skills ready" from the session's status segments → "17 skills". */
export function skillsValue(texts: readonly string[]): string | undefined {
  for (const t of texts) {
    const m = /(\d+)\s+skills?\b/i.exec(t);
    if (m) return `${m[1]} skill${m[1] === '1' ? '' : 's'}`;
  }
  return undefined;
}

export function PhoneChatHeader(props: {
  hostId: string;
  sessionId?: string;
  title: string;
  /** Project basename (never a path), or "No workspace". */
  project?: string;
  hostName?: string;
  cwd?: string;
  ctx?: ChatActionCtx;
  onBack: () => void;
}) {
  const navigate = useNavigate();
  const owner = getOwner();
  const [menu, setMenu] = createSignal(false);
  const [lhMenu, setLhMenu] = createSignal(false);
  const session = createMemo(() => (props.sessionId ? sessions().ensure(props.hostId, props.sessionId) : undefined));
  createEffect(
    on(session, (s) => {
      if (!s) return;
      const release = s.acquire();
      s.subscribeTopics([LH_STATE_TOPIC, LH_PROGRESS_TOPIC]);
      onCleanup(release);
    }),
  );
  const card = () => (props.sessionId ? hubs().ensure(props.hostId).state.cards[props.sessionId] : undefined);
  const running = () => !!props.sessionId && isRunning(card(), session()?.state);
  // A draft says where the chat will run ("on <PC>", §3.6); a chat says its project · PC.
  const sub = () => (running() ? 'Working…' : !props.sessionId ? (props.hostName ? `on ${props.hostName}` : '') : [props.project, props.hostName].filter(Boolean).join(' · '));

  // ── values shown in the sheet ──
  const [gitValue, setGitValue] = createSignal<string | undefined>();
  createEffect(
    on(menu, (open) => {
      if (!open) return;
      const path = props.cwd || '~';
      void (async () => {
        const repo = await git.repo(props.hostId, path);
        if (!repo.ok || !repo.value.repo) return setGitValue(undefined);
        const stat = await git.shortstat(props.hostId, repo.value.root ?? path);
        const s = stat.ok && stat.value.repo ? stat.value : undefined;
        const name = repo.value.branch || repo.value.head || 'no branch';
        setGitValue(s && (s.add || s.del) ? `${name} +${compactCount(s.add)} −${compactCount(s.del)}` : name);
      })().catch(() => setGitValue(undefined));
    }),
  );
  const skills = () => skillsValue(statusSegments(`${props.hostId}/${props.sessionId ?? 'draft'}`).map((s) => s.text));
  // Long-horizon on for this chat (the desktop header's mode chip, asked once the stream is live).
  const [lhOn] = createResource(
    () => (session()?.state.conn === 'live' ? session() : undefined),
    async (s) => !!parseExtensions(await s.cmd<unknown>({ type: '_oino/extensions' }).catch(() => undefined)).find((e) => e.id === LH_EXTENSION_ID && e.enabled),
  );
  const lhEnabled = () => lhOn.latest;
  const lh = createMemo(() => {
    const s = session();
    if (!s || !lhEnabled()) return undefined;
    return lhSummary(parseLhState(s.state.topics[LH_STATE_TOPIC]?.data), parseLhProgress(s.state.topics[LH_PROGRESS_TOPIC]?.data)) ?? null;
  });
  const hasPermission = () => !!session()?.state.commands.some((c) => c.name === 'permission');

  const actionRow = (id: 'rename' | 'fork' | 'export') => (props.ctx ? chatActionsFor(props.ctx).find((r) => r.id === id) : undefined);
  const openCtx = () => ({ hostId: props.hostId, sessionId: props.sessionId, cwd: props.cwd });

  const sections = (): ActionItem[][] => {
    const sid = props.sessionId;
    const open: ActionItem[] = [
      { id: 'terminal', label: 'Terminal', icon: SquareTerminal, onSelect: () => openPhoneTerminal(openCtx(), owner) },
      {
        id: 'files',
        label: 'Files',
        icon: Folder,
        onSelect: () => {
          if (props.cwd) setFilesTarget({ hostId: props.hostId, path: props.cwd });
          navigate(props.cwd ? `/files?path=${encodeURIComponent(props.cwd)}` : '/files');
        },
      },
      { id: 'git', label: 'Git changes', icon: GitBranch, value: gitValue(), onSelect: () => openStatusSection({ hostId: props.hostId, sessionId: sid, path: props.cwd || '~' }) },
    ];
    if (sid) {
      open.push(
        { id: 'side-chat', label: 'Side chat', icon: MessageCircleQuestion, onSelect: () => openBtwTab({ hostId: props.hostId, sessionId: sid, cwd: props.cwd }) },
        { id: 'tree', label: 'Conversation tree', icon: ListTree, onSelect: () => openTreeTab({ hostId: props.hostId, sessionId: sid, cwd: props.cwd }) },
      );
    }
    const chat: ActionItem[] = [];
    if (sid && hasPermission()) chat.push({ id: 'mode', label: 'Mode', icon: ShieldCheck, value: permissionModeLabel(session()!), onSelect: () => openModeSheet(session()!.key) });
    if (sid && lh() !== undefined) chat.push({ id: 'working-mode', label: 'Working mode', icon: Workflow, value: lh()?.label ?? 'Plain', onSelect: () => setLhMenu(true) });
    if (sid) {
      chat.push(
        { id: 'info', label: 'Context & info', icon: Info, value: skills(), onSelect: () => openSessionModal({ hostId: props.hostId, sessionId: sid, tab: 'context' }) },
        { id: 'find', label: 'Search in chat', icon: Search, onSelect: () => openFind() },
      );
    }
    const manage: ActionItem[] = [];
    const ctx = props.ctx;
    if (ctx) {
      const rename = actionRow('rename');
      const fork = actionRow('fork');
      const exp = actionRow('export');
      if (rename) manage.push({ id: 'rename', label: 'Rename', icon: Pencil, onSelect: () => rename.action.run(ctx) });
      if (fork) manage.push({ id: 'fork', label: 'Fork', icon: GitFork, onSelect: () => fork.action.run(ctx) });
      manage.push({
        id: 'export',
        label: 'Share / export',
        icon: Share,
        onSelect: () => (exp ? exp.action.run(ctx) : void import('../../lib/session/exportSession').then((m) => m.exportSession(ctx.hostId, ctx.sessionId))),
      });
      if (card()) {
        manage.push({
          id: 'archive',
          label: 'Archive',
          icon: Archive,
          onSelect: () => {
            void archiveSession(ctx.hostId, ctx.sessionId).then(() => navigate('/chat', { replace: true }));
          },
        });
      }
    }
    return [open, chat, manage];
  };

  const lhSections = (): ActionItem[][] => {
    const s = session();
    if (!s) return [];
    const start = (mode: (typeof STARTER_ORDER)[number]) => {
      setComposerText(s.key, withStarter(composerDraft(s.key).text, mode));
      focusComposer(s.key);
    };
    const starters: ActionItem[] = STARTER_ORDER.map((mode) => ({
      id: `mode-start-${mode}`,
      label: mode[0]!.toUpperCase() + mode.slice(1),
      description: STARTER_HINT[mode],
      checked: lh()?.mode === mode ? true : undefined,
      onSelect: () => start(mode),
    }));
    const plain: ActionItem[] = lh()
      ? [{ id: 'mode-plain', label: 'Plain chat', description: 'Stop the mode and run prompts plainly', onSelect: () => void s.send('/regular', { mode: 'prompt' }) }]
      : [];
    return [starters, plain];
  };

  return (
    <header class="conv-head ph-chat-head" data-testid="conv-head">
      <IconButton icon={ArrowLeft} label="Back" onClick={() => props.onBack()} testId="conv-back" />
      <div class="ph-chat-head-text">
        <h1 class="ph-chat-title" data-testid="conv-title">
          {props.title}
        </h1>
        <Show when={sub()}>
          <p class="ph-chat-sub" data-running={running() ? '' : undefined} data-testid="conv-sub">
            {sub()}
          </p>
        </Show>
      </div>
      <IconButton icon={Ellipsis} label="Chat actions" onClick={() => setMenu(true)} testId="conv-more" />
      <ActionSheet open={menu()} onClose={() => setMenu(false)} title={props.title} sections={sections()} testId="chat-actions-sheet" />
      <ActionSheet open={lhMenu()} onClose={() => setLhMenu(false)} title="Working mode" subtitle={lh() ? `${lh()!.label} mode is on` : 'Plain chat'} sections={lhSections()} testId="working-mode-sheet" />
    </header>
  );
}
