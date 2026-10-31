/**
 * Tool bodies (WP-A8, Z-39 hybrid): keep unipi's bodies — run terminal,
 * edit diff, write diff (+ `.md` markdown preview), read/search/web/memory
 * heads — and add the todo and mcp renderers and the In/Out fallback.
 * All bodies mount only inside an expanded row (lazy detail, §3.6).
 */
import { createMemo, createSignal, For, Match, Show, Switch } from 'solid-js';
import { CircleCheck, CircleDashed, ExternalLink, Loader2 } from 'lucide-solid';
import { diffStat, toolSubject } from '../../../../lib/chat/model';
import { allAdditions, editDiff, renderDiffHtml } from '../../../../lib/markdown';
import { languageForPath } from '../../../../lib/markdown';
import { renderBlocks } from '../../../../lib/chat/markdown';
import type { ToolRun } from '../../../../lib/session/turns';
import type { TimelineCtx } from '../../context';
import { MdBlock } from '../../../chat/MdBlock';
import { ImageChipRow } from '../../../chat/ImageChip';
import { openDiffTab } from '../../../panes/tabs/sessionTabs';
import { toolImages } from '../toolData';
import { useFullOutput } from './fullOutput';

export const tail = (s: string, n: number) => (s.length > n ? `…\n${s.slice(s.length - n)}` : s);
export const head = (s: string, n: number) => (s.length > n ? `${s.slice(0, n)}\n…` : s);
const OUT_TAIL = 6000;
const OUT_HEAD = 3000;
const OUT_IO = 4000;

function FullOutputControls(props: {
  trimmed: boolean;
  expanded: boolean;
  canFold: boolean;
  loading: boolean;
  onExpand: () => void;
  onFold: () => void;
}) {
  return (
    <Show
      when={!props.loading}
      fallback={
        <span class="tool-more tool-more-loading" role="status" data-testid="tool-loading-output">
          <Loader2 size={12} class="tool-more-spin" />
          Loading full output…
        </span>
      }
    >
      <Show when={props.trimmed}>
        <button type="button" class="tool-more" onClick={props.onExpand} data-testid="tool-full-output">
          Show full output
        </button>
      </Show>
      <Show when={props.expanded && props.canFold}>
        <button type="button" class="tool-more" onClick={props.onFold} data-testid="tool-fold-output">
          Fold output
        </button>
      </Show>
    </Show>
  );
}

/** `$ cmd` + output tail; "Show full output" pulls the entry (`get_entries`) inline without hiding the tail. */
export function RunBody(props: { tool: ToolRun; ctx: TimelineCtx }) {
  const t = () => props.tool;
  const { full, loading, snapshotTruncated, load } = useFullOutput(props.ctx, t);
  const [all, setAll] = createSignal(false);
  const base = () => t().output ?? t().live ?? '';
  const text = () => full() ?? base();
  const needsFullFetch = () => full() === undefined && !!t().resultId && (snapshotTruncated() || base().length > OUT_TAIL);
  const trimmed = () => (!all() && text().length > OUT_TAIL) || needsFullFetch();
  const shown = () => (all() ? text() : tail(text(), OUT_TAIL));
  const images = () => toolImages(props.ctx, t());
  return (
    <div class="tool-term">
      <div class="tool-term-cmd">
        <span class="tool-term-prompt">$</span>
        <span>{String(t().args.command ?? t().args.cmd ?? t().name)}</span>
      </div>
      <Show when={shown()}>
        <pre class="tool-term-out" data-all={all() ? '' : undefined} data-error={t().state === 'error' ? '' : undefined}>
          {shown()}
        </pre>
      </Show>
      <Show when={images().length}>
        <ImageChipRow chat={props.ctx.refs} images={images()} />
      </Show>
      <FullOutputControls
        trimmed={trimmed()}
        expanded={all()}
        canFold={text().length > OUT_TAIL}
        loading={loading()}
        onExpand={() => {
          load();
          setAll(true);
        }}
        onFold={() => setAll(false)}
      />
    </div>
  );
}

/** Z-40 "View diff": the same change in the full `diff` side-pane tab (at the edit's line). */
export function ViewDiffAction(props: { tool: ToolRun; ctx: TimelineCtx }) {
  const path = () => {
    const p = props.tool.args.path ?? props.tool.args.file_path;
    return typeof p === 'string' ? p : '';
  };
  const open = () =>
    openDiffTab(
      { hostId: props.ctx.hostId, sessionId: props.ctx.sessionId(), cwd: props.ctx.cwd() },
      { toolId: props.tool.id, path: path() },
    );
  return (
    <Show when={path()}>
      <button type="button" class="tool-more tool-view-diff" onClick={open} data-testid="tool-view-diff">
        View diff
        <ExternalLink size={12} />
      </button>
    </Show>
  );
}

