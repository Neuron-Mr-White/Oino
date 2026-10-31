/**
 * Tool body dispatcher (WP-A8, Z-39): native built-in GUI tools (`bash`,
 * `read`, `edit`, `write`, `grep`, `find`, `ls`, `todo`, `mcp__*`) render
 * synchronously with UniPi's own rich bodies (zero RPC roundtrip on expand,
 * readable unified diffs, `.md` preview/diff tabs, `Show full output`).
 * Extension-owned tools (`ask_user`, `bg_run`, `run_subagent`, custom tools)
 * check `_oino/render{target:"tool"}` through the session render cache and
 * show `IoBody` immediately while in flight or when no renderer is registered.
 */
import { Match, Switch } from 'solid-js';
import { toolCategory } from '../../../lib/chat/model';
import type { ToolRun } from '../../../lib/session/turns';
import { sessionRenderCache, widthClassOf } from '../../../lib/session/renderCache';
import { NodeView } from '../../../ui/NodeView';
import type { TimelineCtx } from '../context';
import { toolRenderData } from './toolData';
import { EditBody, HeadBody, IoBody, McpBody, RunBody, TodoBody, WriteBody } from './bodies';

const isTodo = (name: string) => /todo/i.test(name);

/** Core built-ins whose expanded views are owned natively by UniPi's GUI (`unipi.utility` `Regular` rule). */
const NATIVE_GUI_TOOLS = new Set([
  'bash',
  'powershell',
  'shell',
  'sh',
  'read',
  'edit',
  'write',
  'grep',
  'ffgrep',
  'find',
  'fffind',
  'glob',
  'ls',
  'list',
]);

export function ToolBody(props: { tool: ToolRun; ctx: TimelineCtx; /** Z-91: the row is folding shut. */ leaving?: boolean }) {
  const t = () => props.tool;
  const lower = () => t().name.trim().toLowerCase();
  const nativeOwned = () => NATIVE_GUI_TOOLS.has(lower()) || isTodo(lower()) || lower().startsWith('mcp__');
  const cat = () => toolCategory(t().name);
  const render = () => {
    if (nativeOwned()) return null;
    return sessionRenderCache(props.ctx.session).node(
      { target: 'tool', key: t().name, entryId: t().resultId ?? t().id, width: widthClassOf(props.ctx.columnWidth()) },
      { density: 'regular', expanded: true, running: t().state === 'running' || t().state === 'preparing', data: toolRenderData(t()) },
    )();
  };
  return (
    <div class="tool-body" data-leaving={props.leaving ? '' : undefined}>
      <Switch>
        <Match when={render()} keyed>
          {(node) => (
            <div class="tl-tool-render" data-testid="tool-render">
              <NodeView node={node} path={["render"]} mode="auto" />
            </div>
          )}
        </Match>
        <Match when={t().name.startsWith('mcp__')}>
          <McpBody tool={t()} ctx={props.ctx} />
        </Match>
        <Match when={isTodo(t().name)}>
          <TodoBody tool={t()} />
        </Match>
        <Match when={cat() === 'run'}>
          <RunBody tool={t()} ctx={props.ctx} />
        </Match>
        <Match when={cat() === 'edit' && lower() === 'write' && typeof t().args.content === 'string'}>
          <WriteBody tool={t()} ctx={props.ctx} />
        </Match>
        <Match when={cat() === 'edit' && hasEdits(t())}>
          <EditBody tool={t()} ctx={props.ctx} />
        </Match>
        <Match when={cat() === 'read' || cat() === 'search' || cat() === 'web' || cat() === 'memory'}>
          <HeadBody tool={t()} ctx={props.ctx} />
        </Match>
        <Match when={true}>
          <IoBody tool={t()} ctx={props.ctx} />
        </Match>
      </Switch>
    </div>
  );
}

function hasEdits(tool: ToolRun): boolean {
  const a = tool.args;
  if (Array.isArray(a.edits)) return (a.edits as unknown[]).length > 0;
  return (
    typeof a.oldText === 'string' ||
    typeof a.newText === 'string' ||
    typeof a.old_string === 'string' ||
    typeof a.new_string === 'string'
  );
}
