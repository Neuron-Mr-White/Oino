/**
 * Chat event row ([chat-events]): one compact card per non-prose event —
 * its own icon and accent, "Source · one-line summary", a meta and a
 * chevron — that expands to the full body (markdown, plain text or pretty
 * JSON in a code block; never raw JSON as prose). Kinds whose owning
 * extension draws the body (`view.renderer`) ask `_oino/render` with
 * `expanded: true` only once opened, and show the app's body while the
 * answer is in flight or `null`.
 *
 * Desktop expands inline (the open state lives in the expansion store, so a
 * virtual-scroll remount keeps it and the virtualizer re-measures the row via
 * its ResizeObserver); the phone opens the body in a full sheet like tool
 * rows (PH-27: never a growing inline card).
 */
import { createSignal, For, Match, Show, Switch, type JSX } from 'solid-js';
import { Dynamic } from 'solid-js/web';
import {
  Bell,
  Bot,
  BotMessageSquare,
  BrainCircuit,
  Camera,
  ChevronRight,
  CircleCheck,
  CircleX,
  ClipboardList,
  DatabaseZap,
  GitBranch,
  Hourglass,
  Layers,
  MessageCircleQuestion,
  MoonStar,
  Paperclip,
  Repeat,
  ScrollText,
  Sparkles,
  SquareKanban,
  SquareTerminal,
  Target,
  TriangleAlert,
  Wand,
  type LucideIcon,
} from 'lucide-solid';
import type { SessionEntry } from '../../../lib/session/types';
import type { EventIcon, EventView } from '../../../lib/timeline/eventKinds';
import { renderBlocks } from '../../../lib/chat/markdown';
import { expansionOpen, toggleExpansion } from '../../../lib/timeline/expansion';
import { sessionRenderCache, widthClassOf } from '../../../lib/session/renderCache';
import { createPresence } from '../../../lib/motion';
import { layout } from '../../../lib/layout';
import { NodeView } from '../../../ui/NodeView';
import { MdBlock } from '../../chat/MdBlock';
import { Sheet } from '../../phone';
import type { TimelineCtx } from '../context';

export const EVENT_ICON: Record<EventIcon, LucideIcon> = {
  memory: BrainCircuit,
  memorySave: DatabaseZap,
  memoryWake: BrainCircuit,
  agent: Bot,
  sidekick: BotMessageSquare,
  bgDone: CircleCheck,
  bgFailed: CircleX,
  bgRunning: Bell,
  shell: SquareTerminal,
  compactor: Layers,
  kanboard: SquareKanban,
  mode: Hourglass,
  ralph: Repeat,
  goal: Target,
  skills: Wand,
  plan: ClipboardList,
  watchdog: TriangleAlert,
  dream: MoonStar,
  btw: MessageCircleQuestion,
  browser: Camera,
  attach: Paperclip,
  system: ScrollText,
  branch: GitBranch,
  response: Sparkles,
  command: SquareTerminal,
  generic: Sparkles,
};

/** Accent family per icon (CSS `data-family` → `--ev`). */
const FAMILY: Record<EventIcon, string> = {
  memory: 'memory',
  memorySave: 'memory',
  memoryWake: 'memory',
  agent: 'agent',
  sidekick: 'agent',
  bgDone: 'task',
  bgFailed: 'task',
  bgRunning: 'task',
  shell: 'shell',
  command: 'shell',
  compactor: 'compactor',
  kanboard: 'board',
  mode: 'mode',
  ralph: 'mode',
  goal: 'mode',
  plan: 'mode',
  skills: 'harness',
  watchdog: 'warn',
  dream: 'dream',
  btw: 'harness',
  browser: 'harness',
  attach: 'harness',
  system: 'system',
  branch: 'system',
  response: 'harness',
  generic: 'harness',
};

