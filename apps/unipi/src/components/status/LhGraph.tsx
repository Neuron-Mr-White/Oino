/**
 * P1-04: the long-horizon graph / swarm inspector inside the LH section —
 * the Mermaid DAG of the run and one row per work node (status, instruction)
 * with "Open" into the node's worker session (the subagent tab).
 *
 * Data: per-node state is not in the LH topics (counts only). The owner's
 * `/visualize-progress` answers with the reserved `unipi.long-horizon/visualize`
 * entry whose fallback tree carries the mermaid source and one
 * `- <work> [<status>] <instruction>` line per node; this view asks for it
 * (on open, then when the counts move) and reads that tree. Worker sessions
 * are the subagents titled `Graph · <work>` / `Swarm · <work>` (`_oino/agents`).
 */
import { createEffect, createMemo, createResource, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { ExternalLink, Maximize2, RefreshCw } from 'lucide-solid';
import type { SessionHandle } from '../../lib/session/store';
import { sessionUi } from '../../ui/registries';
import type { UiNode } from '../../ui/types';
import { renderMermaid } from '../../lib/markdown/mermaid';
import { openMermaidViewer } from '../chat/MermaidViewer';
import { fetchAgents } from '../panes/tabs/SubagentTab';
import { openSubagentTab } from '../panes/tabs/sessionTabs';
import { quietLhFocus } from '../../ui/slots/reserved';

export const LH_VISUALIZE_ID = 'unipi.long-horizon/visualize';

export type GraphNodeRow = { id: string; status: string; instruction: string };

/** The mermaid source and node lines of the `/visualize-progress` fallback tree. Pure. */
export function parseVisualize(tree: unknown): { mermaid?: string; nodes: GraphNodeRow[] } {
  const out: { mermaid?: string; nodes: GraphNodeRow[] } = { nodes: [] };
  const walk = (n: unknown) => {
    const node = n as { t?: string; source?: unknown; language?: unknown; children?: unknown[] } | null;
    if (!node || typeof node !== 'object') return;
    if (node.t === 'Code' && node.language === 'mermaid' && typeof node.source === 'string') out.mermaid = node.source;
    if (node.t === 'Markdown' && typeof node.source === 'string') {
      for (const line of node.source.split('\n')) {
        const m = /^- (\S+) \[([^\]]+)\] ?(.*)$/.exec(line.trim());
        if (m) out.nodes.push({ id: m[1]!, status: m[2]!, instruction: m[3]! });
      }
    }
    for (const c of node.children ?? []) walk(c);
  };
  walk(tree);
  return out;
}

/** The dot a node's status shows (running cyan, queued amber, done green, failed red, else dim). Pure. */
export function nodeDot(status: string): 'running' | 'queued' | 'done' | 'failed' | 'off' {
  if (status === 'running') return 'running';
  if (status === 'queued') return 'queued';
  if (status === 'completed' || status === 'done') return 'done';
  if (status === 'failed') return 'failed';
  return 'off';
}

/** A worker subagent's work id from its title (`Graph · w1`, `Swarm · w1`). Pure. */
export function workOfAgent(label: string): string | undefined {
  const m = /^(?:Graph|Swarm) · (.+)$/.exec(label.trim());
  return m?.[1];
}

export function LhGraph(props: { s: SessionHandle; counts: string; onToast?: (text: string) => void }) {
  const [asked, setAsked] = createSignal(0);
  const ask = () => {
    setAsked((n) => n + 1);
    quietLhFocus();
    void props.s.cmd({ type: 'prompt', message: '/visualize-progress' }).catch(() => props.onToast?.('Could not load the graph'));
  };
  // Ask on open and whenever the counts move (a node started / settled), debounced.
  createEffect(
    on(
      () => props.counts,
      () => {
        const t = setTimeout(ask, asked() === 0 ? 0 : 1200);
        onCleanup(() => clearTimeout(t));
      },
    ),
  );
  const entry = () => sessionUi(props.s.hostId, props.s.sessionId()).entry(LH_VISUALIZE_ID);
  const data = createMemo(() => {
    const view = entry()?.view as { kind?: string; fallback?: UiNode; root?: UiNode } | undefined;
    return parseVisualize(view?.kind === 'reserved' ? view.fallback : view?.root);
  });
  const [svg] = createResource(
    () => data().mermaid,
    async (source) => {
      const r = await renderMermaid(source);
      return r.ok ? r.svg : undefined;
    },
  );
  const [agents, { refetch }] = createResource(
    () => `${props.counts}#${asked()}`,
    () => fetchAgents(props.s),
  );
  const workerOf = (work: string) => (agents.latest ?? []).find((a) => workOfAgent(a.label ?? '') === work);
  const open = (work: string) => {
    const agent = workerOf(work);
    if (!agent) return;
    openSubagentTab({ hostId: props.s.hostId, sessionId: props.s.sessionId() }, { agentId: agent.id, name: agent.label });
  };
  const [expanded, setExpanded] = createSignal<string | undefined>();

  return (
    <div class="lh-graph" data-testid="lh-graph-inspector">
      <div class="lh-graph-head">
        <span class="status-dim">Graph</span>
        <span class="flex-1" />
        <Show when={svg.latest}>
          <button type="button" class="lh-icon-btn" aria-label="Open the graph full screen" title="Full screen" onClick={() => openMermaidViewer({ svg: svg.latest!, source: data().mermaid ?? '' })}>
            <Maximize2 size={13} />
          </button>
        </Show>
        <button type="button" class="lh-icon-btn" aria-label="Refresh the graph" title="Refresh" onClick={() => (ask(), void refetch())}>
          <RefreshCw size={13} />
        </button>
      </div>
      <Show when={svg.latest} fallback={<Show when={!data().nodes.length}><p class="status-empty">Loading the graph…</p></Show>}>
        {/* Sanitised by renderMermaid (DOMPurify). */}
        <button
          type="button"
          class="lh-graph-svg"
          aria-label="The run's graph — open full screen"
          onClick={() => openMermaidViewer({ svg: svg.latest!, source: data().mermaid ?? '' })}
          innerHTML={svg.latest}
        />
      </Show>
      <Show when={data().nodes.length > 0}>
        <ul class="lh-nodes" aria-label="Graph nodes">
          <For each={data().nodes}>
            {(node) => (
              <li class="lh-node" data-status={node.status} data-testid="lh-node">
                <span class="lh-dot" data-state={nodeDot(node.status)} />
                <button
                  type="button"
                  class="lh-node-main"
                  aria-expanded={expanded() === node.id}
                  title={node.instruction}
                  onClick={() => setExpanded((cur) => (cur === node.id ? undefined : node.id))}
                >
                  <span class="lh-node-id">{node.id}</span>
                  <span class="lh-node-status">{node.status}</span>
                  <span class="lh-node-text" data-open={expanded() === node.id ? '' : undefined}>
                    {node.instruction}
                  </span>
                </button>
                <Show when={workerOf(node.id)}>
                  <button type="button" class="lh-icon-btn" aria-label={`Open the ${node.id} worker session`} title="Open session" data-testid="lh-node-open" onClick={() => open(node.id)}>
                    <ExternalLink size={13} />
                  </button>
                </Show>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </div>
  );
}
