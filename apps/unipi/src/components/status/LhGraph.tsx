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
 *
 * [graph-view] A current host also puts the structured nodes in the entry's
 * `params` (`{graph, nodes}`): each node's profile and — once it ran — the
 * model + thinking level from the subagents run record. Then the app draws its
 * own Mermaid (short `profile · model` labels, truncated on the phone, status
 * classes) and a muted second line per row; an older host's tree still works
 * (an indented `  - ran on <profile> · <model> · thinking <level>` line is read
 * when present).
 */
import { createEffect, createMemo, createResource, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { ExternalLink, Loader2, Maximize2, RefreshCw } from 'lucide-solid';
import type { SessionHandle } from '../../lib/session/store';
import { sessionUi } from '../../ui/registries';
import type { UiNode } from '../../ui/types';
import { renderMermaid } from '../../lib/markdown/mermaid';
import { openMermaidViewer } from '../chat/MermaidViewer';
import { fetchAgents } from '../panes/tabs/SubagentTab';
import { openSubagentTab } from '../panes/tabs/sessionTabs';
import { quietLhFocus } from '../../ui/slots/reserved';
import { prettyModelId } from '../../lib/model/displayName';
import { layout } from '../../lib/layout';

export const LH_VISUALIZE_ID = 'unipi.long-horizon/visualize';

export type GraphNodeRow = {
  id: string;
  status: string;
  instruction: string;
  /** Profile id (`subagent_general`, `subagent_explore`, a custom agent). */
  profile?: string;
  /** `provider/model` the node's subagent ran on (the run record; absent until it ran). */
  model?: string;
  thinking?: string;
  inputs?: string[];
  failure?: string;
  supersededBy?: string;
  /** Tool calls of the settled run. */
  toolCalls?: number;
};

export type GraphVisual = { mermaid?: string; nodes: GraphNodeRow[]; structured?: boolean };

const str = (v: unknown): string | undefined => (typeof v === 'string' && v.trim() ? v.trim() : undefined);

/** The structured nodes of the reserved entry's `params` (`{nodes: [...]}`), if any. Pure. */
export function parseVisualizeParams(params: unknown): GraphNodeRow[] | undefined {
  const list = (params as { nodes?: unknown } | null | undefined)?.nodes;
  if (!Array.isArray(list)) return undefined;
  const out: GraphNodeRow[] = [];
  for (const raw of list.slice(0, 200)) {
    const n = raw as Record<string, unknown> | null;
    const id = str(n?.id);
    if (!n || !id) continue;
    const row: GraphNodeRow = { id, status: str(n.status) ?? 'queued', instruction: str(n.instruction) ?? '' };
    for (const k of ['profile', 'model', 'thinking', 'failure', 'supersededBy'] as const) {
      const v = str(n[k]);
      if (v) row[k] = v;
    }
    if (Array.isArray(n.inputs)) row.inputs = n.inputs.filter((i): i is string => typeof i === 'string');
    if (typeof n.toolCalls === 'number' && n.toolCalls > 0) row.toolCalls = Math.floor(n.toolCalls);
    out.push(row);
  }
  return out;
}

/** `  - ran on <profile> · <provider/model> · thinking <level>` (the fallback tree's second line). Pure. */
function parseRanOn(line: string): Pick<GraphNodeRow, 'profile' | 'model' | 'thinking'> | undefined {
  const m = /^- ran on (.+)$/.exec(line.trim());
  if (!m) return undefined;
  const parts = m[1]!.split(' · ').map((p) => p.trim());
  const out: Pick<GraphNodeRow, 'profile' | 'model' | 'thinking'> = {};
  if (parts[0]) out.profile = parts[0];
  for (const p of parts.slice(1)) {
    if (p.startsWith('thinking ')) out.thinking = p.slice(9);
    else if (p) out.model = p;
  }
  return out;
}

/** The mermaid source and node lines of the `/visualize-progress` fallback tree, or the
 *  structured `params` nodes when the host sent them. Pure. */
export function parseVisualize(tree: unknown, params?: unknown): GraphVisual {
  const out: GraphVisual = { nodes: [] };
  const walk = (n: unknown) => {
    const node = n as { t?: string; source?: unknown; language?: unknown; children?: unknown[] } | null;
    if (!node || typeof node !== 'object') return;
    if (node.t === 'Code' && node.language === 'mermaid' && typeof node.source === 'string') out.mermaid = node.source;
    if (node.t === 'Markdown' && typeof node.source === 'string') {
      for (const line of node.source.split('\n')) {
        const m = /^- (\S+) \[([^\]]+)\] ?(.*)$/.exec(line.trim());
        if (m) {
          out.nodes.push({ id: m[1]!, status: m[2]!, instruction: m[3]! });
          continue;
        }
        const last = out.nodes[out.nodes.length - 1];
        const ran = /^\s+- ran on /.test(line) ? parseRanOn(line) : undefined;
        if (last && ran) Object.assign(last, ran);
      }
    }
    for (const c of node.children ?? []) walk(c);
  };
  walk(tree);
  const structured = parseVisualizeParams(params);
  if (structured && structured.length) {
    out.nodes = structured;
    out.structured = true;
  }
  return out;
}

