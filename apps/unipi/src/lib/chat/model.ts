/**
 * Chat model (UNI-113): pi session entries + live stream → what the Chat
 * screen draws. Pure functions, no Solid; the store (store.ts) feeds them.
 *
 * Layout (deepseek-harness pattern, folded by default — DESIGN §3.4):
 *   user bubble
 *   turn: [process group: thinking + tool calls, folded to one line]
 *         assistant prose (markdown)
 *         … repeated for each step of the turn
 *   unipi cards (custom messages / entries) inline where they happened.
 */
import { editDiff } from '../markdown';

export type Part =
  | { type: 'text'; text: string }
  | { type: 'thinking'; thinking: string; redacted?: boolean }
  | { type: 'toolCall'; id: string; name: string; arguments: unknown }
  /** An image part (bridge `phoneSafe()`): small ones carry `data` (base64,
   *  `omitted:false`) and render right away; bigger ones are `omitted:true`
   *  with a `mediaRef` the phone fetches with `media{mediaRef}`. Oino session
   *  entries carry pi's `mimeType`; the v1 bridge said `mime`. */
  | { type: 'image'; mime?: string; mimeType?: string; omitted?: boolean; bytes?: number; data?: string; mediaRef?: string }
  | { type: string; [key: string]: unknown };

/** One image reference extracted from a message/tool-result for the
 * transcript's chip strip / previewer (see `imagesOf`). */
export type ImageRef = { mime: string; data?: string; mediaRef?: string; bytes?: number };

/** One `details.attachments[]` entry (bridge `attachmentSafe()`, UNI-157):
 * a file the agent produced/referenced (kanboard upload, screenshot path,
 * tool-written file…), fetchable via `mediaRef` or (after `file_share`)
 * `blob_get{path}`. */
export type AttachmentRef = { path: string; mime: string; size?: number; caption?: string; mediaRef?: string };

/** `details.attachments[]`, slimmed and filtered to well-formed entries. */
export function attachmentsOf(details: unknown): AttachmentRef[] {
  const list = (details as { attachments?: unknown } | null)?.attachments;
  if (!Array.isArray(list)) return [];
  return list
    .map((raw) => raw as { path?: unknown; mime?: unknown; size?: unknown; caption?: unknown; mediaRef?: unknown })
    .filter((a): a is { path: string; mime?: unknown; size?: unknown; caption?: unknown; mediaRef?: unknown } => typeof a?.path === 'string')
    .map((a) => ({
      path: a.path,
      mime: typeof a.mime === 'string' ? a.mime : 'application/octet-stream',
      size: typeof a.size === 'number' ? a.size : undefined,
      caption: typeof a.caption === 'string' ? a.caption : undefined,
      mediaRef: typeof a.mediaRef === 'string' ? a.mediaRef : undefined,
    }));
}

export type Usage = { input?: number; output?: number; cacheRead?: number; cost?: { total?: number } };

export type Message =
  | { role: 'user'; content: string | Part[]; timestamp?: number; unipiHarness?: Harness }
  | {
      role: 'assistant';
      content: Part[];
      stopReason?: string;
      errorMessage?: string;
      model?: string;
      provider?: string;
      usage?: Usage;
      timestamp?: number;
    }
  | { role: 'toolResult'; toolCallId: string; toolName: string; content: Part[]; isError?: boolean; details?: unknown; timestamp?: number }
  | { role: 'bashExecution'; command: string; output: string; exitCode?: number; timestamp?: number }
  | { role: string; [key: string]: unknown };

export type Harness = { source?: string; title?: string; synopsis?: string; severity?: string; delivery?: string };

export type Entry =
  | { type: 'message'; id: string; timestamp?: string; message: Message }
  | { type: 'custom_message'; id: string; timestamp?: string; customType: string; content: string | Part[]; display?: boolean; details?: Record<string, unknown> }
  | { type: 'custom'; id: string; timestamp?: string; customType: string; data?: Record<string, unknown> }
  | { type: 'compaction'; id: string; timestamp?: string; summary?: string; tokensBefore?: number }
  | { type: 'model_change'; id: string; timestamp?: string; provider?: string; modelId?: string }
  | { type: string; id: string; timestamp?: string; [key: string]: unknown };

