/**
 * UNI-222: long-horizon progress (goal / ralph / swarm / graph) and the pure
 * pieces of the LH status view (moved from lib/chat/progress.ts by WP-A1;
 * the data arrives as topics / reserved panels after WP-A2, §3.10): the mermaid source for
 * graph/swarm (nodes coloured by status via classDef, deps as edges, waves
 * as subgraphs) and small formatters.
 */

import { createSignal } from 'solid-js';

/** The Progress sheet's open state — module-level so the desktop palette can open it too. */
export const [progressSheetOpen, setProgressSheetOpen] = createSignal(false);

export type LhItemStatus = 'queued' | 'ready' | 'running' | 'done' | 'failed' | 'aborted';
export type LhRunStatus = 'running' | 'paused' | 'done' | 'failed' | 'stopped';
export type LhMode = 'goal' | 'ralph' | 'swarm' | 'graph';

export type LhItem = {
  id: string;
  label: string;
  status: LhItemStatus;
  deps: string[];
  wave?: number;
  summary?: string;
  attempts?: number;
  /** UNI-282 (graph/swarm steps): the subagent that runs/ran it — `work_transcript{id: operatorId}`. */
  operatorId?: string;
  /** `r-<id>` once completed. */
  resultId?: string;
  profile?: string;
  startedAt?: number;
  endedAt?: number;
  toolCalls?: number;
  /** Input + output tokens, once settled. */
  tokens?: number;
};

export type LhRun = {
  mode: LhMode;
  title: string;
  status: LhRunStatus;
  reason?: string;
  items: LhItem[];
  counts: { total: number; done: number; running: number; failed: number; queued: number };
  ralph?: { name: string; iteration: number; maxIterations: number; checked: number; total: number };
  goal?: { objective: string; status: string; turn: number; maxTurns: number; percent?: number; summary?: string; estimatedAt?: number };
  endedAt?: number;
  /** UNI-282: the durable agent graph id (graph/swarm). */
  graphId?: string;
  /** UNI-282: result ids the supervisor's `finish` chose. */
  selected?: string[];
};

export type LhLogLine = { at: number; text: string; item?: string; status?: LhItemStatus | LhRunStatus };

export type LhProgress = {
  v: 1;
  mode: LhMode | 'none';
  current?: LhRun;
  last?: LhRun;
  log: LhLogLine[];
  updatedAt: number;
};

export const MODE_LABEL: Record<LhMode, string> = { goal: 'Goal', ralph: 'Ralph', swarm: 'Swarm', graph: 'Graph' };
export const ITEM_LABEL: Record<LhItemStatus, string> = { queued: 'Queued', ready: 'Ready', running: 'Running', done: 'Done', failed: 'Failed', aborted: 'Aborted' };
export const RUN_LABEL: Record<LhRunStatus, string> = { running: 'Running', paused: 'Paused', done: 'Done', failed: 'Finished with failures', stopped: 'Stopped' };

/** The run the sheet shows: the current one, else the last finished one. */
export function shownRun(p: LhProgress | undefined): { run: LhRun; live: boolean } | undefined {
  if (p?.current) return { run: p.current, live: p.current.status === 'running' };
  if (p?.last) return { run: p.last, live: false };
  return undefined;
}

/** Mermaid-safe node id (ids are model-chosen strings). */
export function nodeId(id: string, index: number): string {
  return `n${index}_${id.replace(/[^A-Za-z0-9_]/g, '_').slice(0, 24)}`;
}

