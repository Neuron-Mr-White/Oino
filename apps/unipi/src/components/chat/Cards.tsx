/**
 * UniPi event cards in the chat (UNI-113): one native look for unipi's
 * custom messages and entries. Most unipi messages carry harness meta
 * (`details.unipiHarness` {source, title, severity}); a few get their own
 * card (subagent / sidekick results, background tasks, command echo,
 * memory, compaction stats, kanboard badges, model changes).
 */
import { createSignal, For, Match, Show, Switch, type JSX } from 'solid-js';
import {
  Bell,
  Bot,
  Brain,
  CheckCircle2,
  ChevronDown,
  CircleX,
  Cpu,
  Database,
  Eraser,
  Hourglass,
  KanbanSquare,
  Layers,
  Sparkles,
  SquareTerminal,
  TriangleAlert,
} from 'lucide-solid';
import type { AttachmentRef, Entry } from '../../lib/chat/model';
import { attachmentsOf, textOf } from '../../lib/chat/model';
import { renderBlocks } from '../../lib/chat/markdown';
import { formatDuration } from '../../lib/chat/model';
import type { Chat } from '../../lib/chat/store';
import { MdBlock } from './MdBlock';
import { AttachmentChipRow } from './ImageChip';

type Known = { source: string; title: string; severity?: 'warning'; icon: () => JSX.Element };
const KNOWN: Record<string, Known> = {
  'unipi:lh-continue': { source: 'Long-horizon', title: 'Continuation', icon: () => <Hourglass size={15} /> },
  'unipi:kanboard-continue': { source: 'Kanboard', title: 'Unfinished task', icon: () => <KanbanSquare size={15} /> },
  'unipi:kanboard-next': { source: 'Kanboard', title: 'Next task', icon: () => <KanbanSquare size={15} /> },
  'unipi:kanboard-show': { source: 'Kanboard', title: 'Board', icon: () => <KanbanSquare size={15} /> },
  'unipi:kanboard-help': { source: 'Kanboard', title: 'Help', icon: () => <KanbanSquare size={15} /> },
  'unipi:kanboard-doctor': { source: 'Kanboard', title: 'Doctor', icon: () => <KanbanSquare size={15} /> },
  'unipi:kanboard-reminder': { source: 'Kanboard', title: 'Reminder', icon: () => <KanbanSquare size={15} /> },
  'unipi-memory-recall-reminder': { source: 'Memory', title: 'Memory recall', icon: () => <Database size={15} /> },
  'unipi-memory-retro-reminder': { source: 'Memory', title: 'Memory save?', icon: () => <Database size={15} /> },
  'unipi-skills-revealed': { source: 'Skills', title: 'Skill reveal', icon: () => <Sparkles size={15} /> },
  'unipi:plan-mode-message': { source: 'Plan mode', title: 'Plan mode', icon: () => <Layers size={15} /> },
  'unipi-watchdog': { source: 'Watchdog', title: 'Warning', severity: 'warning', icon: () => <TriangleAlert size={15} /> },
  'compactor-recall': { source: 'Compactor', title: 'Session recall', icon: () => <Layers size={15} /> },
  'unipi-response': { source: 'UniPi', title: 'Response', icon: () => <Sparkles size={15} /> },
  'btw-note': { source: 'btw', title: 'Side note', icon: () => <Sparkles size={15} /> },
};