export function EventCard(props: { entry: SessionEntry; view: EventView; ctx: TimelineCtx }) {
  const v = () => props.view;
  const key = () => `ev:${props.entry.id}`;
  const phone = () => !layout().desktop();
  const open = () => expansionOpen(props.ctx.sessionId(), key(), false);
  const [sheet, setSheet] = createSignal(false);
  const has = () => !!v().body || !!v().renderer;
  const toggle = () => {
    if (!has()) return;
    if (phone()) setSheet(true);
    else toggleExpansion(props.ctx.sessionId(), key(), false);
  };
  const body = createPresence(() => open() && !phone());
  const label = () => [v().source, v().title, v().meta].filter(Boolean).join(' · ');
  return (
    <div
      class="tl-event"
      data-event={v().kind}
      data-family={FAMILY[v().icon]}
      data-tone={v().tone}
      data-open={open() && !phone() ? '' : undefined}
      data-testid="tl-event"
    >
      <button
        type="button"
        class="tl-event-head"
        aria-expanded={has() ? (phone() ? sheet() : open()) : undefined}
        aria-disabled={has() ? undefined : 'true'}
        title={label()}
        onClick={toggle}
        data-testid="tl-event-head"
      >
        <span class="tl-event-icon">
          <Dynamic component={EVENT_ICON[v().icon]} size={15} />
        </span>
        <span class="tl-event-source">{v().source}</span>
        <span class="tl-event-title">{v().title}</span>
        <Show when={v().meta}>
          <span class="tl-event-meta">{v().meta}</span>
        </Show>
        <Show when={has()}>
          <ChevronRight size={15} class="tl-event-chev" data-open={open() && !phone() ? '' : undefined} />
        </Show>
      </button>
      <Show when={!phone() && body.mounted()}>
        <div class="tl-event-bodywrap" data-leaving={body.leaving() ? '' : undefined}>
          <EventBodyView entry={props.entry} view={v()} ctx={props.ctx} />
        </div>
      </Show>
      <Show when={phone() && has()}>
        <Sheet
          open={sheet()}
          onClose={() => setSheet(false)}
          label={v().source}
          title={
            <span class="ph-tool-sheet-title">
              {v().source}
              <span class="ph-tool-sheet-sub">{v().title}</span>
            </span>
          }
          detents={['full']}
          initialDetent="full"
          class="ph-tool-sheet"
          testId="event-sheet"
        >
          <Show when={sheet()}>
            <div class="tl-event-sheetbody">
              <Show when={v().meta}>
                <div class="tl-event-sheetmeta">{v().meta}</div>
              </Show>
              <EventBodyView entry={props.entry} view={v()} ctx={props.ctx} />
            </div>
          </Show>
        </Sheet>
      </Show>
    </div>
  );
}

/** The expanded body: the owning extension's node when it draws one, else the app's own. */
export function EventBodyView(props: { entry: SessionEntry; view: EventView; ctx: TimelineCtx }): JSX.Element {
  const node = () => {
    const target = props.view.renderer;
    const ct = props.entry.customType;
    if (!target || !ct) return null;
    return sessionRenderCache(props.ctx.session).node(
      { target, key: ct, entryId: props.entry.id, width: widthClassOf(props.ctx.columnWidth()), expanded: true },
      {
        density: 'regular',
        expanded: true,
        data: { entry: props.entry, message: props.entry.message ?? null, customType: ct, data: props.entry.data ?? null, content: (props.entry as { content?: unknown }).content ?? null, details: (props.entry as { details?: unknown }).details ?? null },
      },
    )();
  };
  return (
    <Show
      when={node()}
      keyed
      fallback={
        <Switch fallback={<div class="tl-event-body tl-event-empty">Nothing more to show.</div>}>
          <Match when={props.view.body?.format === 'markdown'}>
            <div class="tl-event-body md" data-testid="tl-event-body" data-format="markdown">
              <For each={renderBlocks(props.view.body!.text, false)}>{(html) => <MdBlock html={html} chat={props.ctx.refs} />}</For>
            </div>
          </Match>
          <Match when={props.view.body}>
            <pre class="tl-event-body tl-event-pre" data-testid="tl-event-body" data-format={props.view.body!.format}>
              {props.view.body!.text}
            </pre>
          </Match>
        </Switch>
      }
    >
      {(n) => (
        <div class="tl-event-body tl-event-render" data-testid="tl-event-render">
          <NodeView node={n} path={['render']} mode="auto" />
        </div>
      )}
    </Show>
  );
}
