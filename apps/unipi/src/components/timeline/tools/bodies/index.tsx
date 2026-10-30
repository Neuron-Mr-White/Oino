/**
 * Tool bodies (WP-A8, Z-39 hybrid): keep unipi's bodies — run terminal,
 * edit diff, write diff (+ `.md` markdown preview), read/search/web/memory
 * heads — and add the todo and mcp renderers and the In/Out fallback.
 * All bodies mount only inside an expanded row (lazy detail, §3.6).
 */
import { createMemo, createSignal, For, Match, Show, Switch } from 'solid-js';
import { CircleCheck, CircleDashed, Loader2 } from 'lucide-solid';
import { diffStat } from '../../../../lib/chat/model';
import { allAdditions, editDiff, renderDiffHtml } from '../../../../lib/markdown';
import { languageForPath } from '../../../../lib/markdown';
import { renderBlocks } from '../../../../lib/chat/markdown';
import type { ToolRun } from '../../../../lib/session/turns';
import type { TimelineCtx } from '../../context';
import { MdBlock } from '../../../chat/MdBlock';
import { ImageChipRow } from '../../../chat/ImageChip';
import { toolImages } from '../toolData';
import { useFullOutput } from './fullOutput';

export const tail = (s: string, n: number) => (s.length > n ? `…\n${s.slice(s.length - n)}` : s);
export const head = (s: string, n: number) => (s.length > n ? `${s.slice(0, n)}\n…` : s);
const OUT_TAIL = 6000;
const OUT_HEAD = 3000;

/** `$ cmd` + output tail; "Show full output" pulls the entry (`get_entries`). */
export function RunBody(props: { tool: ToolRun; ctx: TimelineCtx }) {
  const t = () => props.tool;
  const { full, load } = useFullOutput(props.ctx, t());
  const [all, setAll] = createSignal(false);
  const base = () => t().output ?? t().live ?? '';
  const text = () => full() ?? base();
  const trimmed = () => !all() && text().length > OUT_TAIL;
  const shown = () => (all() ? text() : tail(text(), OUT_TAIL));
  const images = () => toolImages(props.ctx, t());
  return (
    <div class="tool-term">
      <div class="tool-term-cmd">
        <span class="tool-term-prompt">$</span>
        <span>{String(t().args.command ?? t().args.cmd ?? t().name)}</span>
      </div>
      <Show when={shown()}>
        <pre class="tool-term-out" data-error={t().state === 'error' ? '' : undefined}>
          {shown()}
        </pre>
      </Show>
      <Show when={images().length}>
        <ImageChipRow chat={props.ctx.refs} images={images()} />
      </Show>
      <Show when={trimmed() || (full() === undefined && base().length > OUT_TAIL)}>
        <button type="button" class="tool-more" onClick={() => (load(), setAll(true))} data-testid="tool-full-output">
          Show full output
        </button>
      </Show>
    </div>
  );
}

/** Unified diff of `edit` calls (one hunk per edit pair). */
export function EditBody(props: { tool: ToolRun; ctx: TimelineCtx }) {
  const t = () => props.tool;
  const edits = () => {
    const a = t().args;
    if (Array.isArray(a.edits)) return a.edits as Array<{ oldText?: string; newText?: string }>;
    if (typeof a.oldText === 'string' || typeof a.newText === 'string') return [{ oldText: a.oldText as string | undefined, newText: a.newText as string | undefined }];
    return [];
  };
  const html = createMemo(() => renderDiffHtml({ files: [{ hunks: edits().map((e) => editDiff(e.oldText ?? '', e.newText ?? '')) }] }));
  return (
    <>
      <div class="tool-diff diffview-wrap" innerHTML={html()} />
      <Show when={t().state === 'error' && t().output}>
        <pre class="tool-err">{tail(t().output!, 1200)}</pre>
      </Show>
    </>
  );
}

