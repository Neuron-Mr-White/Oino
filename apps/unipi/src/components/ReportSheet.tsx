/**
 * The command report panel (P1-02): a multi-line `notify` (`/mcp-status`,
 * `/doctor`, `/cron list`, `/memory-search`…) shown whole — a centred card on
 * the desktop, a bottom sheet on the phone — with Copy and ✕. Monospace keeps
 * the reports' aligned columns; long lines wrap, the body scrolls. Opened by
 * `lib/reports.ts` (`routeNotify`); one at a time, the newest replaces.
 */
import { createSignal, Show } from 'solid-js';
import { Check, Copy, X } from 'lucide-solid';
import { BoardSheet } from './board/BoardSheet';
import { closeReport, openReport } from '../lib/reports';
import { copyText } from '../lib/board/share';
import './report.css';

export function ReportSheet() {
  const [copied, setCopied] = createSignal(false);
  let copiedTimer: ReturnType<typeof setTimeout> | undefined;
  const copy = async () => {
    const report = openReport();
    if (!report) return;
    if (await copyText(report.body)) {
      setCopied(true);
      clearTimeout(copiedTimer);
      copiedTimer = setTimeout(() => setCopied(false), 1500);
    }
  };
  return (
    <BoardSheet
      open={!!openReport()}
      onClose={closeReport}
      label={openReport()?.title ?? 'Report'}
      title={<span class="report-title" title={openReport()?.title}>{openReport()?.title}</span>}
      class="report-sheet"
      testId="report-sheet"
      headerAction={
        <div class="report-actions">
          <button type="button" class="report-copy" onClick={() => void copy()} aria-label="Copy report">
            <Show when={copied()} fallback={<Copy size={14} />}>
              <Check size={14} />
            </Show>
            <span>{copied() ? 'Copied' : 'Copy'}</span>
          </button>
          <button type="button" class="ghost-icon-button report-close" aria-label="Close" onClick={closeReport}>
            <X size={18} />
          </button>
        </div>
      }
    >
      <pre class="report-body" data-kind={openReport()?.kind} data-testid="report-body">
        {openReport()?.body}
      </pre>
    </BoardSheet>
  );
}
