/**
 * Native background-task tool body (D-06): clean, responsive card for
 * `unipi.background-tasks` (`bg_run`, `bg_logs`, `bg_kill`, `bg_status`)
 * without the TUI horizontal-column layout (`Min(0)` + bordered `Block`
 * divider) or raw internal temp paths (`.../tmp/tasks/.../output.log`) that
 * caused horizontal scrollbars and text overflow at narrow widths.
 */
import { createSignal, For, Show } from 'solid-js';
import { formatDuration } from '../../../../lib/chat/model';
import type { ToolRun } from '../../../../lib/session/turns';
import type { TimelineCtx } from '../../context';
import { useFullOutput } from './fullOutput';

const BG_TOOLS = new Set(['bg_run', 'bg_logs', 'bg_kill', 'bg_status']);
const OUT_LOGS = 6000;

type TaskStatus = 'running' | 'completed' | 'failed' | 'killed';

type BgTask = {
  id?: string;
  name?: string;
  command?: string;
  status?: TaskStatus;
  startTime?: number;
  endTime?: number;
  exitCode?: number | null;
  error?: string;
  notifyOnCompletion?: boolean;
  triggerOnCompletion?: boolean;
  outputTail?: string[];
};

const isObj = (v: unknown): v is Record<string, unknown> => !!v && typeof v === 'object' && !Array.isArray(v);
const str = (v: unknown): string | undefined => (typeof v === 'string' && v.trim() ? v.trim() : undefined);

/** Whether `tool` is a real background-task run (keeps mock `_oino/render` unit tests without `details` working). */
export function isNativeBgTool(tool: ToolRun): boolean {
  const name = tool.name.trim().toLowerCase();
  if (!BG_TOOLS.has(name)) return false;
  if (tool.state === 'running' || tool.state === 'preparing' || tool.state === 'error') return true;
  if (isObj(tool.details) && (isObj(tool.details.task) || Array.isArray(tool.details.tasks))) return true;
  const out = tool.output ?? '';
  return (
    /^(Started|Killed) background task\b/.test(out) ||
    /^\[Showing (tail|head)\b/.test(out.trimStart()) ||
    out.trim() === '(no output yet)' ||
    out.trim() === 'No background tasks.'
  );
}

function normalizeStatus(raw: unknown, fallback: TaskStatus = 'running'): TaskStatus {
  if (raw === 'completed' || raw === 'done') return 'completed';
  if (raw === 'failed' || raw === 'error') return 'failed';
  if (raw === 'killed' || raw === 'stopped') return 'killed';
  if (raw === 'running') return 'running';
  return fallback;
}

const BADGE_LABEL: Record<TaskStatus, string> = {
  running: 'Background',
  completed: 'Done',
  failed: 'Failed',
  killed: 'Stopped',
};

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1).replace(/\.0$/, '')} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** Strips raw internal host temp-log paths (`Output: /.../output.log`, `Full output: /.../output.log`) from model-facing text. */
export function stripBgInternalPaths(text: string): string {
  return text
    .replace(/\.\s*Full output:\s*\S+/g, '')
    .replace(/^(?:Full output|Output):\s*\S+\s*$/gm, '')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
}

function parseTask(raw: unknown, args: Record<string, unknown>, toolState: ToolRun['state']): BgTask {
  const obj = isObj(raw) ? raw : {};
  const fallbackStatus: TaskStatus = toolState === 'error' ? 'failed' : toolState === 'stopped' ? 'killed' : 'running';
  const tail = Array.isArray(obj.outputTail) ? obj.outputTail.filter((l): l is string => typeof l === 'string' && l.length > 0) : undefined;
  return {
    id: str(obj.id) ?? str(args.taskId),
    name: str(obj.name) ?? str(args.name) ?? str(args.description),
    command: str(obj.command) ?? str(args.command),
    status: normalizeStatus(obj.status, fallbackStatus),
    startTime: typeof obj.startTime === 'number' ? obj.startTime : undefined,
    endTime: typeof obj.endTime === 'number' ? obj.endTime : undefined,
    exitCode: typeof obj.exitCode === 'number' || obj.exitCode === null ? obj.exitCode : undefined,
    error: str(obj.error),
    notifyOnCompletion: typeof obj.notifyOnCompletion === 'boolean' ? obj.notifyOnCompletion : typeof args.notifyOnCompletion === 'boolean' ? args.notifyOnCompletion : true,
    triggerOnCompletion: typeof obj.triggerOnCompletion === 'boolean' ? obj.triggerOnCompletion : typeof args.triggerOnCompletion === 'boolean' ? args.triggerOnCompletion : true,
    outputTail: tail?.length ? tail : undefined,
  };
}

