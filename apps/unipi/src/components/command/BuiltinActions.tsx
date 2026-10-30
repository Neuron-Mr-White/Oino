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
import { MANAGE_ACTIONS, runManageAction } from '../../lib/settings/manage';
import { exportSession } from '../../lib/session/exportSession';
import { duplicateSession } from '../../lib/session/duplicate';
import { setAppNavigate } from '../../lib/nav';
import { ImportSessionDialog } from './ImportSession';

export function BuiltinActions() {
  const navigate = useNavigate();
  onCleanup(setAppNavigate((href, opts) => navigate(href, opts)));

  onCleanup(
    registerChatAction('export', {
      run: (ctx) => void exportSession(ctx.hostId, ctx.sessionId),
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
        run: () => void runManageAction(action, { navigate }),
      })),
    ),
  );

  return <ImportSessionDialog />;
}
