/**
 * [graph-view] "Where can I see the graph?" — one entry point for every
 * control that opens a chat's long-horizon graph: the header mode chip while
 * graph / swarm mode runs, the chat ⋯ "View graph" row and the phone ⋯ sheet.
 *
 * Desktop: the status pane's Long-horizon section (the side-pane `git` tab),
 * scrolled to it. Phone: a full-screen sheet holding the same section (mode,
 * counts, stop / resume / history and the DAG with one row per node).
 */
import { createSignal, Show } from 'solid-js';
import { desktopShell } from '../../lib/layout';
import { toasts } from '../../lib/toasts';
import { graphSettled, type LhSummary } from '../../lib/status/lh';
import { Sheet } from '../phone/Sheet';
import { openStatusSection } from './StatusPane';
import { LhSection } from './LhSection';

export type GraphTarget = { hostId: string; sessionId: string; path?: string };

const [sheet, setSheet] = createSignal<GraphTarget | undefined>(undefined);
/** The phone sheet's chat (tests). */
export const graphSheetTarget = sheet;

/** A graph / swarm run with at least one node exists for this summary. Pure. */
export function hasGraph(summary: LhSummary | null | undefined): boolean {
  return !!summary?.graph && graphSettled(summary.graph.counts).total > 0;
}

/** Graph or swarm mode is on and has a graph: the mode chip opens the graph. Pure. */
export function chipOpensGraph(summary: LhSummary | null | undefined): boolean {
  return !!summary && (summary.mode === 'graph' || summary.mode === 'swarm') && hasGraph(summary);
}

/** Opens the chat's graph: the status pane's LH section (desktop) or the full-screen sheet (phone). */
export function openGraphView(target: GraphTarget): void {
  if (desktopShell()) {
    openStatusSection({ hostId: target.hostId, sessionId: target.sessionId, path: target.path || '~' }, 'lh');
    return;
  }
  setSheet({ hostId: target.hostId, sessionId: target.sessionId });
}

export function closeGraphView(): void {
  setSheet(undefined);
}

/** The phone's full-screen graph sheet. Mounted once by the shell. */
export function GraphSheetLayer() {
  return (
    <Show when={sheet()} keyed>
      {(t) => (
        <Sheet open onClose={closeGraphView} label="Graph" title="Graph" detents={['full']} initialDetent="full" testId="graph-sheet" class="graph-sheet">
          <div class="graph-sheet-body" data-testid="graph-sheet-body">
            <LhSection hostId={t.hostId} sessionId={t.sessionId} onToast={(text) => toasts().push(text)} />
          </div>
        </Sheet>
      )}
    </Show>
  );
}
