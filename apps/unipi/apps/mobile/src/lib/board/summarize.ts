/**
 * Summaries (UNI-131), ported from the kanboard web UI's SummarizeDialog
 * (Done lane: summarize → review/edit → save & archive) and
 * BoardSummarizeDialog (whole board, optional prompt, nothing archived).
 *
 * The summary is written by an AI agent ON THE PC and can take minutes,
 * so the job lives here, not in the sheet: closing the sheet (Back, swipe,
 * "Hide") keeps it running and a toast with "Open" brings the result
 * back; "Stop waiting" drops the answer (the PC may still finish — the
 * daemon has no cancel endpoint). One job at a time.
 */
import { createRoot, createSignal } from 'solid-js';
import { createStore } from 'solid-js/store';
import { api, ApiError, describe } from './api';
import { boardStore } from './store';
import { boardUi } from './ui';
import { copyText } from './share';

export type SummarizeScope = 'done' | 'board';
export type SummarizeStep = 'config' | 'running' | 'result';

export type SummarizeJob = {
  scope: SummarizeScope;
  hostId: string;
  slug: string;
  step: SummarizeStep;
  /** Done scope: the instruction (prefilled from settings); board scope: the optional prompt. */
  instruction: string;
  summary: string;
  taskIds: string[];
  needsAgent: boolean;
  error: string | undefined;
  /** A resummarize (with the note) is running, the shown summary stays. */
  resummarizing: boolean;
  startedAt: number | undefined;
  note: string;
  saving: boolean;
};

function createSummaries() {
  const [open, setOpen] = createSignal<SummarizeScope | null>(null);
  const [job, setJob] = createStore<SummarizeJob>(blank('done', '', ''));
  let seq = 0;

  function blank(scope: SummarizeScope, hostId: string, slug: string): SummarizeJob {
    return {
      scope,
      hostId,
      slug,
      step: 'config',
      instruction: '',
      summary: '',
      taskIds: [],
      needsAgent: false,
      error: undefined,
      resummarizing: false,
      startedAt: undefined,
      note: '',
      saving: false,
    };
  }

  const busy = () => job.step === 'running' || job.resummarizing;

  /** Opens the sheet; a running/finished job of the same kind on the same board is resumed. */
  function show(scope: SummarizeScope): void {
    const at = boardStore().target();
    if (!at?.slug) return;
    const same = job.scope === scope && job.hostId === at.hostId && job.slug === at.slug;
    if (!(same && (busy() || job.step === 'result'))) {
      if (busy()) seq += 1; // a different job replaces the old one: drop its answer
      setJob(blank(scope, at.hostId, at.slug));
      if (scope === 'done') {
        const mine = seq;
        void api
          .settings(at.hostId)
          .then((settings) => {
            if (mine === seq && job.step === 'config' && !job.instruction) setJob('instruction', settings.summaryInstruction);
          })
          .catch((error) => boardUi().toast(describe(error), 'error'));
      }
    }
    setOpen(scope);
  }

  /** Closes the sheet; a running job keeps going in the background. */
  function hide(): void {
    setOpen(null);
  }

  /** Done with it: closes and forgets (not while saving). */
  function discard(): void {
    if (job.saving) return;
    if (busy()) seq += 1;
    setJob(blank(job.scope, job.hostId, job.slug));
    setOpen(null);
  }

  async function run(previous?: string, note?: string): Promise<void> {
    if (busy() || !job.slug) return;
    const mine = ++seq;
    const { hostId, slug, scope } = job;
    const again = previous !== undefined;
    setJob({ needsAgent: false, error: undefined, startedAt: Date.now() });
    if (again) setJob('resummarizing', true);
    else setJob('step', 'running');
    try {
      const result = await api.summarize(hostId, slug, {
        scope: scope === 'board' ? 'board' : undefined,
        instruction: job.instruction,
        previous,
        note,
      });
      if (mine !== seq) return;
      setJob({ summary: result.summary, taskIds: result.taskIds ?? [], step: 'result', resummarizing: false, note: '' });
      if (open() !== scope) {
        boardUi().toast(scope === 'board' ? 'Board summary is ready' : 'Done-lane summary is ready', 'success', {
          label: 'Open',
          run: () => void setOpen(scope),
        });
      }
    } catch (error) {
      if (mine !== seq) return;
      setJob({ step: again ? 'result' : 'config', resummarizing: false });
      if (error instanceof ApiError && error.needsAgent) setJob('needsAgent', true);
      else setJob('error', describe(error));
      if (open() !== scope) boardUi().toast(`Summary failed: ${describe(error)}`, 'error', { label: 'Open', run: () => void setOpen(scope) });
    }
  }

  /** Generate (or Regenerate) from the instruction / prompt. */
  const generate = () => run();
  /** Resummarize: feed the shown summary + the user's note back in. */
  const resummarize = () => run(job.summary, job.note);

  /** Stop waiting for the PC (it may still finish there; the answer is dropped). */
  function stop(): void {
    if (!busy()) return;
    seq += 1;
    setJob({ step: job.resummarizing ? 'result' : 'config', resummarizing: false, startedAt: undefined });
  }

  /** Done scope: write the summary file and archive the summarized tasks. */
  async function saveAndArchive(): Promise<boolean> {
    if (busy() || job.saving || !job.summary.trim()) return false;
    const { hostId, slug } = job;
    setJob('saving', true);
    try {
      const result = await api.archiveSummary(hostId, slug, { markdown: job.summary, taskIds: [...job.taskIds] });
      boardUi().toast(`Archived ${result.archived.length} task${result.archived.length === 1 ? '' : 's'} · summary saved`, 'success', {
        label: 'Copy path',
        run: async () => {
          const ok = await copyText(result.path);
          boardUi().toast(ok ? 'Copied the summary path' : "Couldn't copy the path", ok ? 'success' : 'error');
        },
      });
      setJob(blank(job.scope, hostId, slug));
      setOpen(null);
      const at = boardStore().target();
      if (at?.hostId === hostId && at.slug === slug) await boardStore().reload();
      return true;
    } catch (error) {
      boardUi().toast(describe(error), 'error');
      return false;
    } finally {
      setJob('saving', false);
    }
  }

  return { open, job, setJob, busy, show, hide, discard, generate, resummarize, stop, saveAndArchive };
}

export type Summaries = ReturnType<typeof createSummaries>;
let shared: Summaries | undefined;
export function summaries(): Summaries {
  if (!shared) shared = createRoot(() => createSummaries());
  return shared;
}
/** Test hook. */
export function resetSummaries(): void {
  shared = undefined;
}