/** `subagent_explore` → `explore`; custom agent ids stay. Pure. */
export function shortProfile(profile: string): string {
  return profile.replace(/^subagent_/, '');
}

/** The muted second line of a node: `rel-qa · GLM 5.3 · high` (empty before it ran
 *  and without a profile). Pure. */
export function nodeRunLabel(node: Pick<GraphNodeRow, 'profile' | 'model' | 'thinking' | 'toolCalls'>): string {
  const tools = node.toolCalls ? `${node.toolCalls} tool call${node.toolCalls === 1 ? '' : 's'}` : undefined;
  return [node.profile ? shortProfile(node.profile) : undefined, node.model ? prettyModelId(node.model) : undefined, node.model ? node.thinking : undefined, tools]
    .filter(Boolean)
    .join(' · ');
}

const clipLabel = (text: string, max: number) => (text.length > max ? `${text.slice(0, Math.max(1, max - 1))}…` : text);
const mermaidId = (id: string) => `n_${id.replace(/^r-/, '').replace(/[^A-Za-z0-9_]/g, '_')}`;
const mermaidText = (text: string) => text.replace(/["[\]<>{}|]/g, ' ');

/** The run class of a status (Mermaid `classDef` + row style). Pure. */
export function statusClass(status: string): 'running' | 'done' | 'failed' | 'blocked' | 'off' | 'queued' {
  if (status === 'running') return 'running';
  if (status === 'completed' || status === 'done') return 'done';
  if (status === 'failed') return 'failed';
  if (status === 'blocked') return 'blocked';
  if (status === 'superseded' || status === 'stopped' || status === 'cancelled') return 'off';
  return 'queued';
}

/**
 * The app's Mermaid of structured nodes: `<work> · <status>` over a short
 * `<profile> · <model>` label (clipped to `max` chars — the phone passes a smaller
 * one), input edges, dotted supersession edges and one class per status. Pure.
 */
export function graphMermaid(nodes: readonly GraphNodeRow[], max = 30): string {
  const lines = ['flowchart TD'];
  const known = new Set(nodes.map((n) => n.id));
  const classes = new Map<string, string[]>();
  for (const n of nodes) {
    const id = mermaidId(n.id);
    const run = [n.profile ? shortProfile(n.profile) : undefined, n.model ? prettyModelId(n.model) : undefined].filter(Boolean).join(' · ');
    const head = mermaidText(clipLabel(`${n.id} · ${n.status}`, max));
    lines.push(`  ${id}["${head}${run && n.model ? `<br/>${mermaidText(clipLabel(run, max))}` : ''}"]`);
    for (const input of n.inputs ?? []) {
      const from = input.replace(/^r-/, '');
      if (known.has(from)) lines.push(`  ${mermaidId(from)} --> ${id}`);
    }
    if (n.supersededBy && known.has(n.supersededBy)) lines.push(`  ${id} -.-> ${mermaidId(n.supersededBy)}`);
    const cls = statusClass(n.status);
    if (cls !== 'queued') classes.set(cls, [...(classes.get(cls) ?? []), id]);
  }
  if (classes.size) {
    lines.push(
      '  classDef running stroke:#28BEF0,stroke-width:2px',
      '  classDef done stroke:#5AD25A',
      '  classDef failed stroke:#F03C32,stroke-width:2px,color:#F03C32',
      '  classDef blocked stroke:#F0A818,stroke-dasharray:4',
      '  classDef off stroke:#6B635C,color:#8A8178,stroke-dasharray:3',
    );
    for (const [cls, ids] of classes) lines.push(`  class ${ids.join(',')} ${cls}`);
  }
  return lines.join('\n');
}

/** The dot a node's status shows (running cyan, queued amber, done green, failed red,
 *  blocked amber ring, superseded/stopped/cancelled dim). Pure. */
export function nodeDot(status: string): 'running' | 'queued' | 'done' | 'failed' | 'blocked' | 'off' {
  const cls = statusClass(status);
  if (cls === 'queued') return status === 'queued' ? 'queued' : 'off';
  return cls;
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
    const view = entry()?.view as { kind?: string; fallback?: UiNode; root?: UiNode; params?: unknown } | undefined;
    return parseVisualize(view?.kind === 'reserved' ? view.fallback : view?.root, view?.kind === 'reserved' ? view.params : undefined);
  });
  // Structured nodes: the app's own Mermaid (short labels; the phone clips harder).
  const source = createMemo(() => {
    const d = data();
    return d.structured ? graphMermaid(d.nodes, layout().desktop() ? 30 : 18) : d.mermaid;
  });
  const [svg] = createResource(
    () => source(),
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
          <button type="button" class="lh-icon-btn" aria-label="Open the graph full screen" title="Full screen" onClick={() => openMermaidViewer({ svg: svg.latest!, source: source() ?? '' })}>
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
          onClick={() => openMermaidViewer({ svg: svg.latest!, source: source() ?? '' })}
          innerHTML={svg.latest}
        />
      </Show>
      <Show when={data().nodes.length > 0}>
        <ul class="lh-nodes" aria-label="Graph nodes">
          <For each={data().nodes}>
            {(node) => (
              <li class="lh-node" data-status={node.status} data-state={statusClass(node.status)} data-testid="lh-node">
                <Show when={node.status === 'running'} fallback={<span class="lh-dot" data-state={nodeDot(node.status)} />}>
                  <Loader2 size={12} class="lh-node-spin" aria-hidden="true" data-testid="lh-node-spinner" />
                </Show>
                <button
                  type="button"
                  class="lh-node-main"
                  aria-expanded={expanded() === node.id}
                  title={[node.instruction, node.failure].filter(Boolean).join('\n')}
                  onClick={() => setExpanded((cur) => (cur === node.id ? undefined : node.id))}
                >
                  <span class="lh-node-id">{node.id}</span>
                  <span class="lh-node-status">{node.status}</span>
                  <span class="lh-node-text" data-open={expanded() === node.id ? '' : undefined}>
                    {node.instruction}
                  </span>
                  <Show when={nodeRunLabel(node)}>
                    {(label) => (
                      <span
                        class="lh-node-run"
                        data-testid="lh-node-run"
                        title={[node.profile && `Profile: ${node.profile}`, node.model && `Model: ${node.model}`, node.model && node.thinking && `Thinking: ${node.thinking}`].filter(Boolean).join('\n')}
                      >
                        {label()}
                      </span>
                    )}
                  </Show>
                  <Show when={expanded() === node.id && node.failure}>
                    {(f) => <span class="lh-node-failure">{f()}</span>}
                  </Show>
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