/** Text inside a quoted mermaid label: no quotes / brackets / markup survive. */
export function mermaidText(text: string, max = 42): string {
  const flat = text
    .replace(/\s+/g, ' ')
    .replace(/["`]/g, "'")
    .replace(/[<>{}[\]|#;]/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
  return flat.length > max ? `${flat.slice(0, max - 1)}…` : flat;
}

const GLYPH: Record<LhItemStatus, string> = { queued: '·', ready: '○', running: '▶', done: '✓', failed: '✗', aborted: '⊘' };

/**
 * Status colours from the app tokens (DESIGN.md §1.1–1.3): working = claw,
 * done = wm-green, failed = wm-red, aborted = wm-yellow, waiting = surfaces.
 */
export const CLASS_DEFS = [
  'classDef queued fill:#1C1917,stroke:#2E2825,color:#A39A90',
  'classDef ready fill:#1C1917,stroke:#A39A90,color:#F5EFE6',
  'classDef running fill:#3A2210,stroke:#F07818,color:#F5EFE6,stroke-width:2px',
  'classDef done fill:#16301A,stroke:#5AD25A,color:#F5EFE6',
  'classDef failed fill:#3A1412,stroke:#F03C32,color:#F5EFE6,stroke-width:2px',
  'classDef aborted fill:#33290E,stroke:#FAC828,color:#F5EFE6',
];

/** Swarm chart columns (a phone-width grid). */
export const SWARM_COLS = 3;

/** Wave / item-group boxes: surface-1 on the chart's surface, line border, muted title. */
const SUBGRAPH_STYLE = 'fill:#141211,stroke:#2E2825,color:#A39A90';

/**
 * Mermaid flowchart for a graph/swarm run. Graph: one subgraph per wave
 * (left→right), an edge per dependency. Swarm: one "items" subgraph, no
 * edges. Deterministic for the same run (the renderer caches by source).
 */
export function progressMermaid(run: LhRun): string {
  const lines: string[] = ['flowchart LR'];
  const ids = new Map<string, string>();
  run.items.forEach((item, i) => ids.set(item.id, nodeId(item.id, i)));
  const node = (item: LhItem) => {
    const label = `${GLYPH[item.status]} ${mermaidText(item.id, 28)}<br/>${mermaidText(item.label)}`;
    return `    ${ids.get(item.id)}["${label}"]:::${item.status}`;
  };
  if (run.mode === 'graph') {
    const waves = new Map<number, LhItem[]>();
    for (const item of run.items) {
      const w = item.wave ?? 0;
      waves.set(w, [...(waves.get(w) ?? []), item]);
    }
    for (const w of [...waves.keys()].sort((a, b) => a - b)) {
      const items = waves.get(w)!;
      const done = items.filter((i) => i.status === 'done').length;
      lines.push(`  subgraph wave${w}["Wave ${w + 1} · ${done}/${items.length}"]`);
      lines.push('    direction TB');
      for (const item of items) lines.push(node(item));
      lines.push('  end');
      lines.push(`  style wave${w} ${SUBGRAPH_STYLE}`);
    }
    for (const item of run.items) {
      for (const dep of item.deps) {
        const from = ids.get(dep);
        if (from) lines.push(`  ${from} --> ${ids.get(item.id)}`);
      }
    }
  } else {
    lines.push(`  subgraph items["${mermaidText(run.title, 60)} · ${runSettled(run)}/${run.counts.total}"]`);
    for (const item of run.items) lines.push(node(item));
    // Rows of SWARM_COLS: invisible links chain a row left→right; unlinked rows stack.
    for (let i = 0; i < run.items.length; i += SWARM_COLS) {
      const row = run.items.slice(i, i + SWARM_COLS).map((it) => ids.get(it.id));
      if (row.length > 1) lines.push(`    ${row.join(' ~~~ ')}`);
    }
    lines.push('  end');
    lines.push(`  style items ${SUBGRAPH_STYLE}`);
  }
  lines.push(...CLASS_DEFS.map((d) => `  ${d}`));
  return lines.join('\n');
}

/** `12s ago` / `4m ago` / `2h ago`. */
export function agoText(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s}s ago`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  return h < 48 ? `${h}h ago` : `${Math.round(h / 24)}d ago`;
}

/** `14:05:09`. */
export function clockText(at: number): string {
  const d = new Date(at);
  const p = (n: number) => String(n).padStart(2, '0');
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

/** Settled items (done + failed/aborted) — the progress number; running and waiting don't count. */
export function runSettled(run: LhRun): number {
  return run.counts.done + run.counts.failed;
}

/** Settled / total as a 0..100 integer (0 for an empty run). */
export function runPercent(run: LhRun): number {
  if (run.mode === 'goal') return run.goal?.percent ?? 0;
  if (run.mode === 'ralph' && run.ralph) return run.ralph.total ? Math.round((run.ralph.checked / run.ralph.total) * 100) : 0;
  return run.counts.total ? Math.round((runSettled(run) / run.counts.total) * 100) : 0;
}

export type BarSegment = { tone: 'done' | 'failed' | 'running'; count: number; pct: number };

/** The status bar's coloured segments in order done · failed · running (the grey rest is the track). */
export function runSegments(counts: LhRun['counts']): BarSegment[] {
  const total = counts.total;
  if (total <= 0) return [];
  let left = total;
  const take = (n: number) => {
    const v = Math.max(0, Math.min(left, n));
    left -= v;
    return v;
  };
  const parts: Array<[BarSegment['tone'], number]> = [
    ['done', take(counts.done)],
    ['failed', take(counts.failed)],
    ['running', take(counts.running)],
  ];
  return parts.filter(([, n]) => n > 0).map(([tone, count]) => ({ tone, count, pct: (count / total) * 100 }));
}

/** UNI-282: one step's full result (`lh_graph{op:"output"}`). */
export type LhGraphOutput = {
  graphId: string;
  id: string;
  status: string;
  resultId?: string;
  operatorId?: string;
  instruction: string;
  text?: string;
  failure?: string;
  truncated?: boolean;
};

/** UNI-282: one row of a subagent's mini-transcript (`work_transcript`). */
export type TranscriptItem =
  | { kind: 'task' | 'text'; text: string }
  | { kind: 'tool'; name: string; arg: string; output: string; isError: boolean; running: boolean; durationMs?: number };

export type WorkTranscript = { id: string; items: TranscriptItem[]; report?: string };

/** A failed request: the reason, or (a host/extension that lacks the op) `unsupported`. */
export type GraphError = { error: string; unsupported?: boolean };

/** "Unknown command/type" from a host or extension that lacks the op. */
export function isUnknownOpError(message: string | undefined | null): boolean {
  return typeof message === 'string' && /^unknown (type|command)(\s|:|$)/i.test(message.trim());
}

/** The app's text for a failed agent-graph request; names what was being attempted
 * when the host/extension does not know the op. */
export function graphErrorText(message: string, what: string): GraphError {
  return isUnknownOpError(message) ? { error: `${what} needs a newer unipi on the PC.`, unsupported: true } : { error: message };
}

// Moved from the deleted Dream sheet helpers (lib/chat/dream.ts, WP-A1).
/** `42s`, `3m 05s`, `1h 12m`. */
export function durationText(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${String(s % 60).padStart(2, '0')}s`;
  return `${Math.floor(m / 60)}h ${String(m % 60).padStart(2, '0')}m`;
}

/** "Today 14:02", "Yesterday 09:10", "Oct 3, 14:02". */
export function startedText(ms: number, now = Date.now()): string {
  const d = new Date(ms);
  const time = `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  const today = new Date(now);
  const y = new Date(now - 86_400_000);
  if (d.toDateString() === today.toDateString()) return `Today ${time}`;
  if (d.toDateString() === y.toDateString()) return `Yesterday ${time}`;
  return `${d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}, ${time}`;
}

/** A quick shape check for one LhRun off the wire (history rows). */
export function isRun(value: unknown): value is LhRun {
  const r = value as Partial<LhRun> | undefined;
  return !!r && typeof r === 'object' && typeof r.mode === 'string' && Array.isArray(r.items) && !!r.counts && typeof r.counts === 'object';
}

/** Is this item's result one the supervisor's finish chose? */
export function isSelected(run: LhRun, item: LhItem): boolean {
  return !!item.resultId && !!run.selected?.includes(item.resultId);
}

/** `1.2k` / `34k` / `1.4M` tokens. */
export function tokensText(n: number): string {
  if (n < 1000) return String(n);
  if (n < 10_000) return `${(n / 1000).toFixed(1).replace(/\.0$/, '')}k`;
  if (n < 1_000_000) return `${Math.round(n / 1000)}k`;
  return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, '')}M`;
}

/** A step's run time: settled (start → end) or live (start → now); undefined before it starts. */
export function stepDurationMs(item: LhItem, now: number): number | undefined {
  if (item.startedAt === undefined) return undefined;
  if (item.endedAt !== undefined) return Math.max(0, item.endedAt - item.startedAt);
  return item.status === 'running' ? Math.max(0, now - item.startedAt) : undefined;
}

/** A quick sanity check on an untrusted payload (the wire is JSON from another process). */
export function isProgress(value: unknown): value is LhProgress {
  return !!value && typeof value === 'object' && (value as { v?: unknown }).v === 1 && Array.isArray((value as { log?: unknown }).log);
}