function Card(props: {
  icon: JSX.Element;
  source: string;
  title: string;
  meta?: string;
  tone?: 'warning' | 'error' | 'ok';
  body?: string;
  markdown?: boolean;
  /** Body visible without tapping. */
  open?: boolean;
  chat?: Chat;
  attachments?: AttachmentRef[];
}) {
  const [open, setOpen] = createSignal(!!props.open);
  const has = () => !!props.body?.trim();
  return (
    <div class="ucard" data-tone={props.tone}>
      <button type="button" class="ucard-head" disabled={!has()} aria-expanded={has() ? open() : undefined} onClick={() => setOpen(!open())}>
        <span class="ucard-icon">{props.icon}</span>
        <span class="ucard-source">{props.source}</span>
        <span class="ucard-title">{props.title}</span>
        <Show when={props.meta}>
          <span class="ucard-meta">{props.meta}</span>
        </Show>
        <Show when={has()}>
          <ChevronDown size={14} class="chat-chev" data-open={open() ? '' : undefined} />
        </Show>
      </button>
      <Show when={open() && has()}>
        <Show when={props.markdown} fallback={<pre class="ucard-body">{props.body}</pre>}>
          <div class="ucard-body md">
            <For each={renderBlocks(props.body!, false)}>{(html) => <MdBlock html={html} />}</For>
          </div>
        </Show>
      </Show>
      <Show when={props.chat && props.attachments?.length}>
        <div class="ucard-attachments">
          <AttachmentChipRow chat={props.chat!} attachments={props.attachments!} />
        </div>
      </Show>
    </div>
  );
}

const money = (n?: number) => (typeof n === 'number' && n >= 0.0005 ? `$${n.toFixed(n < 0.1 ? 3 : 2)}` : undefined);
const dur = (ms?: number) => (typeof ms === 'number' ? formatDuration(ms) : undefined);
const join = (...parts: Array<string | undefined>) => parts.filter(Boolean).join(' · ') || undefined;