/** A tool call joined with its result (or live progress). */
export type ToolRun = {
  id: string;
  name: string;
  args: Record<string, unknown>;
  /** Raw args JSON while the model is still writing it. */
  argsText?: string;
  state: 'preparing' | 'running' | 'ok' | 'error' | 'stopped';
  output?: string;
  /** Live tail while running. */
  live?: string;
  details?: unknown;
};

export type ProcessItem =
  | { kind: 'thinking'; key: string; text: string; live: boolean }
  | { kind: 'tool'; key: string; tool: ToolRun }
  | { kind: 'step'; key: string; entry: Entry };

/** Rows of the transcript. */
export type Row =
  | {
      kind: 'user';
      key: string;
      text: string;
      images: ImageRef[];
      harness?: Harness;
      pending?: boolean;
      waiting?: boolean;
      /** UNI-251: the text is a typed `/command …` (its echo). `name` always
       *  chips at the start; `tag`/`body` = the harness turn it opened (the
       *  injected text, minus the user's own words), a collapsed chip under
       *  the bubble. `entryId` = the session entry to fork/start from (the
       *  harness user message), absent for a bare echo. */
      command?: { name: string; tag?: string; body?: string; entryId?: string };
    }
  | { kind: 'process'; key: string; items: ProcessItem[]; live: boolean }
  | { kind: 'prose'; key: string; text: string; live: boolean; stopped?: boolean }
  | { kind: 'error'; key: string; text: string }
  | { kind: 'card'; key: string; entry: Entry }
  | { kind: 'compaction'; key: string; entry: Entry }
  | { kind: 'turn_end'; key: string; ms?: number; cost?: number; stopped?: boolean; files?: TurnFile[] };


/** One `{oldText, newText}` pair that touched a `TurnFile`, kept separate
 * (UNI-192) so the card can expand/review each edit on its own — a `write`
 * call becomes one all-additions "edit". `realLine`, when known, is this
 * edit's first changed line *in the real file* (from the edit tool
 * result's `details.firstChangedLine`, or 1 for a `write`) — not to be
 * confused with the snippet-relative line a diff of just `oldText`/`newText`
 * would give (UNI-297: that printed a wrong "L1" on every edit row). Only
 * trustworthy when the call made exactly one edit to the file; omitted
 * otherwise rather than showing a guess. */
export type TurnFileEdit = { add: number; del: number; oldText: string; newText: string; tool: ToolRun; realLine?: number };

/** One file edited/written during a turn (for the "Changes this turn" card):
 * several edit/write calls against the same path merge into one `TurnFile`
 * with several `edits` (UNI-192). `tool` is the first call that touched it
 * (kept for "Open in editor" targeting). */
export type TurnFile = { path: string; add: number; del: number; tool: ToolRun; edits: TurnFileEdit[] };

/** The line "Open in editor" and Review jump to: the edit's real file
 * line when known (`realLine`, UNI-297), else 1 — safer than a line
 * number computed from the edit's own snippet (right for the snippet,
 * wrong for the file it's actually jumping into). */
export function turnFileEditLine(edit: TurnFileEdit): number {
  return edit.realLine ?? 1;
}

export function textOf(content: string | Part[] | undefined): string {
  if (typeof content === 'string') return content;
  if (!Array.isArray(content)) return '';
  return content
    .filter((p): p is { type: 'text'; text: string } => p.type === 'text' && typeof (p as { text?: unknown }).text === 'string')
    .map((p) => p.text)
    .join('');
}

/** Every image part of a message's content, phone-ready (see `ImageRef`). */
export function imagesOf(content: string | Part[] | undefined): ImageRef[] {
  if (!Array.isArray(content)) return [];
  return content
    .filter((p): p is Extract<Part, { type: 'image' }> => p.type === 'image')
    .map((p) => ({
      // Oino session entries carry pi's `mimeType`; the v1 bridge said `mime`.
      mime: typeof p.mime === 'string' ? p.mime : typeof p.mimeType === 'string' ? p.mimeType : 'image/*',
      data: typeof p.data === 'string' ? p.data : undefined,
      mediaRef: typeof p.mediaRef === 'string' ? p.mediaRef : undefined,
      bytes: typeof p.bytes === 'number' ? p.bytes : undefined,
    }));
}

/** Live assistant message being streamed. */
export type Streaming = { id: string; parts: Part[] };

/** Live tool runs keyed by call id (from tool_start/update/end). */
export type LiveTools = Record<string, { name: string; args: unknown; text?: string; done?: boolean; isError?: boolean }>;

