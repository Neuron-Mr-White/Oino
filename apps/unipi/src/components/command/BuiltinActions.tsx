/**
 * App-side plumbing for the harness surfaces (WP-A25): the chat ⋯ rows
 * (`Export…`, `Duplicate`), the command-center entries of the capability
 * "Manage…" actions, the `Import session…` dialog and the router handle the
 * non-component code needs (`lib/nav.ts`).
 *
 * Mounted once from the shell (`AppShell`, beside `ModelPickerLayer`), so a
 * palette entry works on any screen and the ⋯ rows exist in every chat.
 */
import { onCleanup } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { Wrench } from 'lucide-solid';
import { registerChatAction } from '../shell/ConversationHeader';
import { registerCommands } from '../../lib/shell';
import { CONSOLIDATE_COMMAND, MANAGE_ACTIONS, chatCanRun, runInChat, runManageActionAnywhere } from '../../lib/settings/manage';
import { LOCAL_HOST } from '../../lib/host/client';
import { useHostsOptional } from '../../hosts/context';
import { exportSession } from '../../lib/session/exportSession';
import { duplicateSession } from '../../lib/session/duplicate';
import { setAppNavigate } from '../../lib/nav';
import { ImportSessionDialog } from './ImportSession';
import { MemoryBrowserLayer, openMemoryBrowserFor } from '../memory/MemoryBrowser';
import { hasGraph, openGraphView } from '../status/GraphView';
import { sessions } from '../../lib/session/store';
import { lhSummary, LH_PROGRESS_TOPIC, LH_STATE_TOPIC, parseLhProgress, parseLhState } from '../../lib/status/lh';

export function BuiltinActions() {
  const navigate = useNavigate();
  const hosts = useHostsOptional();
  onCleanup(setAppNavigate((href, opts) => navigate(href, opts)));

  onCleanup(
    registerChatAction('export', {
      run: (ctx) => void exportSession(ctx.hostId, ctx.sessionId),
    }),
  );
  // The memory extension's save pass for this chat (only where the chat lists the command).
  onCleanup(
    registerChatAction('memory-consolidate', {
      run: (ctx) => void runInChat(ctx.hostId, ctx.sessionId, CONSOLIDATE_COMMAND),
      when: (ctx) => chatCanRun(ctx.hostId, ctx.sessionId, CONSOLIDATE_COMMAND),
    }),
  );
  // [graph-view] "View graph" while this chat has a graph / swarm run (the header chip
  // keeps the LH topics subscribed for the open chat).
  onCleanup(
    registerChatAction('view-graph', {
      run: (ctx) => openGraphView({ hostId: ctx.hostId, sessionId: ctx.sessionId, path: ctx.cwd }),
      when: (ctx) => {
        const topics = sessions().ensure(ctx.hostId, ctx.sessionId).state.topics;
        return hasGraph(lhSummary(parseLhState(topics[LH_STATE_TOPIC]?.data), parseLhProgress(topics[LH_PROGRESS_TOPIC]?.data)));
      },
    }),
  );
  onCleanup(
    registerChatAction('duplicate', {
      run: (ctx) => void duplicateSession(ctx.hostId, ctx.sessionId),
    }),
  );

  // The capability pages' Manage… buttons as palette actions (A25): the same
  // command that opens the extension's panel in the chat.
  onCleanup(
    registerCommands(
      MANAGE_ACTIONS.map((action) => ({
        id: `manage.${action.id}`,
        title: action.title,
        group: action.group,
        keywords: `${action.keywords ?? ''} manage panel`,
        icon: () => <Wrench size={16} />,
        when: () => true,
        run: () => {
          const hostId = hosts?.hosts().find((h) => h.transport === 'local')?.id ?? LOCAL_HOST;
          // Browse memories / All projects: the app's own modal (no chat, paged on the host).
          if (action.id === 'memory.browse' || action.id === 'memory.global') return openMemoryBrowserFor(action.id === 'memory.browse' ? 'project' : 'all', hostId);
          void runManageActionAnywhere(action, { navigate, hostId });
        },
      })),
    ),
  );

  return (
    <>
      <ImportSessionDialog />
      <MemoryBrowserLayer />
    </>
  );
}
