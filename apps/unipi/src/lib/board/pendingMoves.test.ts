// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api, type Task } from './api';
import { boardStore } from './store';
import { boardUi, resetBoardUi } from './ui';
import { toasts } from '../toasts';
import { flushMoves, isPending, noteKey, onMoveUndone, onMovesSettled, resetPendingMoves, scheduleMove, undoMove, UNDO_MS } from './pendingMoves';

const task = { id: 'CLO-13', title: 'Referral', status: 'in_review', activity: [] } as unknown as Task;
const input = (to = 'done', note = '') => ({ hostId: 'pc', slug: 'cloud', task, to, note });
let move: ReturnType<typeof vi.spyOn>;

beforeEach(() => {
  vi.useFakeTimers();
  toasts().clear();
  move = vi.spyOn(api, 'move').mockResolvedValue({ ...task, status: 'done' } as Task);
  vi.spyOn(boardStore(), 'target').mockReturnValue(undefined);
});
afterEach(() => { resetPendingMoves(); vi.useRealTimers(); vi.restoreAllMocks(); resetBoardUi(); });

describe('the 5 s undo window of Mark done / Rework', () => {
  it('hides the ticket at once, shows "marked done · Undo" and saves after 5 s', async () => {
    const settled = vi.fn();
    onMovesSettled(settled);
    scheduleMove(input());
    expect(isPending('cloud', 'CLO-13')).toBe(true);
    const toast = boardUi().toasts()[0]!;
    expect(toast.message).toBe('CLO-13 marked done');
    expect(toast.action?.label).toBe('Undo');
    await vi.advanceTimersByTimeAsync(UNDO_MS - 1);
    expect(move).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(2);
    expect(move).toHaveBeenCalledWith('pc', 'cloud', 'CLO-13', 'done', undefined);
    expect(isPending('cloud', 'CLO-13')).toBe(false);
    expect(settled).toHaveBeenCalled();
  });

  it('Undo cancels the save, brings the ticket back and puts the note back in its field', async () => {
    const undone = vi.fn();
    onMoveUndone(undone);
    scheduleMove(input('todo', 'fix X'));
    expect(boardUi().toasts()[0]!.message).toBe('CLO-13 sent back to Todo');
    expect(boardUi().notes[noteKey('cloud', 'CLO-13')]).toBe('');
    void boardUi().toasts()[0]!.action!.run();
    expect(isPending('cloud', 'CLO-13')).toBe(false);
    expect(boardUi().notes[noteKey('cloud', 'CLO-13')]).toBe('fix X');
    expect(undone).toHaveBeenCalledWith({ project: 'cloud', task: 'CLO-13', note: 'fix X' });
    await vi.advanceTimersByTimeAsync(UNDO_MS * 2);
    expect(move).not.toHaveBeenCalled();
  });

  it('a rework saves its note as the move comment', async () => {
    scheduleMove(input('todo', '  fix X ![a](att:CLO-13/a.png) '));
    await vi.advanceTimersByTimeAsync(UNDO_MS + 1);
    expect(move).toHaveBeenCalledWith('pc', 'cloud', 'CLO-13', 'todo', 'fix X ![a](att:CLO-13/a.png)');
  });

  it('closing the Workbench (flushMoves) saves what is waiting immediately', async () => {
    scheduleMove(input());
    await flushMoves();
    expect(move).toHaveBeenCalledTimes(1);
    expect(isPending('cloud', 'CLO-13')).toBe(false);
    await vi.advanceTimersByTimeAsync(UNDO_MS * 2);
    expect(move).toHaveBeenCalledTimes(1); // not saved twice
  });

  it('the app going hidden saves it too', async () => {
    scheduleMove(input());
    Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'hidden' });
    document.dispatchEvent(new Event('visibilitychange'));
    await vi.advanceTimersByTimeAsync(0);
    expect(move).toHaveBeenCalledTimes(1);
    Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'visible' });
  });

  it('a move the daemon refuses comes back to the queue with its note and an error toast', async () => {
    move.mockRejectedValueOnce(new Error('E_CONFLICT'));
    const undone = vi.fn();
    onMoveUndone(undone);
    scheduleMove(input('todo', 'fix X'));
    await vi.advanceTimersByTimeAsync(UNDO_MS + 1);
    expect(isPending('cloud', 'CLO-13')).toBe(false);
    expect(undone).toHaveBeenCalledWith({ project: 'cloud', task: 'CLO-13', note: 'fix X' });
    expect(boardUi().notes[noteKey('cloud', 'CLO-13')]).toBe('fix X');
    expect(boardUi().toasts().some((t) => t.kind === 'error' && t.message.startsWith('CLO-13 was not moved'))).toBe(true);
  });

  it('Undo after the save went through does nothing', async () => {
    scheduleMove(input());
    const undo = boardUi().toasts()[0]!.action!;
    await vi.advanceTimersByTimeAsync(UNDO_MS + 1);
    void undo.run();
    undoMove('cloud/CLO-13');
    expect(move).toHaveBeenCalledTimes(1);
  });
});
