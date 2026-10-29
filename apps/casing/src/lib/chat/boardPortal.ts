/**
 * Chat → board side of the portal (UNI-181): hide the chat, mark the
 * return, hand the Board screen a jump and switch to its tab. The Board
 * returns to this chat when the task pane / New task sheet closes.
 */
import { hideChat, markReturnToChat } from './open';
import { requestBoardJump } from '../board/portal';

function goBoard(): void {
  hideChat();
  markReturnToChat();
  // Hash route switch (works outside a router-scoped component, e.g. MdBlock).
  if (typeof location !== 'undefined') location.hash = '#/board';
}

/** Opens a task's detail pane on the board. */
export function openTaskFromChat(hostId: string, taskId: string, cwd?: string): void {
  requestBoardJump({ hostId, taskId, cwd, fromChat: true });
  goBoard();
}

/** Opens the board's New task sheet (project picked from the chat's cwd). */
export function addTaskFromChat(hostId: string, cwd?: string): void {
  requestBoardJump({ hostId, newTask: true, cwd, fromChat: true });
  goBoard();
}

/** Just the board, no pane (header menu "Kanboard"). */
export function openBoardFromChat(): void {
  hideChat();
  if (typeof location !== 'undefined') location.hash = '#/board';
}