/** Custom entries that render as their own row (everything else custom is dropped). */
export const STEP_TYPES = new Set(['subagent-step', 'sidekick-step']);
const CARD_ENTRY_TYPES = new Set([
  'unipi-command-echo',
  'unipi-command-output',
  'unipi-memory-session-card',
  'unipi-memory-save-card',
  'unipi-compaction',
  'unipi:kanboard-badge',
  'unipi:kanboard-notice',
  'unipi-progress',
  'unipi-cleared-input',
  'long-horizon-decision',
  'unipi:attachments',
]);

const toolState = (result: Extract<Message, { role: 'toolResult' }> | undefined, live: LiveTools[string] | undefined, stopped: boolean): ToolRun['state'] => {
  if (result) return result.isError ? 'error' : 'ok';
  if (live?.done) return live.isError ? 'error' : 'ok';
  if (live) return 'running';
  return stopped ? 'stopped' : 'running';
};

function parseArgs(args: unknown): Record<string, unknown> {
  if (args && typeof args === 'object' && !Array.isArray(args)) return args as Record<string, unknown>;
  if (typeof args === 'string') {
    try {
      const v = JSON.parse(args);
      if (v && typeof v === 'object') return v as Record<string, unknown>;
    } catch {
      // partial JSON while streaming
    }
  }
  return {};
}

/**
 * Builds the transcript rows. `entries` is the active branch (oldest first);
 * `streaming` the assistant message in flight; `running` whether pi works.
 */
