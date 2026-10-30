/**
 * Summaries as a bottom sheet (UNI-131) — the web's SummarizeDialog
 * (Done lane → review/edit → Save & archive) and BoardSummarizeDialog
 * (whole board, optional prompt, nothing archived), one sheet for both.
 *
 * Phone differences: generating shows a progress state with the elapsed
 * time and the PC doing the work; the sheet can be closed (Back, swipe,
 * Hide) while the agent works — the job keeps running in
 * lib/board/summarize.ts and a toast with Open brings the result back;
 * Stop waiting drops the answer. The web blocks its dialog instead.
 */
import { createEffect, createSignal, on, onCleanup, Show } from 'solid-js';
import { Copy, Pencil, RefreshCw, Settings2, Sparkles, TriangleAlert, Eye, MessageSquarePlus } from 'lucide-solid';
import { boardStore } from '../../lib/board/store';
import { boardUi } from '../../lib/board/ui';
import { summaries } from '../../lib/board/summarize';
import { copyToClipboard } from '../../lib/board/taskActions';
import { BoardSheet } from './BoardSheet';
import { StatusGlyph } from './Glyphs';
import { Markdown } from './Markdown';
import crab from '../../assets/unicrab-pixel.png';

function clock(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;
}

export function SummarizeSheet(props: { pcName?: string }) {
  const s = summaries();
  const job = s.job;
  const store = boardStore();
  const ui = boardUi();
  const [editing, setEditing] = createSignal(false);
  const [noteOpen, setNoteOpen] = createSignal(false);
  const [tick, setTick] = createSignal(Date.now());
  const scope = () => s.open() ?? job.scope;
  const board = () => scope() === 'board';
  const doneCount = () => store.state.tasks.filter((task) => task.status === 'done').length;
  const pc = () => props.pcName ?? 'your PC';

  createEffect(
    on(s.open, (open) => {
      if (!open) return;
      setEditing(false);
      setNoteOpen(false);
    }),
  );
  createEffect(() => {
    if (!s.busy()) return;
    const timer = setInterval(() => setTick(Date.now()), 1000);
    onCleanup(() => clearInterval(timer));
  });
  const elapsedText = () => (job.startedAt ? clock(tick() - job.startedAt) : '0:00');

  const close = () => {
    if (job.saving) return;
    if (s.busy()) ui.toast('Still summarizing on the PC — you get a toast when it is ready', 'info');
    s.hide();
  };

  const title = () => (board() ? 'Summarize board' : 'Summarize & archive');
  const footer = () => {
    if (job.step === 'running') {
      return (
        <div class="flex gap-3">
          <button type="button" class="pair-secondary flex-1" onClick={() => s.stop()}>
            Stop waiting
          </button>
          <button type="button" class="pair-primary flex-1" onClick={close}>
            Hide
          </button>
        </div>
      );
    }
    if (job.step === 'result') {
      return board() ? (
        <button type="button" class="pair-primary" disabled={s.busy()} onClick={() => s.discard()}>
          Done
        </button>
      ) : (
        <button type="button" class="pair-primary" disabled={s.busy() || job.saving || !job.summary.trim()} onClick={() => void s.saveAndArchive()}>
          {job.saving ? 'Archiving…' : `Save & archive ${job.taskIds.length} task${job.taskIds.length === 1 ? '' : 's'}`}
        </button>
      );
    }
    return (
      <button type="button" class="pair-primary gap-2" disabled={!board() && doneCount() === 0} onClick={() => void s.generate()}>
        <Sparkles size={18} /> Generate summary
      </button>
    );
  };

  return (
    <BoardSheet
      open={s.open() !== null}
      onClose={close}
      label={title()}
      testId="summarize-sheet"
      class="board-sheet-tall summarize-sheet"
      guarded={job.step === 'result' && editing()}
      title={
        <span class="flex items-center gap-2 min-w-0">
          <Show when={!board()}>
            <span class="crumb-pill">
              <StatusGlyph status="done" size={13} /> Done
            </span>
          </Show>
          <span class="summarize-title">{title()}</span>
        </span>
      }
      footer={footer()}
    >
      <Show when={job.step === 'config'}>
        <p class="sheet-hint">
          <Show
            when={board()}
            fallback={
              <>
                {doneCount()} done task{doneCount() === 1 ? '' : 's'} will be summarized by the configured agent on {pc()}, then archived. You review the
                summary first.
              </>
            }
          >
            The agent on {pc()} summarizes every lane of the board. An optional prompt steers it. Nothing is archived.
          </Show>
        </p>
        <label class="field-label" for="summary-instruction">
          {board() ? 'Prompt (optional)' : 'Instruction'}
        </label>
        <textarea
          id="summary-instruction"
          class="sheet-textarea tall"
          aria-label={board() ? 'Summary prompt' : 'Summary instruction'}
          placeholder={board() ? 'e.g. Summarize only todo, in priority order\ne.g. Explain the relevance of the current todo tasks' : 'Loading the instruction…'}
          value={job.instruction}
          onInput={(event) => s.setJob('instruction', event.currentTarget.value)}
        />
        <Show when={job.needsAgent}>
          <div class="board-banner warn mt-3" role="alert">
            <TriangleAlert size={18} class="shrink-0 text-amber" />
            <div class="min-w-0 flex-1">
              <p class="font-medium">No agent configured</p>
              <p class="text-text-muted">Set the command that writes the summary first.</p>
              <button
                type="button"
                class="banner-button mt-2 inline-flex items-center gap-2"
                onClick={() => {
                  s.hide();
                  ui.setSettingsOpen(true);
                }}
              >
                <Settings2 size={16} /> Configure an agent first
              </button>
            </div>
          </div>
        </Show>
        <Show when={job.error}>
          <p class="sheet-error" role="alert">
            {job.error}
          </p>
        </Show>
      </Show>

      <Show when={job.step === 'running'}>
        <div class="summary-progress" role="status" aria-live="polite">
          <img src={crab} alt="" class="summary-crab" />
          <p class="summary-progress-title">Summarizing on {pc()}…</p>
          <p class="summary-progress-time tabular-nums">{elapsedText()}</p>
          <div class="summary-bar" aria-hidden="true">
            <span />
          </div>
          <p class="sheet-hint text-center">
            The agent writes the summary on the PC; this can take a few minutes. You can hide this sheet — a toast tells you when it's ready.
          </p>
        </div>
      </Show>

      <Show when={job.step === 'result'}>
        <div class="summary-tools" role="toolbar" aria-label="Summary tools">
          <Show when={!board()}>
            <button type="button" class="tool-chip" aria-pressed={editing()} onClick={() => setEditing(!editing())}>
              <Show when={editing()} fallback={<Pencil size={15} />}>
                <Eye size={15} />
              </Show>
              {editing() ? 'Preview' : 'Edit'}
            </button>
          </Show>
          <button type="button" class="tool-chip" onClick={() => void copyToClipboard(job.summary, 'the summary')}>
            <Copy size={15} /> Copy
          </button>
          <button type="button" class="tool-chip" aria-pressed={noteOpen()} aria-label="Resummarize with note" onClick={() => setNoteOpen(!noteOpen())}>
            <MessageSquarePlus size={15} /> With note
          </button>
          <Show when={!board()}>
            <button type="button" class="tool-chip" disabled={s.busy()} onClick={() => void s.generate()}>
              <RefreshCw size={15} /> Regenerate
            </button>
          </Show>
        </div>
        <Show when={noteOpen()}>
          <div class="resummarize-note">
            <textarea
              class="sheet-textarea"
              aria-label="Note for the resummary"
              placeholder={board() ? 'What should the summary do differently? (e.g. Summarize only todo, in priority order)' : 'What should the summary do differently? (e.g. focus on blockers)'}
              value={job.note}
              onInput={(event) => s.setJob('note', event.currentTarget.value)}
            />
            <button type="button" class="banner-button primary inline-flex items-center gap-2 mt-2" disabled={s.busy()} onClick={() => void s.resummarize()}>
              <Show when={s.busy()} fallback={<Sparkles size={15} />}>
                <span class="spinner" />
              </Show>
              {s.busy() ? `Resummarizing… ${elapsedText()}` : 'Resummarize'}
            </button>
          </div>
        </Show>
        <Show when={job.error}>
          <p class="sheet-error" role="alert">
            {job.error}
          </p>
        </Show>
        <Show
          when={editing()}
          fallback={
            <div class="summary-preview">
              <Markdown text={job.summary} label="Summary" />
            </div>
          }
        >
          <textarea class="sheet-textarea summary-edit" aria-label="Summary (markdown)" value={job.summary} onInput={(event) => s.setJob('summary', event.currentTarget.value)} />
        </Show>
      </Show>
    </BoardSheet>
  );
}