/** Unified diff of `edit` calls (one hunk per edit pair). */
export function EditBody(props: { tool: ToolRun; ctx: TimelineCtx }) {
  const t = () => props.tool;
  const displayPath = () => toolSubject({ name: t().name, args: t().args }, props.ctx.cwd()) || undefined;
  const edits = () => {
    const a = t().args;
    if (Array.isArray(a.edits)) {
      return (a.edits as Array<{ oldText?: string; newText?: string; old_string?: string; new_string?: string }>).map((e) => ({
        oldText: e.oldText ?? e.old_string,
        newText: e.newText ?? e.new_string,
      }));
    }
    if (typeof a.oldText === 'string' || typeof a.newText === 'string' || typeof a.old_string === 'string' || typeof a.new_string === 'string') {
      return [
        {
          oldText: (a.oldText ?? a.old_string) as string | undefined,
          newText: (a.newText ?? a.new_string) as string | undefined,
        },
      ];
    }
    return [];
  };
  const html = createMemo(() =>
    renderDiffHtml({
      files: [
        {
          newPath: displayPath(),
          hunks: edits().map((e) => editDiff(e.oldText ?? '', e.newText ?? '')),
        },
      ],
    }),
  );
  return (
    <div class="tool-edit-wrap">
      <div class="tool-diff diffview-wrap" innerHTML={html()} />
      <Show when={t().state === 'error' && t().output}>
        <pre class="tool-err">{tail(t().output!, 1200)}</pre>
      </Show>
      <ViewDiffAction tool={t()} ctx={props.ctx} />
    </div>
  );
}

/** `write`: all-additions diff; a `.md` file previews as markdown (Z-39). */
export function WriteBody(props: { tool: ToolRun; ctx: TimelineCtx }) {
  const t = () => props.tool;
  const content = () => (typeof t().args.content === 'string' ? (t().args.content as string) : undefined);
  const displayPath = () => toolSubject({ name: t().name, args: t().args }, props.ctx.cwd()) || undefined;
  const isMd = () => /\.(md|markdown)$/i.test(String(t().args.path ?? t().args.file_path ?? ''));
  const [preview, setPreview] = createSignal(isMd());
  const diffHtml = createMemo(() => {
    const parsed = allAdditions(content() ?? '');
    if (parsed.files[0] && displayPath()) parsed.files[0].newPath = displayPath();
    return renderDiffHtml(parsed);
  });
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
      <ViewDiffAction tool={t()} ctx={props.ctx} />
    </div>
  );
}

/** read / search / web / memory: the output head, full text on demand. */
export function HeadBody(props: { tool: ToolRun; ctx: TimelineCtx }) {
  const t = () => props.tool;
  const { full, loading, snapshotTruncated, load } = useFullOutput(props.ctx, t);
  const [all, setAll] = createSignal(false);
  const base = () => t().output ?? t().live ?? '';
  const text = () => full() ?? base();
  const needsFullFetch = () => full() === undefined && !!t().resultId && (snapshotTruncated() || base().length > OUT_HEAD);
  const trimmed = () => (!all() && text().length > OUT_HEAD) || needsFullFetch();
  const images = () => toolImages(props.ctx, t());
  return (
    <div class="tool-readwrap">
      <Show when={text()}>
        <pre class="tool-read" data-all={all() ? '' : undefined} data-error={t().state === 'error' ? '' : undefined}>
          {all() ? text() : head(text(), OUT_HEAD)}
        </pre>
      </Show>
      <Show when={images().length}>
        <ImageChipRow chat={props.ctx.refs} images={images()} />
      </Show>
      <FullOutputControls
        trimmed={trimmed()}
        expanded={all()}
        canFold={text().length > OUT_HEAD}
        loading={loading()}
        onExpand={() => {
          load();
          setAll(true);
        }}
        onFold={() => setAll(false)}
      />
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
export function McpBody(props: { tool: ToolRun; ctx?: TimelineCtx }) {
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
      <IoBody tool={props.tool} ctx={props.ctx} />
    </div>
  );
}

/** The generic In/Out fallback, with lazy full output when truncated. */
export function IoBody(props: { tool: ToolRun; ctx?: TimelineCtx }) {
  const t = () => props.tool;
  const fullOut = props.ctx ? useFullOutput(props.ctx, t) : undefined;
  const [all, setAll] = createSignal(false);
  const base = () => t().output ?? t().live ?? '';
  const out = () => fullOut?.full() ?? base();
  const needsFullFetch = () => !!fullOut && fullOut.full() === undefined && !!t().resultId && (fullOut.snapshotTruncated() || base().length > OUT_IO);
  const trimmed = () => (!all() && out().length > OUT_IO) || needsFullFetch();
  const hasArgs = () => Object.keys(t().args).length > 0;
  return (
    <div class="tool-io-wrap">
      <div class="tool-io">
        <Show when={hasArgs()}>
          <div class="tool-io-sec">
            <span class="tool-io-label">In</span>
            <pre>{JSON.stringify(t().args, null, 2)}</pre>
          </div>
        </Show>
        <Show when={out()}>
          <div class="tool-io-sec" data-error={t().state === 'error' ? '' : undefined}>
            <span class="tool-io-label">Out</span>
            <pre data-all={all() ? '' : undefined}>{all() ? out() : tail(out(), OUT_IO)}</pre>
          </div>
        </Show>
      </div>
      <Show when={fullOut}>
        {(fo) => (
          <FullOutputControls
            trimmed={trimmed()}
            expanded={all()}
            canFold={out().length > OUT_IO}
            loading={fo().loading()}
            onExpand={() => {
              fo().load();
              setAll(true);
            }}
            onFold={() => setAll(false)}
          />
        )}
      </Show>
    </div>
  );
}