export function buildRows(entries: readonly Entry[], streaming: Streaming | undefined, live: LiveTools, running: boolean, seams?: ReadonlySet<string>): Row[] {
  const rows: Row[] = [];
  const results = new Map<string, Extract<Message, { role: 'toolResult' }>>();
  for (const e of entries) {
    if (e.type === 'message' && (e as { message: Message }).message.role === 'toolResult') {
      const r = (e as { message: Extract<Message, { role: 'toolResult' }> }).message;
      results.set(r.toolCallId, r);
    }
  }
  let process: Extract<Row, { kind: 'process' }> | undefined;
  let turnStart: number | undefined;
  let turnCost = 0;
  let inTurn = false;
  let turnFiles: TurnFile[] = [];
  const flushProcess = () => {
    process = undefined;
  };
  const ensureProcess = (key: string) => {
    if (!process) {
      process = { kind: 'process', key: `p-${key}`, items: [], live: false };
      rows.push(process);
    }
    return process;
  };
  const recordEdit = (tool: ToolRun) => {
    if (toolCategory(tool.name) !== 'edit' || tool.state === 'error') return;
    const stat = diffStat(tool);
    if (!stat) return;
    const path = toolSubject(tool);
    if (!path) return;
    const pairs = toolFileEdits(tool);
    if (!pairs.length) return;
    const existing = turnFiles.find((f) => f.path === path);
    if (existing) {
      existing.add += stat.add;
      existing.del += stat.del;
      existing.edits.push(...pairs);
    } else {
      turnFiles.push({ path, add: stat.add, del: stat.del, tool, edits: pairs });
    }
  };
  // A turn may close with several back-to-back turn_end rows (continuations);
  // dedupeTurnEnds keeps only the last one, so every row gets a snapshot of
  // every file touched so far this turn — never drained mid-turn, only reset
  // when the next user message starts a new one.
  const filesSnapshot = (): TurnFile[] | undefined => (turnFiles.length ? turnFiles.slice() : undefined);
  const endTurn = (key: string, at: number | undefined, stopped: boolean) => {
    if (!inTurn) return;
    rows.push({ kind: 'turn_end', key: `end-${key}`, ms: turnStart && at ? Math.max(0, at - turnStart) : undefined, cost: turnCost || undefined, stopped, files: filesSnapshot() });
    inTurn = false;
    turnCost = 0;
    turnFiles = [];
  };

  const assistantParts = (key: string, parts: Part[], isLive: boolean, stopped: boolean) => {
    parts.forEach((part, i) => {
      const k = `${key}-${i}`;
      if (part.type === 'thinking') {
        const text = String((part as { thinking?: unknown }).thinking ?? '');
        if (!text.trim() && !isLive) return;
        ensureProcess(k).items.push({ kind: 'thinking', key: k, text, live: isLive && i === parts.length - 1 });
      } else if (part.type === 'toolCall') {
        const call = part as { id: string; name: string; arguments: unknown; partialJson?: string };
        const result = results.get(call.id);
        const l = live[call.id];
        const tool: ToolRun = {
          id: call.id,
          name: call.name,
          args: parseArgs(call.arguments),
          state: isLive && !l && !result ? 'preparing' : toolState(result, l, stopped || !running),
          output: result ? textOf(result.content) : undefined,
          live: l?.text,
          details: result?.details,
        };
        ensureProcess(k).items.push({ kind: 'tool', key: k, tool });
        recordEdit(tool);
      } else if (part.type === 'text') {
        const text = String((part as { text?: unknown }).text ?? '');
        if (!text.trim()) return;
        flushProcess();
        rows.push({ kind: 'prose', key: k, text, live: isLive, stopped: stopped && i === parts.length - 1 });
      }
    });
  };

  for (const e of entries) {
    // A paged-in older chunk ends here: never grow a newer row group backwards
    // (its key, and so the reader's scroll anchor, must stay the same).
    if (seams?.has(e.id)) flushProcess();
    if (e.type === 'message') {
      const m = (e as { message: Message }).message;
      const at = typeof m.timestamp === 'number' ? m.timestamp : e.timestamp ? Date.parse(e.timestamp) : undefined;
      if (m.role === 'user') {
        endTurn(e.id, at, false);
        flushProcess();
        const um = m as Extract<Message, { role: 'user' }>;
        if (isHiddenCommandText(textOf(um.content))) continue;
        rows.push({ kind: 'user', key: e.id, text: textOf(um.content), images: imagesOf(um.content), harness: um.unipiHarness });
        inTurn = true;
        turnStart = at;
      } else if (m.role === 'assistant') {
        const am = m as Extract<Message, { role: 'assistant' }>;
        inTurn = true;
        turnCost += am.usage?.cost?.total ?? 0;
        // A user stop can surface as stopReason "aborted" or as an error "…aborted".
        const stopped = am.stopReason === 'aborted' || (am.stopReason === 'error' && /\baborted\b/i.test(am.errorMessage ?? ''));
        assistantParts(e.id, am.content ?? [], false, stopped);
        if (am.stopReason === 'error' && am.errorMessage && !stopped) {
          flushProcess();
          rows.push({ kind: 'error', key: `${e.id}-err`, text: am.errorMessage });
        }
        if (am.stopReason === 'stop' || am.stopReason === 'aborted' || am.stopReason === 'error' || am.stopReason === 'length') {
          // The turn may continue (follow-ups) — close it only when nothing follows; handled at the end / next user.
          // Snapshot every file touched so far this turn (continuations keep
          // accumulating; dedupeTurnEnds drops every turn_end but the last one
          // before the next user message, which then carries the full list).
          (rows as Array<Row & { _stop?: number }>).push({ kind: 'turn_end', key: `end-${e.id}`, ms: turnStart && at ? Math.max(0, at - turnStart) : undefined, cost: turnCost || undefined, stopped, files: filesSnapshot() });
          inTurn = false;
          turnCost = 0;
          flushProcess();
        }
      } else if (m.role === 'bashExecution') {
        const b = m as Extract<Message, { role: 'bashExecution' }>;
        flushProcess();
        rows.push({ kind: 'card', key: e.id, entry: { type: 'custom', id: e.id, customType: 'user-bash', data: { command: b.command, output: b.output, exitCode: b.exitCode } } });
      }
      // toolResult: joined into its call above.
    } else if (e.type === 'custom_message') {
      flushProcess();
      rows.push({ kind: 'card', key: e.id, entry: e });
    } else if (e.type === 'custom') {
      const ct = (e as { customType: string }).customType;
      if (ct === 'unipi-command-echo' && isHiddenCommandText(entryData(e).text)) continue;
      if (STEP_TYPES.has(ct)) ensureProcess(e.id).items.push({ kind: 'step', key: e.id, entry: e });
      else if (CARD_ENTRY_TYPES.has(ct)) {
        flushProcess();
        rows.push({ kind: 'card', key: e.id, entry: e });
      }
    } else if (e.type === 'compaction') {
      flushProcess();
      rows.push({ kind: 'compaction', key: e.id, entry: e });
    } else if (e.type === 'model_change') {
      flushProcess();
      rows.push({ kind: 'card', key: e.id, entry: e });
    }
  }
  if (streaming) {
    inTurn = true;
    assistantParts(streaming.id, streaming.parts, true, false);
  }
  // Live tools without a streamed call part (joined late / reconnect).
  const shown = new Set<string>();
  for (const r of rows) if (r.kind === 'process') for (const it of r.items) if (it.kind === 'tool') shown.add(it.tool.id);
  for (const [id, t] of Object.entries(live)) {
    if (shown.has(id) || t.done) continue;
    ensureProcess(id).items.push({ kind: 'tool', key: `live-${id}`, tool: { id, name: t.name, args: parseArgs(t.args), state: 'running', live: t.text } });
  }
  // Mark the trailing process group live while pi works.
  if (running) {
    const last = rows[rows.length - 1];
    if (last?.kind === 'process') last.live = true;
  }
  return collapseCommandRows(dedupeTurnEnds(rows));
}