/** `write`: all-additions diff; a `.md` file previews as markdown (Z-39). */
export function WriteBody(props: { tool: ToolRun; ctx: TimelineCtx }) {
  const t = () => props.tool;
  const content = () => (typeof t().args.content === 'string' ? (t().args.content as string) : undefined);
  const isMd = () => /\.(md|markdown)$/i.test(String(t().args.path ?? t().args.file_path ?? ''));
  const [preview, setPreview] = createSignal(isMd());
  const diffHtml = createMemo(() => renderDiffHtml(allAdditions(content() ?? '')));
  const stat = () => diffStat({ name: t().name, args: t().args });
  return (
    <div class="tool-write">
      <Show
        when={content() !== undefined && isMd()}
        fallback={<div class="tool-diff diffview-wrap" innerHTML={diffHtml()} />}
      >
        <div class="tool-write-tabs" role="tablist">
          <button type="button" role="tab" aria-selected={preview()} class="tool-write-tab" onClick={() => setPreview(true)} data-testid="tool-write-preview">
            Preview
          </button>
          <button type="button" role="tab" aria-selected={!preview()} class="tool-write-tab" onClick={() => setPreview(false)} data-testid="tool-write-diff">
            Diff <Show when={stat()}>+{stat()!.add}</Show>
          </button>
        </div>
        <Show
          when={preview()}
          fallback={<div class="tool-diff diffview-wrap" innerHTML={diffHtml()} />}
        >
          <div class="tool-write-md md" data-lang={languageForPath(String(t().args.path ?? ''))}>
            <For each={renderBlocks(content()!, false)}>{(html) => <MdBlock html={html} chat={props.ctx.refs} />}</For>
          </div>
        </Show>
      </Show>
      <Show when={t().state === 'error' && t().output}>
        <pre class="tool-err">{tail(t().output!, 1200)}</pre>
      </Show>
    </div>
  );
}

/** read / search / web / memory: the output head, full text on demand. */
export function HeadBody(props: { tool: ToolRun; ctx: TimelineCtx }) {
  const t = () => props.tool;
  const { full, load } = useFullOutput(props.ctx, t());
  const [all, setAll] = createSignal(false);
  const base = () => t().output ?? t().live ?? '';
  const text = () => full() ?? base();
  const trimmed = () => !all() && text().length > OUT_HEAD;
  const images = () => toolImages(props.ctx, t());
  return (
    <div class="tool-readwrap">
      <Show when={text()}>
        <pre class="tool-read" data-error={t().state === 'error' ? '' : undefined}>
          {all() ? text() : head(text(), OUT_HEAD)}
        </pre>
      </Show>
      <Show when={images().length}>
        <ImageChipRow chat={props.ctx.refs} images={images()} />
      </Show>
      <Show when={trimmed()}>
        <button type="button" class="tool-more" onClick={() => (load(), setAll(true))} data-testid="tool-full-output">
          Show full output
        </button>
      </Show>
    </div>
  );
}

export type TodoItem = { content?: string; status?: string; activeForm?: string };

/** The todo tool (`todo_write`-shaped): a checklist with live/done/pending states. */
export function TodoBody(props: { tool: ToolRun }) {
  const items = () => {
    const a = props.tool.args;
    const raw = Array.isArray(a.todos) ? a.todos : Array.isArray(a.tasks) ? a.tasks : [];
    return (raw as TodoItem[]).filter((x) => x && typeof x === 'object');
  };
  const icon = (status: string | undefined, i: number) => (
    <Switch>
      <Match when={status === 'completed'}>
        <CircleCheck size={14} class="todo-done" />
      </Match>
      <Match when={status === 'in_progress' || (i === 0 && !status)}>
        <Loader2 size={14} class="todo-live" />
      </Match>
      <Match when={true}>
        <CircleDashed size={14} class="todo-pending" />
      </Match>
    </Switch>
  );
  return (
    <ul class="tool-todo" data-testid="tool-todo">
      <For each={items()}>
        {(item, i) => (
          <li class="tool-todo-item" data-status={item.status ?? 'pending'} data-active={item.status === 'in_progress' ? '' : undefined}>
            {icon(item.status, i())}
            <span>{item.activeForm || item.content || 'Untitled step'}</span>
          </li>
        )}
      </For>
    </ul>
  );
}

/** `mcp__server__tool`: server · tool header over the In/Out body. */
export function McpBody(props: { tool: ToolRun }) {
  const parts = () => props.tool.name.split('__');
  const server = () => (parts().length >= 3 ? parts()[1] : undefined);
  const fn = () => parts().at(-1);
  return (
    <div class="tool-mcp">
      <Show when={server()}>
        <div class="tool-mcp-head">
          <span class="tool-mcp-server">{server()}</span>
          <span class="tool-mcp-dot" />
          <span class="tool-mcp-fn">{fn()}</span>
        </div>
      </Show>
      <IoBody tool={props.tool} />
    </div>
  );
}

/** The generic In/Out fallback. */
export function IoBody(props: { tool: ToolRun }) {
  const t = () => props.tool;
  const out = () => t().output ?? t().live ?? '';
  return (
    <div class="tool-io">
      <div class="tool-io-sec">
        <span class="tool-io-label">In</span>
        <pre>{JSON.stringify(t().args, null, 2)}</pre>
      </div>
      <Show when={out()}>
        <div class="tool-io-sec" data-error={t().state === 'error' ? '' : undefined}>
          <span class="tool-io-label">Out</span>
          <pre>{tail(out(), 4000)}</pre>
        </div>
      </Show>
    </div>
  );
}
