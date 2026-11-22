// @vitest-environment jsdom
/**
 * `lib/fileSummary` (WP-A27): the pure helpers the footer and the edit menu
 * share (status order, totals, labels) and the A7 capability
 * (`fileSummaryCapability`) — its cache read and its refusal to force an undo
 * that conflicts with newer work (Z-28).
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SessionHandle } from './session/store';
import {
  FileUndoConflict,
  canUndoFiles,
  canUndoSummary,
  fileCountLabel,
  fileSummaryCapability,
  isUndone,
  resetFileSummaries,
  rowStatLabel,
  setTurnSummary,
  statLabel,
  statusOf,
  totalsOf,
  type TurnFileRecord,
  type TurnSummary,
} from './fileSummary';

const file = (path: string, over: Partial<TurnFileRecord> = {}): TurnFileRecord => ({
  path,
  existed: true,
  beforeSha: 'aa',
  afterSha: 'bb',
  add: 1,
  del: 1,
  ...over,
});

const summary = (files: TurnFileRecord[], over: Partial<TurnSummary> = {}): TurnSummary => ({
  turnEntryId: 'u1',
  files,
  undone: false,
  undonePaths: [],
  running: false,
  ...over,
});

beforeEach(() => resetFileSummaries());
afterEach(() => {
  resetFileSummaries();
  vi.restoreAllMocks();
});

describe('statusOf', () => {
  it('follows the extension order: skipped, undone, new, deleted, modified', () => {
    expect(statusOf(file('a', { skipped: 'too_large' }), true)).toBe('skipped');
    expect(statusOf(file('a'), true)).toBe('undone');
    expect(statusOf(file('a', { existed: false, beforeSha: undefined }), false)).toBe('new');
    expect(statusOf(file('a', { afterSha: undefined }), false)).toBe('deleted');
    expect(statusOf(file('a'), false)).toBe('modified');
  });
});

describe('totals and labels', () => {
  it('sums every file (skipped files carry 0) and zero-pads like the TUI', () => {
    const files = [file('a', { add: 10, del: 2 }), file('b', { add: 2, del: 2 }), file('big', { add: 0, del: 0, skipped: 'too_large' })];
    expect(totalsOf([files[0]!])).toEqual({ count: 1, add: 10, del: 2 });
    expect(totalsOf(files)).toEqual({ count: 3, add: 12, del: 4 });
    expect(statLabel(12, 4)).toBe('+12 −04');
    expect(statLabel(0, 0)).toBe('+00 −00');
    expect(rowStatLabel(files[1]!)).toBe('+2 −2');
    expect(fileCountLabel(1)).toBe('1 file');
    expect(fileCountLabel(3)).toBe('3 files');
  });
});

describe('undo-ability', () => {
  it('a skipped or already undone path never counts; an empty summary never does', () => {
    expect(canUndoSummary(undefined)).toBe(false);
    expect(canUndoSummary(summary([]))).toBe(false);
    expect(canUndoSummary(summary([file('a', { skipped: 'unreadable' })]))).toBe(false);
    expect(canUndoSummary(summary([file('a')], { undone: true, undonePaths: ['a'] }))).toBe(false);
    expect(canUndoSummary(summary([file('a', { skipped: 'unreadable' }), file('b')]))).toBe(true);
    expect(isUndone(summary([file('a')], { undonePaths: ['a'] }), 'a')).toBe(true);
  });
});

describe('fileSummaryCapability (A7 "Also undo file changes")', () => {
  /** A session whose `_oino/service_call` answers `undoFrom` from `reply`. */
  const fakeSession = (reply: unknown) => {
    const cmds: Array<Record<string, unknown>> = [];
    const s = {
      key: 'h1\u0000s1',
      cmd: (cmd: Record<string, unknown>) => {
        cmds.push(cmd);
        return Promise.resolve(reply);
      },
    } as unknown as SessionHandle;
    return { s, cmds };
  };

  it('canUndo reads the summary the footer cached', () => {
    const { s } = fakeSession({});
    setTurnSummary(s.key, 'u1', summary([file('a')]));
    setTurnSummary(s.key, 'u2', summary([file('a')], { undonePaths: ['a'] }));
    expect(canUndoFiles(s.key, 'u1')).toBe(true);
    expect(canUndoFiles(s.key, 'u2')).toBe(false);
    expect(canUndoFiles(s.key, 'u3')).toBe(false); // never fetched ⇒ hidden
  });

  it('undoFrom sends `undoFrom{turnEntryId}` without force and re-reads the undone turns', async () => {
    const { s, cmds } = fakeSession({ turns: ['u1'], restored: ['a.ts'], deleted: [], conflicts: [], failed: [] });
    const cap = fileSummaryCapability(s);
    await cap.undoFrom('u1');
    expect(cmds[0]).toEqual({ type: '_oino/service_call', target: 'unipi.file-summary', name: 'undoFrom', payload: { turnEntryId: 'u1' } });
    // the refresh of the undone turn (and only that turn) follows
    expect(cmds.slice(1).every((c) => (c as { name?: string }).name === 'turn')).toBe(true);
    expect((cmds[1] as { payload?: { turnEntryId?: string } }).payload?.turnEntryId).toBe('u1');
  });

  it('a conflict rejects with the paths and never forces over newer work', async () => {
    const { s, cmds } = fakeSession({ turns: ['u1'], restored: [], deleted: [], conflicts: ['a.ts', 'b.ts'], failed: [] });
    const cap = fileSummaryCapability(s);
    await expect(cap.undoFrom('u1')).rejects.toBeInstanceOf(FileUndoConflict);
    await expect(cap.undoFrom('u1')).rejects.toThrow(/changed since that turn/);
    // no retry with force, and no summary refetch after a failure
    expect(cmds.every((c) => (c as { name?: string }).name === 'undoFrom')).toBe(true);
    expect(cmds.every((c) => (c as { payload?: { force?: boolean } }).payload?.force === undefined)).toBe(true);
  });
});