function customTypeOf(entry: Entry): string | undefined {
  return entry.type === 'custom' || entry.type === 'custom_message' ? (entry as { customType: string }).customType : undefined;
}

function entryData(entry: Entry): Record<string, unknown> {
  return ((entry as { data?: Record<string, unknown> }).data ?? {}) as Record<string, unknown>;
}

/** Internal commands the app bridge runs itself (UNI-251): never shown,
 * not even from sessions recorded before pi stopped echoing them. */
export const HIDDEN_COMMANDS: ReadonlySet<string> = new Set(['unipi-app-session']);

export function isHiddenCommandText(text: unknown): boolean {
  if (typeof text !== 'string') return false;
  const m = /^\/(\S+)/.exec(text.trim());
  return !!m && HIDDEN_COMMANDS.has(m[1]!);
}

/** `Kanboard · Task budget` → `Kanboard · task budget` (sentence case). */
function harnessTag(source: string | undefined, title: string | undefined): string | undefined {
  const t = title ? title.charAt(0).toLowerCase() + title.slice(1) : undefined;
  if (source && t) return `${source} · ${t}`;
  return source || title || undefined;
}

/** The injected part of a harness message: its text minus the user's own
 * words when it merely repeats them (`…\n\nRequest: <args>` for kanboard,
 * the whole text for long-horizon). Empty → nothing to expand. */
export function injectedBody(body: string, args: string): string {
  let b = body.trim();
  const a = args.trim();
  if (!a) return b;
  if (b === a) return '';
  if (b.endsWith(a)) {
    b = b.slice(0, b.length - a.length).trimEnd();
    // …and the label that introduced them (`Request:`, `Text to analyze:`).
    b = b.replace(/(?:^|\n)[^\n]{0,40}:\s*$/, '').trimEnd();
  }
  return b;
}

/**
 * UNI-210 / UNI-251: what the user typed as a `/command …` shows as THEIR
 * message — a normal user bubble with the command as a chip at the start —
 * never as a monospace echo line. `withCommandEcho` appends the echo entry;
 * a command that opens a harness turn then sends the injected prompt as a
 * harness-tagged user message (`sendHarnessUserMessage`), and long-horizon
 * mode switches add a `long-horizon-decision` badge. An echo immediately
 * followed by either or both folds them into the bubble: the harness
 * source·title (or the mode) becomes a small chip under it and the injected
 * text (minus the user's words) expands inline from that chip. Order-agnostic
 * between badge/harness but strictly adjacent — anything else in between (a
 * tool call, an unrelated card) breaks the run and the rows show separately.
 * A bare echo (a command with no harness turn) is still the user's bubble. */
