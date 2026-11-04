/**
 * The `bgtask` side-pane tab ([owner-fix-3/running-pane]): one background task's live output in a
 * read-only xterm. Header = name · status · exit code · Stop; the command under it; the log below,
 * following the bottom while new output arrives (xterm keeps a viewport that sits at the bottom
 * there, and leaves one the user scrolled up alone).
 *
 * Data: the background-tasks extension's `bg.output{id, since}` service (new bytes from a cursor +
 * the task snapshot), polled while the task runs and once more when it settles. An older extension
 * without the service falls back to the snapshot's last output lines from the `unipi:bg:tasks` topic.
 */
import { createEffect, createMemo, createSignal, onCleanup, onMount, Show } from 'solid-js';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import { Square, SquareTerminal } from 'lucide-solid';
import { sessions } from '../../../lib/session/store';
import { setPaneComponent, type PaneTabProps } from '../../../lib/panes/kinds';
import { appTermTheme } from '../../../lib/termTheme';
import { TERM_FONT_FAMILY, termFontsReady } from '../../../lib/termFonts';
import { BG_EXT, bgExitText, formatBytes, parseBgTasks, toTermText, type BgRow } from '../../../lib/status/bgTasks';
import { bgTasksTopics, bgTopicData } from '../../../lib/status/running';
import { ROW_STATUS_LABEL } from '../../../lib/status/subagentRows';
import { formatDurationMs, serviceCall } from '../../session/modalData';
import { tabBaseOf } from './sessionTabs';
import './tabs.css';

/** How often a running task's output is re-read. */
export const BG_OUTPUT_POLL_MS = 1_000;

export type BgOutput = { text: string; size: number; reset: boolean; task?: unknown };

