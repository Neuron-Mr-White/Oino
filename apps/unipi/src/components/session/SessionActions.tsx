/**
 * WP-A5's app-side layer (Z-15/Z-16): plugs the header ⋯ "Rename…" and "Fork"
 * rows in — the two `CHAT_ACTIONS` ids A3 reserved for this WP that stayed
 * dead until now — and mounts the row context menu (`SessionMenu`) plus the
 * Rename dialog, once per shell, so every chat has the rows and both shells
 * share one menu.
 */
import { onCleanup } from 'solid-js';
import { registerChatAction } from '../shell/ConversationHeader';
import { forkSession } from '../../lib/session/actions';
import { openRenameDialog, RenameDialog } from './RenameDialog';
import { SessionMenu } from '../sidebar/SessionMenu';

export function SessionActions() {
  onCleanup(
    registerChatAction('rename', {
      run: (ctx) => openRenameDialog({ hostId: ctx.hostId, sessionId: ctx.sessionId, name: ctx.title }),
    }),
  );
  onCleanup(
    registerChatAction('fork', {
      run: (ctx) => void forkSession(ctx.hostId, ctx.sessionId),
    }),
  );
  return (
    <>
      <SessionMenu />
      <RenameDialog />
    </>
  );
}