export function UnipiCard(props: { entry: Entry; chat?: Chat }) {
  const e = () => props.entry as Entry & { customType?: string; content?: unknown; details?: Record<string, unknown>; data?: Record<string, unknown> };
  const ct = () => e().customType ?? e().type;
  const details = () => (e().details ?? {}) as Record<string, any>;
  const data = () => (e().data ?? {}) as Record<string, any>;
  const body = () => textOf(e().content as never);
  const attachments = () => attachmentsOf(e().details);

  return (
    <Switch
      fallback={
        <Card
          icon={(KNOWN[ct()]?.icon ?? (() => <Sparkles size={15} />))()}
          source={details().unipiHarness?.source ?? KNOWN[ct()]?.source ?? 'UniPi'}
          title={details().unipiHarness?.title ?? KNOWN[ct()]?.title ?? ct()}
          tone={(details().unipiHarness?.severity ?? KNOWN[ct()]?.severity) === 'warning' ? 'warning' : undefined}
          body={body() || (e().type === 'custom' ? JSON.stringify(data(), null, 2) : '')}
          markdown={!!body()}
          chat={props.chat}
          attachments={attachments()}
        />
      }
    >
      <Match when={ct() === 'subagent-completion'}>
        <Card
          icon={<Bot size={15} />}
          source="Subagent"
          title={details().title ?? 'Finished'}
          meta={join(details().status !== 'completed' ? details().status : undefined, dur(details().durationMs), money(details().usage?.cost))}
          tone={details().status === 'completed' ? 'ok' : 'error'}
          body={details().report ?? body()}
          markdown
          chat={props.chat}
          attachments={attachments()}
        />
      </Match>
      <Match when={ct() === 'sidekick-completion'}>
        <Card
          icon={<Bot size={15} />}
          source="Sidekick"
          title={(details().text ?? body()).split('\n').find((l: string) => l.trim())?.replace(/^#+\s*/, '').slice(0, 80) ?? 'Finished'}
          meta={join(dur(details().durationMs), money(details().usage?.cost))}
          tone={details().status === 'completed' ? 'ok' : 'error'}
          body={details().text ?? body()}
          markdown
          chat={props.chat}
          attachments={attachments()}
        />
      </Match>
      <Match when={ct() === 'background-task-notification'}>
        <Card
          icon={details().exitCode === 0 ? <CheckCircle2 size={15} /> : details().status === 'running' ? <Bell size={15} /> : <CircleX size={15} />}
          source="Background task"
          title={details().name ?? 'Task'}
          meta={join(details().status, details().exitCode !== undefined && details().exitCode !== null ? `exit ${details().exitCode}` : undefined)}
          tone={details().exitCode === 0 ? 'ok' : details().status === 'running' ? undefined : 'error'}
          body={[details().command ? `$ ${details().command}` : '', ...(Array.isArray(details().outputTail) ? details().outputTail : [])].filter(Boolean).join('\n')}
          chat={props.chat}
          attachments={attachments()}
        />
      </Match>
      <Match when={ct() === 'unipi-command-echo'}>
        <div class="cmd-echo">
          <span class="cmd-echo-mark">❭</span>
          <span class="cmd-echo-text">{String(data().text ?? '')}</span>
        </div>
      </Match>
      <Match when={ct() === 'unipi-command-output'}>
        {/* UNI-276: what a command said (its ui.notify), as its reply in the chat. */}
        <Show
          when={!data().screen}
          fallback={
            <div class="cmd-reply dim" data-testid="cmd-reply-screen">
              <SquareTerminal size={14} />
              <span>{String(data().command ?? 'This command')} opened a screen in the terminal.</span>
            </div>
          }
        >
          <div class="cmd-reply" data-level={data().level} data-testid="cmd-reply">
            <Show when={data().level === 'error' || data().level === 'warning'} fallback={<Bell size={14} />}>
              <TriangleAlert size={14} />
            </Show>
            <div class="cmd-reply-text">{String(data().text ?? '')}</div>
          </div>
        </Show>
      </Match>
      <Match when={ct() === 'unipi-memory-session-card'}>
        <Card icon={<Brain size={15} />} source="Memory" title={`${data().project ?? 'Project'} · ${data().count ?? 0} ${data().count === 1 ? 'memory' : 'memories'}`} body={data().wakeUp} markdown />
      </Match>
      <Match when={ct() === 'unipi-memory-save-card'}>
        <Card
          icon={<Database size={15} />}
          source="Memory"
          title={`Saved ${(data().titles ?? []).length} ${(data().titles ?? []).length === 1 ? 'memory' : 'memories'}`}
          body={(data().titles ?? []).map((t: string) => `- ${t}`).join('\n')}
          tone="ok"
          markdown
        />
      </Match>
      <Match when={ct() === 'unipi-compaction'}>
        <Card
          icon={<Layers size={15} />}
          source="Compactor"
          title={`${Math.round((data().tokensBefore ?? 0) / 1000)}k → ${Math.round((data().tokensAfter ?? 0) / 1000)}k tokens`}
          meta={join(data().method, data().trigger)}
          body={(data().sections ?? []).map((s: string) => `- ${s}`).join('\n')}
          markdown
        />
      </Match>
      <Match when={ct() === 'unipi:kanboard-badge' || ct() === 'unipi:kanboard-notice'}>
        <div class="cmd-echo kb">
          <KanbanSquare size={14} />
          <span class="cmd-echo-text">
            {data().id ? `${data().id} ` : ''}
            {String(data().kind ?? data().text ?? data().message ?? '')}
          </span>
        </div>
      </Match>
      <Match when={ct() === 'long-horizon-decision'}>
        <Show when={data().mode && data().mode !== 'none'}>
          <div class="cmd-echo">
            <Hourglass size={14} />
            <span class="cmd-echo-text">long-horizon {String(data().mode)}</span>
          </div>
        </Show>
      </Match>
      <Match when={ct() === 'unipi-cleared-input'}>
        <div class="cmd-echo dim">
          <Eraser size={14} />
          <span class="cmd-echo-text">Cleared: {String(data().text ?? '')}</span>
        </div>
      </Match>
      <Match when={ct() === 'unipi-progress'}>
        <div class="cmd-echo">
          <Hourglass size={14} />
          <span class="cmd-echo-text">{String(data().label ?? data().text ?? 'Progress')}</span>
        </div>
      </Match>
      <Match when={ct() === 'user-bash'}>
        <Card
          icon={<SquareTerminal size={15} />}
          source="Shell"
          title={String(data().command ?? '')}
          meta={data().exitCode !== undefined ? `exit ${data().exitCode}` : undefined}
          tone={data().exitCode && data().exitCode !== 0 ? 'error' : undefined}
          body={String(data().output ?? '')}
        />
      </Match>
      <Match when={ct() === 'model_change'}>
        <div class="cmd-echo dim">
          <Cpu size={14} />
          <span class="cmd-echo-text">Model → {String((e() as { modelId?: string }).modelId ?? '')}</span>
        </div>
      </Match>
    </Switch>
  );
}
