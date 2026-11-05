/**
 * Turn file summary footer (WP-A27, app spec §3.13 / D-1): the app's native
 * component for the reserved `unipi.file-summary/turn-files` row inside the
 * turn row (ZCode `ConversationFileSummaryPanel.tsx`). Collapsed it is one
 * line — `[▸] 3 files +12 −04` — and expanding lists the turn's paths with
 * their counts, a status (`new` / `modified` / `deleted` / `skipped` /
 * `undone`) and per-file actions:
 *
 *   click a row    → the lazy inline diff (`diff` service, `parseUnifiedDiff`)
 *   "View diff"    → the diff tab (A10), addressed by turn + path
 *   "Undo"         → `undo{turnEntryId, paths:[path]}`
 *   "Undo all"     → `undo{turnEntryId}` behind a confirm
 *
 * A conflict (`undo` → `conflicts`) asks "Changed since this turn — undo
 * anyway?" and only then re-sends with `force`.
 *
 * Requests: the component mounts with its turn's row (the virtualizer mounts
 * the visible window ± its overscan), so a settled turn is asked **once** plus
 * once per `invalidateTurnFooter` (§6.7 → `footerRev`); a turn the extension
 * reports as `running` is re-asked at most every {@link TURN_FILES_REFRESH_MS}.
 * The footer owns no height state: expanding it resizes this row only, and the
 * timeline's per-row ResizeObserver re-measures that one turn's height-cache
 * entry (`Timeline.tsx` `measureElement`).
 */
import { ChevronRight, ExternalLink, Undo2 } from 'lucide-solid';
import { createEffect, createMemo, createSignal, For, onCleanup, Show } from 'solid-js';
import { expansionOpen, setExpansion } from '../../../lib/timeline/expansion';
import {
  TURN_FILES_REFRESH_MS,
  canUndoSummary,
  fetchTurnFileDiff,
  fileCountLabel,
  isUndone,
  refreshTurnSummary,
  rowStatLabel,
  statusOf,
  totalsOf,
  turnSummary,
  undoFiles,
  type TurnFileDiff,
  type TurnFileRecord,
} from '../../../lib/fileSummary';
import { parseUnifiedDiff, renderDiffHtml } from '../../../lib/markdown';
import { sessions } from '../../../lib/session/store';
import { sessionCwd } from '../../../lib/session/selectors';
import { showActionNotice } from '../../../lib/timeline/actions';
import { openDiffTab } from '../../panes/tabs/sessionTabs';
import { footerRev } from '../../../ui/slots/effects';
import type { ReservedBodyProps } from '../../../ui/slots/reserved';

/** One lazily fetched inline diff (`diff` service) of a row. */
type InlineDiff = { state: 'loading' } | { state: 'ready'; reply: TurnFileDiff } | { state: 'failed'; reason: string };

const leafOf = (path: string): string => path.split('/').filter(Boolean).pop() ?? path;
const dirOf = (path: string): string => path.slice(0, Math.max(0, path.length - leafOf(path).length));
const describe = (error: unknown): string =>
  (error as { message?: unknown } | null)?.message !== undefined ? String((error as { message: unknown }).message) : String(error);

/** Why an inline diff has no rows. */
function diffNote(reply: TurnFileDiff): string {
  switch (reply.reason) {
    case 'no_snapshot':
      return 'No snapshot of this file — the turn created it.';
    case 'not_found':
      return 'This file is no longer part of the turn.';
    case 'unresolvable':
      return "Couldn't find this file in the project.";
    case 'unreadable':
      return "Couldn't read this file.";
    default:
      return reply.reason ? `No diff: ${reply.reason}.` : 'No changes to show.';
  }
}

/** The inline diff block: loading, the rendered unified diff, or why there is none. */
function InlineDiffBody(props: { state: InlineDiff | undefined }) {
  const reply = () => {
    const s = props.state;
    return s?.state === 'ready' ? s.reply : undefined;
  };
  const html = createMemo(() => {
    const diff = reply()?.diff;
    return diff ? renderDiffHtml(parseUnifiedDiff(diff)) : '';
  });
  const note = () => {
    const s = props.state;
    if (!s || s.state === 'loading') return 'Reading the diff…';
    if (s.state === 'failed') return `Couldn't read the diff: ${s.reason}`;
    return reply()?.diff ? undefined : diffNote(s.reply);
  };
  return (
    <Show when={!note()} fallback={<p class="tl-files-note">{note()}</p>}>
      <div class="tool-diff diffview-wrap" innerHTML={html()} />
      <Show when={reply()?.truncated}>
        <p class="tl-files-note">The diff is truncated.</p>
      </Show>
    </Show>
  );
}