export function collapseCommandRows(rows: Row[]): Row[] {
  const out: Row[] = [];
  for (let i = 0; i < rows.length; i++) {
    const r = rows[i]!;
    if (r.kind === 'card' && customTypeOf(r.entry) === 'unipi-command-echo') {
      const text = String(entryData(r.entry).text ?? '').trim();
      if (!text) continue;
      if (isHiddenCommandText(text)) continue;
      const head = /^\/(\S+)\s*/.exec(text);
      const name = head ? head[1]! : '';
      const args = head ? text.slice(head[0].length) : text;
      let j = i + 1;
      let mode: string | undefined;
      let harness: Harness | undefined;
      let harnessRow: Extract<Row, { kind: 'user' }> | undefined;
      for (let k = 0; k < 2 && j < rows.length; k++) {
        const next = rows[j]!;
        if (next.kind === 'card' && customTypeOf(next.entry) === 'long-horizon-decision') {
          const data = entryData(next.entry) as { mode?: string };
          if (data.mode && data.mode !== 'none') mode = data.mode;
          j++;
          continue;
        }
        if (next.kind === 'user' && next.harness && !harness) {
          harness = next.harness;
          harnessRow = next;
          j++;
          continue;
        }
        break;
      }
      const tag = harness ? harnessTag(harness.source, harness.title) : mode ? harnessTag('Long-horizon', mode) : undefined;
      const body = harnessRow ? injectedBody(harnessRow.text, args) || undefined : undefined;
      out.push({
        kind: 'user',
        key: harnessRow?.key ?? r.key,
        text,
        images: harnessRow?.images ?? [],
        command: { name, tag, body, entryId: harnessRow?.key },
      });
      i = j - 1;
      continue;
    }
    out.push(r);
  }
  return out;
}

/** Only the last turn_end before the next user message survives (a turn's steps each end with stop). */
function dedupeTurnEnds(rows: Row[]): Row[] {
  const out: Row[] = [];
  for (let i = 0; i < rows.length; i++) {
    const r = rows[i]!;
    if (r.kind === 'turn_end') {
      const next = rows.slice(i + 1).find((x) => x.kind !== 'card');
      if (next && next.kind !== 'user') continue;
    }
    out.push(r);
  }
  return out;
}

// ── process group summaries ───────────────────────────────────────────

export type ToolCategory = 'read' | 'search' | 'edit' | 'run' | 'web' | 'ask' | 'agent' | 'memory' | 'board' | 'other';

export function toolCategory(name: string): ToolCategory {
  const n = name.toLowerCase();
  if (n === 'read' || n.endsWith('_read') || n === 'ls' || n === 'find' || n === 'fffind' || n === 'session_recall') return n === 'find' || n === 'fffind' ? 'search' : 'read';
  if (n === 'grep' || n === 'ffgrep' || n.includes('search') && !n.includes('web') && !n.includes('memory')) return 'search';
  if (n === 'edit' || n === 'write' || n.includes('edit') || n.includes('write')) return 'edit';
  if (n === 'bash' || n.startsWith('bg_') || n === 'shell') return 'run';
  if (n.startsWith('web') || n.includes('fetch') || n.includes('google') || n.includes('content_read')) return 'web';
  if (n === 'ask_user') return 'ask';
  if (n.includes('subagent') || n.includes('agent')) return 'agent';
  if (n.startsWith('memory') || n.includes('memory')) return 'memory';
  if (n.includes('kanboard') || n.includes('todo') || n.includes('goal')) return 'board';
  return 'other';
}

const CATEGORY_DONE: Record<ToolCategory, string> = {
  read: 'Read files',
  search: 'Searched',
  edit: 'Edited files',
  run: 'Ran commands',
  web: 'Searched the web',
  ask: 'Asked you',
  agent: 'Ran agents',
  memory: 'Used memory',
  board: 'Updated tasks',
  other: 'Used tools',
};
const CATEGORY_LIVE: Record<ToolCategory, string> = {
  read: 'Reading',
  search: 'Searching',
  edit: 'Editing',
  run: 'Running',
  web: 'Searching the web',
  ask: 'Waiting for you',
  agent: 'Running an agent',
  memory: 'Checking memory',
  board: 'Updating tasks',
  other: 'Working',
};

