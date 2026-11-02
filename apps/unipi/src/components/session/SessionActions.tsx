/**
 * WP-A5's app-side layer (Z-15/Z-16): plugs the header ⋯ "Rename…" and "Fork"
 * rows in — the two `CHAT_ACTIONS` ids A3 reserved for this WP that stayed
 * dead until now — and mounts the row context menu (`SessionMenu`) plus the
 * Rename and Move-to-a-workspace dialogs, once per shell, so every chat has the rows and both shells
 * share one menu.
 */
import { onCleanup } from 'solid-js';
import { registerChatAction } from '../shell/ConversationHeader';
import { forkSession, revealFolder } from '../../lib/session/actions';
import { isScratchCwd } from '../../lib/session/selectors';
import { hubs } from '../../lib/session/hub';
import { sessions } from '../../lib/session/store';
import { openRenameDialog, RenameDialog } from './RenameDialog';
import { SessionMenu } from '../sidebar/SessionMenu';
import { MoveWorkspaceDialog, openMoveWorkspaceDialog } from './MoveWorkspaceDialog';

export function SessionActions() {
  onCleanup(
    registerChatAction('rename', {
      run: (ctx) => {
        const current = sessions().peek(ctx.hostId, ctx.sessionId)?.state.name ?? hubs().peek(ctx.hostId)?.state.cards[ctx.sessionId]?.name ?? ctx.title;
        openRenameDialog({ hostId: ctx.hostId, sessionId: ctx.sessionId, name: current });
      },
    }),
  );
  onCleanup(
    registerChatAction('fork', {
      run: (ctx) => void forkSession(ctx.hostId, ctx.sessionId),
    }),
  );
  // UniPi Oino: a No-workspace chat (its private scratch folder) moves to a real folder, one way.
  onCleanup(
    registerChatAction('move-workspace', {
      run: (ctx) => openMoveWorkspaceDialog({ hostId: ctx.hostId, sessionId: ctx.sessionId, cwd: ctx.cwd }),
      when: (ctx) => isScratchCwd(ctx.cwd),
    }),
  );
  onCleanup(
    // The scratch folder's way out to the Files pane (its folder line + ⋯).
    registerChatAction('reveal', {
      run: (ctx) => revealFolder(ctx.hostId, ctx.cwd),
      when: (ctx) => isScratchCwd(ctx.cwd),
    }),
  );
  return (
    <>
      <SessionMenu />
      <RenameDialog />
      <MoveWorkspaceDialog />
    </>
  );
}
