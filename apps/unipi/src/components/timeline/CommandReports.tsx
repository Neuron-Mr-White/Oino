/**
 * Command reports as timeline rows (owner5 #6): a multi-line command reply
 * (`/global-memory-list`, `/mcp-status`, `/cron list`…) that answers a
 * command run in this chat shows at the bottom of the conversation, in the
 * transcript's column — like any other reply — with Copy and ✕, instead of
 * a floating card over the timeline.
 */
import { createSignal, For, Show } from 'solid-js';
import { Check, ChevronDown, ChevronRight, Copy, TerminalSquare, X } from 'lucide-solid';
import { dismissSessionReport, looksMarkdown, reportHeadline, sessionReports, type Report } from '../../lib/reports';
import { copyText } from '../../lib/board/share';
import { Markdown } from '../board/Markdown';
import '../report.css';

export function CommandReports(props: { sessionKey: string }) {
  return (
    <For each={sessionReports(props.sessionKey)}>
      {(report) => <ReportRow report={report} onDismiss={() => dismissSessionReport(props.sessionKey, report.id)} />}
    </For>
  );
}

function ReportRow(props: { report: Report; onDismiss: () => void }) {
  const [copied, setCopied] = createSignal(false);
  /** N3: a short note is open; a long report starts folded to its headline (one click to read). */
  const [open, setOpen] = createSignal(props.report.body.length <= 200 && !props.report.body.includes('\n'));
  const copy = async () => {
    if (await copyText(props.report.body)) {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    }
  };
  return (
    <div class="tl-turn tl-report-row" data-testid="tl-command-report" role="article" aria-label={`${props.report.title} output`}>
      <div class="tl-report-card" data-kind={props.report.kind}>
        <div class="tl-report-head">
          <button type="button" class="tl-report-fold" aria-expanded={open()} aria-label={open() ? 'Collapse output' : 'Expand output'} onClick={() => setOpen(!open())} data-testid="tl-report-fold">
            <Show when={open()} fallback={<ChevronRight size={14} />}>
              <ChevronDown size={14} />
            </Show>
            <TerminalSquare size={14} class="shrink-0" />
            <span class="tl-report-title" title={props.report.title}>{props.report.title}</span>
            <Show when={!open()}>
              <span class="tl-report-peek">{reportHeadline(props.report.body)}</span>
            </Show>
          </button>
          <span class="flex-1" />
          <button type="button" class="report-copy" onClick={() => void copy()} aria-label="Copy output">
            <Show when={copied()} fallback={<Copy size={13} />}>
              <Check size={13} />
            </Show>
            <span>{copied() ? 'Copied' : 'Copy'}</span>
          </button>
          <button type="button" class="ghost-icon-button tl-report-x" aria-label="Dismiss output" onClick={() => props.onDismiss()}>
            <X size={15} />
          </button>
        </div>
        <Show when={open()}>
          <Show
            when={looksMarkdown(props.report.body)}
            fallback={<pre class="report-body tl-report-body" data-kind={props.report.kind}>{props.report.body}</pre>}
          >
            <div class="report-body report-md tl-report-body" data-kind={props.report.kind}>
              <Markdown text={props.report.body} />
            </div>
          </Show>
        </Show>
      </div>
    </div>
  );
}
