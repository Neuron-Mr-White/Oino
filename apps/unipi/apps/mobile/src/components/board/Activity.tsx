/**
 * Activity timeline of a task (web TaskPanel ActivityText, UNI-131):
 * newest first, every entry rendered as sanitised markdown, status
 * changes with a report label (Finished / Blocked / Rework …), long
 * entries clamped with "Show more", reports and long entries open in
 * the full-text reader. The phone shows 15 at a time ("Show older").
 */
import { createSignal, For, Show } from 'solid-js';
import { BookOpen, Bot, Settings2, User } from 'lucide-solid';
import type { Activity, Task } from '../../lib/board/api';
import { relativeTime } from '../../lib/board/markdown';
import { splitReport } from '../../lib/board/report';
import { boardUi } from '../../lib/board/ui';
import { Markdown } from './Markdown';

const PAGE = 15;

export function Avatar(props: { actor: string }) {
  return (
    <span class={`avatar actor-${props.actor}`} aria-hidden="true">
      {props.actor === 'agent' ? <Bot size={15} /> : props.actor === 'system' ? <Settings2 size={14} /> : <User size={15} />}
    </span>
  );
}

export function ActivityList(props: { task: Task }) {
  const [shown, setShown] = createSignal(PAGE);
  const entries = () => [...(props.task.activity ?? [])].reverse();
  return (
    <>
      <ul class="timeline" aria-label="Activity">
        <For each={entries().slice(0, shown())}>{(entry) => <ActivityEntry task={props.task} entry={entry} />}</For>
      </ul>
      <Show when={entries().length > shown()}>
        <button type="button" class="section-more" onClick={() => setShown((count) => count + PAGE * 2)}>
          Show {Math.min(PAGE * 2, entries().length - shown())} older
        </button>
      </Show>
    </>
  );
}

function ActivityEntry(props: { task: Task; entry: Activity }) {
  const ui = boardUi();
  const [expanded, setExpanded] = createSignal(false);
  const [overflows, setOverflows] = createSignal(false);
  const parts = () => splitReport(props.entry.text);
  let body: HTMLDivElement | undefined;
  const measure = () => requestAnimationFrame(() => body && setOverflows(body.scrollHeight > body.clientHeight + 1));
  return (
    <li class="timeline-entry" data-actor={props.entry.actor}>
      <Avatar actor={props.entry.actor} />
      <div class="min-w-0 flex-1">
        <div class="timeline-who">
          <strong>{props.entry.actor}</strong>
          <Show when={props.entry.session}>
            <span class="timeline-session font-mono truncate">{props.entry.session}</span>
          </Show>
          <time dateTime={props.entry.at} title={props.entry.at}>
            {relativeTime(props.entry.at)}
          </time>
        </div>
        <Show when={parts().label}>
          <span class={`report-tag ${parts().label!.toLowerCase()}`}>{parts().label}</span>
        </Show>
        <div ref={(el) => ((body = el), measure())} class="timeline-what" classList={{ clamped: !expanded(), system: props.entry.actor === 'system' }}>
          <Markdown text={parts().body} />
        </div>
        <Show when={overflows() || expanded() || parts().label}>
          <div class="timeline-actions">
            <Show when={overflows() || expanded()}>
              <button type="button" class="link-button" onClick={() => setExpanded((open) => !open)}>
                {expanded() ? 'Show less' : 'Show more'}
              </button>
            </Show>
            <button
              type="button"
              class="link-button"
              aria-label={`Read ${parts().label ?? 'note'} in full`}
              onClick={() =>
                ui.setReader({ title: `${props.task.id} · ${parts().label ?? 'Note'}`, text: parts().body, actor: props.entry.actor, at: props.entry.at })
              }
            >
              <BookOpen size={14} /> Read
            </button>
          </div>
        </Show>
      </div>
    </li>
  );
}