export function BgTaskTab(props: PaneTabProps) {
  const base = () => tabBaseOf(props.tab.params, { hostId: props.hostId, sessionId: props.sessionId });
  const taskId = () => (typeof props.tab.params?.taskId === 'string' ? (props.tab.params.taskId as string) : '');
  const handle = () => (base().sessionId ? sessions().ensure(base().hostId, base().sessionId) : undefined);
  const [row, setRow] = createSignal<BgRow | undefined>();
  const [error, setError] = createSignal<string | undefined>();
  const [now, setNow] = createSignal(Date.now());
  let holder!: HTMLDivElement;
  let term: Terminal | undefined;
  let cursor: number | undefined;
  let opened = false;
  const pending: string[] = [];

  const write = (text: string, reset: boolean) => {
    if (reset) {
      pending.length = 0;
      term?.reset();
    }
    if (!text) return;
    if (opened && term) term.write(toTermText(text));
    else pending.push(text);
  };

  createEffect(() => {
    const h = handle();
    if (h && h.state.conn === 'live') h.subscribeTopics(bgTasksTopics(base().sessionId));
  });
  // The topic's snapshot — live while the stream is live, the fallback for the output service.
  const topicRow = createMemo(() => parseBgTasks(handle() ? bgTopicData(handle()!.state.topics, base().sessionId) : undefined)?.find((r) => r.id === taskId()));
  const shown = () => row() ?? topicRow();
  const running = () => shown()?.status === 'running';

  const read = async () => {
    const s = handle();
    const id = taskId();
    if (!s || !id) return;
    try {
      const out = await serviceCall<BgOutput>(s, 'bg.output', { id, ...(cursor !== undefined ? { since: cursor } : {}) }, BG_EXT);
      cursor = out.size;
      write(out.text ?? '', out.reset);
      const parsed = parseBgTasks({ tasks: [out.task] })?.[0];
      if (parsed) setRow(parsed);
      setError(undefined);
    } catch (e) {
      // No service (older extension) or the task is gone: show the snapshot's tail once.
      const tail = topicRow()?.tail;
      if (tail?.length && cursor === undefined) {
        cursor = -1;
        write(tail.join('\n') + '\n', true);
      } else if (!tail?.length) setError(e instanceof Error ? e.message : String(e));
    }
  };

  createEffect(() => {
    if (handle()?.state.conn === 'live') void read();
  });
  createEffect(() => {
    if (!running()) return;
    const timer = setInterval(() => {
      setNow(Date.now());
      void read();
    }, BG_OUTPUT_POLL_MS);
    onCleanup(() => clearInterval(timer));
  });
  // The task settled: one last read for the tail written between the last poll and the exit.
  let wasRunning = false;
  createEffect(() => {
    const r = running();
    if (wasRunning && !r) void read();
    wasRunning = r;
  });

  onMount(() => {
    let disposed = false;
    const fit = new FitAddon();
    const t = new Terminal({
      allowProposedApi: true,
      theme: appTermTheme(),
      fontFamily: TERM_FONT_FAMILY,
      fontSize: 12.5,
      lineHeight: 1.25,
      scrollback: 20_000,
      disableStdin: true,
      cursorBlink: false,
      cursorInactiveStyle: 'none',
      convertEol: true,
      drawBoldTextInBrightColors: false,
      minimumContrastRatio: 1,
    });
    t.loadAddon(fit);
    term = t;
    const doFit = () => {
      if (opened && holder.clientWidth > 20 && holder.clientHeight > 20) {
        try {
          fit.fit();
        } catch {
          /* not laid out yet */
        }
      }
    };
    const ro = new ResizeObserver(doFit);
    ro.observe(holder);
    void (async () => {
      await termFontsReady(12.5);
      for (let i = 0; i < 60 && holder.getBoundingClientRect().width < 20 && !disposed; i++) await new Promise((r) => requestAnimationFrame(r));
      if (disposed) return;
      t.open(holder);
      opened = true;
      doFit();
      for (const chunk of pending.splice(0)) t.write(toTermText(chunk));
    })();
    onCleanup(() => {
      disposed = true;
      ro.disconnect();
      t.dispose();
      term = undefined;
      opened = false;
    });
  });

  const stop = () => {
    const s = handle();
    if (!s) return;
    void serviceCall(s, 'work-tray.action', { tab: 'bg', actionId: `stop:${taskId()}`, value: null }, BG_EXT)
      .then(() => read())
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  };

  const name = () => shown()?.title ?? (typeof props.tab.params?.name === 'string' ? (props.tab.params.name as string) : 'Task');
  const elapsed = () => {
    const r = shown();
    return r?.startedAt ? formatDurationMs(Math.max(0, (r.status === 'running' ? now() : (r.endedAt ?? now())) - r.startedAt)) : undefined;
  };

  return (
    <div class="stab" data-testid="bgtask-tab" data-task={taskId()}>
      <header class="stab-head">
        <SquareTerminal size={15} class="stab-icon" />
        <span class="stab-title truncate" title={name()}>{name()}</span>
        <Show when={shown()}>
          {(r) => (
            <span class="stab-state bgt-state" data-state={r().status} data-testid="bgt-state">
              {ROW_STATUS_LABEL[r().status]}
              <Show when={bgExitText(r())}>{(x) => ` · ${x()}`}</Show>
            </span>
          )}
        </Show>
        <span class="grow" />
        <Show when={elapsed()}>
          <span class="bgt-meta">{elapsed()}</span>
        </Show>
        <Show when={shown()?.bytes}>
          <span class="bgt-meta">{formatBytes(shown()!.bytes)}</span>
        </Show>
        <Show when={running()}>
          <button type="button" class="stab-stop" onClick={stop} data-testid="bgt-stop">
            <Square size={12} />
            <span>Stop</span>
          </button>
        </Show>
      </header>
      <Show when={shown()?.command}>
        {(c) => <p class="bgt-cmd" title={c()} data-testid="bgt-cmd"><span aria-hidden="true">$</span> {c()}</p>}
      </Show>
      <Show when={error()}>
        <p class="stab-error">{error()}</p>
      </Show>
      <div class="bgt-term" ref={holder} data-testid="bgt-term" />
    </div>
  );
}

setPaneComponent('bgtask', BgTaskTab);
