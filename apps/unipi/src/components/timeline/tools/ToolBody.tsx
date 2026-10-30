/**
 * Tool body dispatcher (WP-A8, Z-39): an extension render-only override
 * (`_oino/render{target:"tool"}`) wins when it answers a node — rendered
 * through U9's `NodeView`; otherwise the native unipi body by category
 * (run / edit / write / heads), the todo and mcp renderers, In/Out
 * fallback. Mounted only while the row is open (lazy detail).
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

export function ToolBody(props: { tool: ToolRun; ctx: TimelineCtx; /** Z-91: the row is folding shut. */ leaving?: boolean }) {
  const t = () => props.tool;
  const cat = () => toolCategory(t().name);
  const render = () =>
    sessionRenderCache(props.ctx.session).node(
      { target: 'tool', key: t().name, entryId: t().resultId ?? t().id, width: widthClassOf(props.ctx.columnWidth()) },
      { density: 'advanced', expanded: true, running: t().state === 'running' || t().state === 'preparing', data: toolRenderData(t()) },
    )();
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
          <McpBody tool={t()} />
        </Match>
        <Match when={isTodo(t().name)}>
          <TodoBody tool={t()} />
        </Match>
        <Match when={cat() === 'run'}>
          <RunBody tool={t()} ctx={props.ctx} />
        </Match>
        <Match when={cat() === 'edit' && t().name === 'write' && typeof t().args.content === 'string'}>
          <WriteBody tool={t()} ctx={props.ctx} />
        </Match>
        <Match when={cat() === 'edit' && hasEdits(t())}>
          <EditBody tool={t()} ctx={props.ctx} />
        </Match>
        <Match when={cat() === 'read' || cat() === 'search' || cat() === 'web' || cat() === 'memory'}>
          <HeadBody tool={t()} ctx={props.ctx} />
        </Match>
        <Match when={true}>
          <IoBody tool={t()} />
        </Match>
      </Switch>
    </div>
  );
}

function hasEdits(tool: ToolRun): boolean {
  const a = tool.args;
  if (Array.isArray(a.edits)) return (a.edits as unknown[]).length > 0;
  return typeof a.oldText === 'string' || typeof a.newText === 'string';
}