/** Short subject of a tool call: path, command, query… */
export function toolSubject(tool: Pick<ToolRun, 'name' | 'args'>, cwd?: string): string {
  const a = tool.args;
  const pick = (...keys: string[]) => {
    for (const k of keys) {
      const v = a[k];
      if (typeof v === 'string' && v.trim()) return v.trim();
    }
    return '';
  };
  let s = pick('description', 'path', 'file_path', 'command', 'pattern', 'query', 'url', 'title', 'prompt', 'task', 'name');
  if (!s && Array.isArray(a.questions)) s = String((a.questions[0] as { question?: string })?.question ?? '');
  if (!s && Array.isArray(a.url)) s = String(a.url[0] ?? '');
  s = s.split('\n')[0]!;
  if (cwd && s.startsWith(cwd + '/')) s = s.slice(cwd.length + 1);
  return s.replace(/^\/home\/[^/]+\//, '~/');
}

/**
 * What pi is doing right now, for the running row (UNI-146). "Thinking" only
 * while the model streams thinking; a running tool names itself.
 */
export function liveActivity(
  st: { dialogs: readonly unknown[]; streaming?: Streaming; tools: LiveTools; run: { running: boolean } },
  cwd?: string,
): { title: string; detail?: string } {
  if (st.dialogs.length) return { title: 'Waiting for you' };
  const running = Object.entries(st.tools).filter(([, t]) => !t.done);
  if (running.length) {
    const [id, t] = running[running.length - 1]!;
    const subject = toolSubject({ name: t.name, args: parseArgs(t.args) }, cwd);
    const more = running.length > 1 ? ` +${running.length - 1}` : '';
    return { title: CATEGORY_LIVE[toolCategory(t.name)] ?? 'Working', detail: (subject || t.name || id) + more };
  }
  const parts = st.streaming?.parts ?? [];
  const last = parts[parts.length - 1];
  if (st.streaming) {
    if (!last) return { title: 'Waiting for the model' };
    if (last.type === 'toolCall') {
      const name = (last as { name?: string }).name;
      return { title: 'Preparing', detail: name || 'a tool call' };
    }
    if (last.type === 'thinking') return { title: 'Thinking' };
    if (last.type === 'text') return { title: 'Writing' };
  }
  return { title: 'Working' };
}

/** Folded header of a process group: live activity or a summary. */
export function processSummary(items: readonly ProcessItem[], live: boolean, cwd?: string): { title: string; detail?: string } {
  const tools = items.filter((i): i is Extract<ProcessItem, { kind: 'tool' }> => i.kind === 'tool');
  const steps = items.filter((i) => i.kind === 'step').length;
  if (live) {
    const last = items[items.length - 1];
    if (last?.kind === 'tool') return { title: CATEGORY_LIVE[toolCategory(last.tool.name)], detail: toolSubject(last.tool, cwd) || last.tool.name };
    if (last?.kind === 'step') return { title: 'Delegating', detail: String((last.entry as { data?: { name?: string } }).data?.name ?? '') };
    return { title: last?.kind === 'thinking' && last.live ? 'Thinking' : 'Working' };
  }
  if (tools.length === 0) return steps ? { title: `${steps} delegated ${steps === 1 ? 'step' : 'steps'}` } : { title: 'Thought' };
  const order: ToolCategory[] = [];
  const count = new Map<ToolCategory, number>();
  for (const t of tools) {
    const c = toolCategory(t.tool.name);
    if (!count.has(c)) order.push(c);
    count.set(c, (count.get(c) ?? 0) + 1);
  }
  const ranked = [...order].sort((a, b) => count.get(b)! - count.get(a)!);
  const words = ranked.slice(0, 3).map((c, i) => (i === 0 ? CATEGORY_DONE[c] : CATEGORY_DONE[c].toLowerCase()));
  const errors = tools.filter((t) => t.tool.state === 'error').length;
  return {
    title: words.join(', ') + (ranked.length > 3 ? ', …' : ''),
    detail: `${tools.length} ${tools.length === 1 ? 'step' : 'steps'}${errors ? ` · ${errors} failed` : ''}`,
  };
}

/** Thinking preview: the first line of the latest finished paragraph. */
export function thinkingPreview(text: string, live: boolean): string {
  const paras = text.split(/\n\s*\n/).map((p) => p.trim()).filter(Boolean);
  if (!paras.length) return '';
  const done = live && paras.length > 1 ? paras[paras.length - 2]! : paras[paras.length - 1]!;
  return done.split('\n')[0]!.replace(/\*\*/g, '').slice(0, 160);
}

/** The `{oldText, newText}` pairs a tool call's args carry (`edits[]`, a
 * single `oldText`/`newText` pair, or a `write`'s whole content as one
 * all-additions pair) — the per-edit rows of the "Changes this turn" card
 * (UNI-192). `realLine` (UNI-297) is set from the pi edit tool's result
 * `details.firstChangedLine` — a real file line — but only when the call
 * made exactly one edit: with several `edits[]` that single number can't
 * be attributed to any one of them, so it's left unset (the row then shows
 * no line rather than a wrong one). A `write` starts at line 1 of the new
 * file. */
export function toolFileEdits(tool: Pick<ToolRun, 'name' | 'args'> & { details?: unknown }): Array<{ oldText: string; newText: string; add: number; del: number; tool: ToolRun; realLine?: number }> {
  const a = tool.args;
  const lines = (s: unknown) => (typeof s === 'string' && s.length ? s.split('\n').length : 0);
  const full = tool as ToolRun;
  const resultFirstLine = (tool.details as { firstChangedLine?: unknown } | null)?.firstChangedLine;
  const singleEditRealLine = typeof resultFirstLine === 'number' ? resultFirstLine : undefined;
  if (Array.isArray(a.edits)) {
    const edits = a.edits as Array<{ oldText?: string; newText?: string }>;
    return edits.map((e) => ({
      oldText: e.oldText ?? '',
      newText: e.newText ?? '',
      add: lines(e.newText),
      del: lines(e.oldText),
      tool: full,
      realLine: edits.length === 1 ? singleEditRealLine : undefined,
    }));
  }
  if (typeof a.oldText === 'string' || typeof a.old_string === 'string') {
    const oldText = String(a.oldText ?? a.old_string ?? '');
    const newText = String(a.newText ?? a.new_string ?? '');
    return [{ oldText, newText, add: lines(newText), del: lines(oldText), tool: full, realLine: singleEditRealLine }];
  }
  if (tool.name === 'write' && typeof a.content === 'string') {
    return [{ oldText: '', newText: a.content, add: lines(a.content), del: 0, tool: full, realLine: 1 }];
  }
  return [];
}

/** `+N −M` for an edit/write call, when its args say. */
export function diffStat(tool: Pick<ToolRun, 'name' | 'args'>): { add: number; del: number } | undefined {
  const a = tool.args;
  const lines = (s: unknown) => (typeof s === 'string' && s.length ? s.split('\n').length : 0);
  if (Array.isArray(a.edits)) {
    let add = 0;
    let del = 0;
    for (const e of a.edits as Array<{ oldText?: string; newText?: string }>) {
      add += lines(e.newText);
      del += lines(e.oldText);
    }
    return { add, del };
  }
  if (typeof a.oldText === 'string' || typeof a.old_string === 'string') return { add: lines(a.newText ?? a.new_string), del: lines(a.oldText ?? a.old_string) };
  if (tool.name === 'write' && typeof a.content === 'string') return { add: lines(a.content), del: 0 };
  return undefined;
}

export function formatDuration(ms: number): string {
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${s % 60}s`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

/**
 * UNI-275: the transcript row that shows session entry `id` (search →
 * "navigate to"). Rows are keyed by entry id (`id`), assistant parts
 * (`id-<i>`), process groups (`p-id-<i>`, holding items keyed `id-<i>`),
 * turn ends / errors (`end-id`, `id-err`). A tool result has no row of its
 * own: it shows in the process group of the call it answers (`callOwner`
 * maps a toolResult entry id to the assistant entry that made the call).
 */
export function rowIndexForEntry(rows: readonly Row[], id: string, callOwner?: ReadonlyMap<string, string>): number {
  const ids = [id, ...(callOwner?.get(id) ? [callOwner.get(id)!] : [])];
  for (const want of ids) {
    const prefix = `${want}-`;
    const i = rows.findIndex((r) => {
      if (r.key === want || r.key.startsWith(prefix)) return true;
      if (r.kind === 'user' && r.command?.entryId === want) return true;
      if (r.kind === 'process') return r.items.some((it) => it.key === want || it.key.startsWith(prefix));
      return false;
    });
    if (i >= 0) return i;
  }
  return -1;
}

/** toolResult entry id → id of the assistant entry whose tool call it answers. */
export function toolResultOwners(entries: readonly Entry[]): Map<string, string> {
  const callIn = new Map<string, string>();
  const out = new Map<string, string>();
  for (const e of entries) {
    const m = (e as { message?: { role?: string; content?: unknown; toolCallId?: string } }).message;
    if (e.type !== 'message' || !m) continue;
    if (m.role === 'assistant' && Array.isArray(m.content)) {
      for (const p of m.content as Array<{ type?: string; id?: string }>) if (p.type === 'toolCall' && p.id) callIn.set(p.id, e.id);
    } else if (m.role === 'toolResult' && m.toolCallId && callIn.has(m.toolCallId)) out.set(e.id, callIn.get(m.toolCallId)!);
  }
  return out;
}
