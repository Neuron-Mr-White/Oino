import { TaskDetail } from '../board/TaskDetail';
import { CommentSheet, ConfirmSheet } from '../board/MoveSheets';
import { ImageViewer, ReportReader } from '../board/Viewers';
import { useBoardLive } from '../../lib/board/live';
import { boardStore } from '../../lib/board/store';
import { taskOrigin } from '../../lib/board/taskOrigin';

/**
 * The Kanboard task detail, mounted over the Superagent page (OIN-40): a
 * ticket opened from the Review / Needs-you queue never leaves this route, so
 * Back lands on the queue exactly as it was. It brings what the detail needs
 * from the Board screen — the note / confirm sheets, the report reader, the
 * image viewer and the live board feed.
 */
export function ReviewTaskLayer() {
  const store = boardStore();
  useBoardLive(store, () => {
    const origin = taskOrigin();
    const at = store.target();
    return origin && at?.hostId === origin.hostId ? { hostId: origin.hostId, slug: at.slug ?? null } : null;
  });
  return <>
    <TaskDetail />
    <CommentSheet />
    <ConfirmSheet />
    <ReportReader />
    <ImageViewer />
  </>;
}