function taskMeta(task: BgTask, toolName: string): string {
  if (toolName === 'bg_kill') return task.id ? `stopped · ${task.id}` : 'stopped';
  const parts: string[] = [];
  if (task.status === 'running') {
    parts.push('started');
    parts.push(task.triggerOnCompletion !== false ? 'wakes agent' : task.notifyOnCompletion !== false ? 'notifies' : 'silent');
  } else {
    if (task.status === 'killed') parts.push('stopped');
    else if (typeof task.exitCode === 'number') parts.push(`exit ${task.exitCode}`);
    else if (task.status === 'failed') parts.push('failed');
    if (task.startTime !== undefined && task.endTime !== undefined && task.endTime >= task.startTime) {
      parts.push(formatDuration(Math.max(0, task.endTime - task.startTime)));
    }
    if (task.triggerOnCompletion && task.status !== 'killed') parts.push('wakes agent');
  }
  if (task.id) parts.push(task.id);
  return parts.join(' · ');
}

export function BgToolBody(props: { tool: ToolRun; ctx: TimelineCtx }) {
  const t = () => props.tool;
  const lower = () => t().name.trim().toLowerCase();
  const details = (): Record<string, unknown> => {
    const d = t().details;
    return isObj(d) ? d : {};
  };

  return (
    <Show
      when={lower() !== 'bg_status'}
      fallback={<BgStatusBody tool={t()} details={details()} />}
    >
      <Show
        when={lower() !== 'bg_logs'}
        fallback={<BgLogsBody tool={t()} details={details()} ctx={props.ctx} />}
      >
        <BgLaunchOrKillBody tool={t()} details={details()} />
      </Show>
    </Show>
  );
}

function BgLaunchOrKillBody(props: { tool: ToolRun; details: Record<string, unknown> }) {
  const lower = () => props.tool.name.trim().toLowerCase();
  const task = () => parseTask(props.details.task, props.tool.args, lower() === 'bg_kill' && props.tool.state !== 'error' ? 'stopped' : props.tool.state);
  const status = () => (props.tool.state === 'error' ? 'failed' : lower() === 'bg_kill' ? 'killed' : (task().status ?? 'running'));
  const title = () => task().name ?? task().command ?? task().id ?? 'Background task';
  const meta = () => taskMeta({ ...task(), status: status() }, lower());
  const errText = () => {
    if (task().error) return task().error;
    if (props.tool.state === 'error' && props.tool.output) return stripBgInternalPaths(props.tool.output);
    return undefined;
  };
  const tailText = () => (task().outputTail?.length ? task().outputTail!.join('\n') : undefined);
  const hasOut = () => Boolean(errText() || tailText());

  return (
    <div class="tool-bg" data-status={status()} data-testid="tool-bg">
      <div class="tool-bg-head" data-border={task().command || hasOut() ? '' : undefined}>
        <span class="tool-bg-badge" data-status={status()}>
          {BADGE_LABEL[status()]}
        </span>
        <span class="tool-bg-title">{title()}</span>
        <Show when={meta()}>
          <span class="tool-bg-meta">{meta()}</span>
        </Show>
      </div>
      <Show when={task().command}>
        <div class="tool-term-cmd" data-last={!hasOut() ? '' : undefined}>
          <span class="tool-term-prompt">$</span>
          <span>{task().command}</span>
        </div>
      </Show>
      <Show when={errText()}>
        <pre class="tool-term-out" data-error="">
          {errText()}
        </pre>
      </Show>
      <Show when={!errText() && tailText()}>
        <pre class="tool-term-out">{tailText()}</pre>
      </Show>
    </div>
  );
}