/** `src/math.ts` → `math.ts` + a muted `src/` (the leaf is what the eye needs). */
function PathLabel(props: { path: string }) {
  return (
    <span class="tl-files-path" title={props.path}>
      <span class="tl-files-leaf">{leafOf(props.path)}</span>
      <Show when={dirOf(props.path)}>
        <span class="tl-files-dir">{dirOf(props.path)}</span>
      </Show>
    </span>
  );
}

export function TurnFilesFooter(props: ReservedBodyProps) {
  const turnId = () => props.target.turnEntryId ?? '';
  const session = () => (props.target.sessionId ? sessions().peek(props.target.hostId, props.target.sessionId) : undefined);
  const summary = () => {
    const s = session();
    return s ? turnSummary(s.key, turnId()) : undefined;
  };
  const files = (): TurnFileRecord[] => summary()?.files ?? [];
  const totals = createMemo(() => totalsOf(files()));
  const canUndo = createMemo(() => canUndoSummary(summary()));

  // By turn (expansion store), not by component: virtualizer slots are reused across turns.
  const fk = () => `files:${turnId()}`;
  const expanded = () => expansionOpen(props.target.sessionId ?? '', fk(), false);
  const setExpanded = (v: boolean | ((open: boolean) => boolean)) =>
    setExpansion(props.target.sessionId ?? '', fk(), typeof v === 'function' ? v(expanded()) : v);
  const [openPath, setOpenPath] = createSignal<string | undefined>();
  const [diffs, setDiffs] = createSignal<Record<string, InlineDiff | undefined>>({});
  /** The undo-all confirm is open. */
  const [confirming, setConfirming] = createSignal(false);
  /** Conflicts from the last undo: the "undo anyway?" prompt and what it asked for. */
  const [conflicts, setConflicts] = createSignal<{ paths: string[]; requested?: string[] } | undefined>();
  const [busy, setBusy] = createSignal(false);

  // ── the summary: on mount, again on §6.7 invalidate ─────────────────
  createEffect(() => {
    const s = session();
    const id = turnId();
    footerRev(props.entry.id);
    if (!s || !id) return;
    void refreshTurnSummary(s, id).catch((error: unknown) => showActionNotice(`Couldn't read the file summary: ${describe(error)}`));
  });

  // ── a running turn: ≤ every 500 ms (§3.13) ──────────────────────────
  const running = createMemo(() => summary()?.running === true);
  createEffect(() => {
    if (!running()) return;
    const timer = setInterval(() => {
      const s = session();
      const id = turnId();
      if (s && id) void refreshTurnSummary(s, id).catch(() => undefined);
    }, TURN_FILES_REFRESH_MS);
    onCleanup(() => clearInterval(timer));
  });

  // ── inline lazy diff ───────────────────────────────────────────────
  async function toggleInline(file: TurnFileRecord): Promise<void> {
    if (file.skipped) return; // no snapshot by design: nothing to diff
    const path = file.path;
    if (openPath() === path) {
      setOpenPath(undefined);
      return;
    }
    setOpenPath(path);
    if (diffs()[path]?.state === 'ready') return; // fetched once, kept per turn
    const s = session();
    if (!s) return;
    setDiffs((cur) => ({ ...cur, [path]: { state: 'loading' } }));
    try {
      const reply = await fetchTurnFileDiff(s, turnId(), path);
      setDiffs((cur) => ({ ...cur, [path]: { state: 'ready', reply } }));
    } catch (error) {
      setDiffs((cur) => ({ ...cur, [path]: { state: 'failed', reason: describe(error) } }));
    }
  }

  /** "View diff" (A10): the diff tab, addressed by turn + path so it re-resolves. */
  function viewInDiffTab(file: TurnFileRecord): void {
    const s = session();
    if (!s || file.skipped) return;
    openDiffTab(
      { hostId: props.target.hostId, sessionId: props.target.sessionId ?? '', cwd: sessionCwd(undefined, s.state) || undefined },
      { path: file.path, turnEntryId: turnId() },
    );
  }

  // ── undo ───────────────────────────────────────────────────────────
  const report = (reply: { restored: string[]; deleted: string[]; failed: string[] }): void => {
    const done = reply.restored.length + reply.deleted.length;
    if (reply.failed.length) showActionNotice(`Couldn't undo ${reply.failed.length} file${reply.failed.length === 1 ? '' : 's'}.`);
    else showActionNotice(`${done} file${done === 1 ? '' : 's'} undone.`);
  };

  async function runUndo(opts: { paths?: string[]; force?: boolean }, requested?: string[]): Promise<void> {
    const s = session();
    if (!s || busy()) return;
    setBusy(true);
    try {
      const reply = await undoFiles(s, turnId(), opts);
      if (reply.conflicts.length && !opts.force) {
        setConflicts({ paths: reply.conflicts, requested });
        return;
      }
      setConflicts(undefined);
      setConfirming(false);
      report(reply);
      await refreshTurnSummary(s, turnId());
    } catch (error) {
      showActionNotice(`Couldn't undo: ${describe(error)}`);
    } finally {
      setBusy(false);
    }
  }

  const undoOne = (file: TurnFileRecord) => void runUndo({ paths: [file.path] }, [file.path]);
  const undoAll = () => void runUndo({});
  const undoAnyway = () => {
    const pending = conflicts();
    if (pending) void runUndo({ paths: pending.requested, force: true }, pending.requested);
  };

  return (
    <Show when={files().length > 0}>
      <div class="tl-files" data-testid="turn-files" data-expanded={expanded() ? '' : undefined} data-turn={turnId()}>
      <button
        type="button"
        class="tl-files-toggle"
        aria-expanded={expanded()}
        onClick={() => setExpanded((open) => !open)}
        data-testid="turn-files-toggle"
      >
        <ChevronRight size={14} class="tl-files-chev" />
        <span class="tl-files-count">{fileCountLabel(totals().count)}</span>
        <span class="tl-files-stat" data-testid="turn-files-stat">
          <span class="add">+{totals().add}</span> <span class="del">−{totals().del}</span>
        </span>
        <Show when={summary()?.undone}>
          <span class="tl-files-undone">· undone</span>
        </Show>
      </button>

      <Show when={expanded()}>
        <div class="tl-files-list">
          <For each={files()}>
            {(file) => {
              const status = () => statusOf(file, isUndone(summary(), file.path));
              const open = () => openPath() === file.path;
              return (
                <>
                  <div class="tl-files-row" data-status={status()} data-path={file.path}>
                    <button
                      type="button"
                      class="tl-files-file"
                      aria-expanded={open()}
                      disabled={!!file.skipped}
                      onClick={() => void toggleInline(file)}
                      data-testid={`turn-file-${file.path}`}
                    >
                      <ChevronRight size={12} class="tl-files-rowchev" />
                      <PathLabel path={file.path} />
                      <span class="tl-files-rowstat">{file.skipped ? '—' : rowStatLabel(file)}</span>
                      <span class="tl-files-status">{status()}</span>
                    </button>
                    <div class="tl-files-rowactions">
                      <Show when={!file.skipped}>
                        <button
                          type="button"
                          class="ghost-icon-button"
                          aria-label={`View diff of ${file.path}`}
                          title="View diff"
                          onClick={() => viewInDiffTab(file)}
                          data-testid={`turn-file-diff-${file.path}`}
                        >
                          <ExternalLink size={14} />
                        </button>
                      </Show>
                      <Show when={!file.skipped && !isUndone(summary(), file.path)}>
                        <button
                          type="button"
                          class="ghost-icon-button"
                          aria-label={`Undo changes to ${file.path}`}
                          title="Undo"
                          onClick={() => undoOne(file)}
                          data-testid={`turn-file-undo-${file.path}`}
                        >
                          <Undo2 size={14} />
                        </button>
                      </Show>
                    </div>
                  </div>
                  <Show when={open()}>
                    <div class="tl-files-diff" data-testid={`turn-file-body-${file.path}`}>
                      <InlineDiffBody state={diffs()[file.path]} />
                    </div>
                  </Show>
                </>
              );
            }}
          </For>

          <Show when={canUndo()}>
            <div class="tl-files-actions">
              <Show
                when={confirming()}
                fallback={
                  <button type="button" class="tl-files-undo-all" onClick={() => setConfirming(true)} data-testid="turn-files-undo-all">
                    Undo all
                  </button>
                }
              >
                <span class="tl-files-question">Undo the {fileCountLabel(totals().count)} this turn changed?</span>
                <button type="button" class="btn btn-secondary" onClick={() => setConfirming(false)} data-testid="turn-files-undo-cancel">
                  Cancel
                </button>
                <button type="button" class="btn btn-danger" disabled={busy()} onClick={undoAll} data-testid="turn-files-undo-confirm">
                  Undo all
                </button>
              </Show>
            </div>
          </Show>

          <Show when={conflicts()}>
            {(pending) => (
              <div class="tl-files-conflict" role="alert" data-testid="turn-files-conflict">
                <span class="tl-files-question">Changed since this turn — undo anyway?</span>
                <span class="tl-files-paths">{pending().paths.join(', ')}</span>
                <button type="button" class="btn btn-secondary" onClick={() => setConflicts(undefined)} data-testid="turn-files-conflict-cancel">
                  Cancel
                </button>
                <button type="button" class="btn btn-danger" disabled={busy()} onClick={undoAnyway} data-testid="turn-files-conflict-confirm">
                  Undo anyway
                </button>
              </div>
            )}
          </Show>
        </div>
      </Show>
      </div>
    </Show>
  );
}