function BgLogsBody(props: { tool: ToolRun; details: Record<string, unknown>; ctx: TimelineCtx }) {
  const { full, loading, snapshotTruncated, load } = useFullOutput(props.ctx, () => props.tool);
  const [all, setAll] = createSignal(false);
  const task = () => parseTask(props.details.task, props.tool.args, props.tool.state);
  const status = () => (props.tool.state === 'error' ? 'failed' : (task().status ?? 'running'));
  const title = () => task().name ?? task().id ?? 'Background logs';
  const meta = () => {
    const parts: string[] = [];
    const isTail = props.details.tail !== false && props.tool.args.tail !== false;
    parts.push(isTail ? 'tail' : 'head');
    if (typeof props.details.bytesRead === 'number') parts.push(formatBytes(props.details.bytesRead));
    if (props.details.truncated === true) parts.push('truncated');
    if (task().id && task().id !== title()) parts.push(task().id!);
    return parts.join(' · ');
  };
  const rawText = () => stripBgInternalPaths(full() ?? props.tool.output ?? props.tool.live ?? '');
  const needsFetch = () => full() === undefined && !!props.tool.resultId && (snapshotTruncated() || rawText().length > OUT_LOGS);
  const trimmed = () => (!all() && rawText().length > OUT_LOGS) || needsFetch();
  const shown = () => (all() || rawText().length <= OUT_LOGS ? rawText() : `…\n${rawText().slice(rawText().length - OUT_LOGS)}`);

  return (
    <div class="tool-bg" data-status={status()} data-testid="tool-bg">
      <div class="tool-bg-head" data-border={shown() ? '' : undefined}>
        <span class="tool-bg-badge" data-status={status()}>
          {BADGE_LABEL[status()]}
        </span>
        <span class="tool-bg-title">{title()}</span>
        <Show when={meta()}>
          <span class="tool-bg-meta">{meta()}</span>
        </Show>
      </div>
      <Show when={shown()}>
        <pre class="tool-term-out" data-all={all() ? '' : undefined} data-error={props.tool.state === 'error' ? '' : undefined}>
          {shown()}
        </pre>
      </Show>
      <Show when={trimmed() && !loading()}>
        <button
          type="button"
          class="tool-more"
          onClick={() => {
            load();
            setAll(true);
          }}
          data-testid="tool-full-output"
        >
          Show full output
        </button>
      </Show>
      <Show when={all() && rawText().length > OUT_LOGS && !loading()}>
        <button type="button" class="tool-more" onClick={() => setAll(false)} data-testid="tool-fold-output">
          Fold output
        </button>
      </Show>
    </div>
  );
}

function BgStatusBody(props: { tool: ToolRun; details: Record<string, unknown> }) {
  const tasks = () => {
    const raw = Array.isArray(props.details.tasks) ? props.details.tasks : [];
    return raw.map((item) => parseTask(item, {}, 'ok'));
  };
  const fallbackText = () => stripBgInternalPaths(props.tool.output ?? '');

  return (
    <div class="tool-bg" data-testid="tool-bg">
      <Show
        when={tasks().length > 0}
        fallback={<pre class="tool-term-out" data-error={props.tool.state === 'error' ? '' : undefined}>{fallbackText() || 'No background tasks.'}</pre>}
      >
        <div class="tool-bg-list">
          <For each={tasks()}>
            {(item) => {
              const st = () => item.status ?? 'running';
              return (
                <div class="tool-bg-item">
                  <span class="tool-bg-badge" data-status={st()}>
                    {BADGE_LABEL[st()]}
                  </span>
                  <span class="tool-bg-title">{item.name ?? item.command ?? item.id ?? 'Task'}</span>
                  <span class="tool-bg-meta">{taskMeta(item, 'bg_status')}</span>
                </div>
              );
            }}
          </For>
        </div>
      </Show>
    </div>
  );
}
