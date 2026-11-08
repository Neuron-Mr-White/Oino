/**
 * Chat event kinds ([chat-events], owner report 2026-11-03: "memory and other
 * events can't expand and don't render differently"). One pure describer for
 * every non-prose session entry that reaches a chat: extension
 * `custom_message`s and `custom` entries, user `!bash` runs, mid-chat system
 * messages and branch summaries. Each known kind gets its own icon, a
 * "Source · one-line summary" head, an optional meta and tone, and the full
 * body the row expands to (markdown, plain text or pretty JSON — never raw
 * JSON as prose). The parity table is docs/dev/v2/qa/chat-events-parity.md.
 *
 * `describeEvent` answers:
 *   - an `EventView` → the app's `EventCard` row (CardRow);
 *   - `null`         → never shown (state-only entries, empty continues);
 *   - `undefined`    → not an event row (model/thinking change, workspace
 *                      move, unknown `custom` state → extension renderer only).
 */
import { formatDuration, textOf, type Part } from '../chat/model';
import type { SessionEntry } from '../session/types';

export type EventIcon =
  | 'memory'
  | 'memorySave'
  | 'memoryWake'
  | 'agent'
  | 'sidekick'
  | 'bgDone'
  | 'bgFailed'
  | 'bgRunning'
  | 'shell'
  | 'compactor'
  | 'kanboard'
  | 'mode'
  | 'ralph'
  | 'goal'
  | 'skills'
  | 'plan'
  | 'watchdog'
  | 'dream'
  | 'btw'
  | 'browser'
  | 'attach'
  | 'system'
  | 'branch'
  | 'response'
  | 'command'
  | 'generic';

export type EventTone = 'ok' | 'warning' | 'error';
export type EventAudience = 'both' | 'agent' | 'user';
export type EventBody = { format: 'markdown' | 'text' | 'json'; text: string };

/**
 * The audience badge on an event row (owner 2026-11-06): who sees it — the
 * user only, the agent only, or both.
 */
export const AUDIENCE_LABEL: Record<EventAudience, string> = {
  user: 'Visible to you only',
  agent: 'Hidden from you',
  both: 'You & agent',
};

export const AUDIENCE_TITLE: Record<EventAudience, string> = {
  user: 'Visible to you only — the agent never sees it',
  agent: 'Hidden from you — sent to the agent only',
  both: 'Visible to you and to the agent',
};

export type EventView = {
  /** Stable kind id (`data-event` on the row; tests and QA key on it). */
  kind: string;
  icon: EventIcon;
  source: string;
  /** The collapsed one-liner after the source. */
  title: string;
  meta?: string;
  tone?: EventTone;
  /** Who receives this entry in chat/context: user only, agent only, or both. */
  audience?: EventAudience;
  /** What the row expands to (absent: the row is a plain marker). */
  body?: EventBody;
  /**
   * The owning extension draws the expanded body (`_oino/render`,
   * `expanded: true`) — images, boards, attachment chips; `body` shows while
   * the answer is in flight or when the extension answers `null`.
   */
  renderer?: 'customMessage' | 'customEntry';
};

/** Body text above this is clipped (the session file keeps everything; perf: no unbounded DOM). */
export const EVENT_BODY_MAX = 60_000;

/** `custom` state entries and `custom_message`s that never render a row. */
const HIDDEN = new Set([
  'oino.tool-dispatch',
  'oino.command-echo',
  'unipi-command-echo',
  'unipi:auto-name',
  'unipi:skills-judged',
  'unipi:skills-revealed',
  'unipi:plan-mode',
  'unipi:permission-mode',
  'unipi-cleared-input',
  'unipi-child-usage',
  'unipi.file-summary/turn',
  'unipi.file-summary/undo',
  'unipi:bg-tasks',
  'unipi:fusion-sidekick',
  'unipi:compactor-pending',
  'compactor-origin',
  'compactor-auto-continue',
  'unipi-continue',
  'watchdog.session-caps',
  'codemode-store',
]);

/** Kinds shown even when their `custom_message` says `display: false` (old UniPi showed every harness message; the owner wants them). */
const SHOW_WHEN_HIDDEN = new Set([
  'unipi-memory-recall-reminder',
  'unipi-memory-retro-reminder',
  'unipi-ralph-loop-reminder',
  'unipi-compactor-resume',
  'unipi-response',
  'unipi-skills-revealed',
  'unipi:plan-mode-message',
  'unipi-watchdog',
  'unipi:lh-continue',
  'unipi:kanboard-continue',
  'unipi:kanboard-next',
  'unipi:kanboard-reminder',
  'compactor-recall',
]);

const str = (v: unknown): string | undefined => (typeof v === 'string' && v.trim() ? v.trim() : undefined);
const num = (v: unknown): number | undefined => (typeof v === 'number' && Number.isFinite(v) ? v : undefined);
const rec = (v: unknown): Record<string, unknown> => (v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : {});
const join = (...parts: Array<string | undefined | false>): string | undefined => parts.filter(Boolean).join(' · ') || undefined;
const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;
const clip = (s: string): string => (s.length > EVENT_BODY_MAX ? `${s.slice(0, EVENT_BODY_MAX)}\n…` : s);
const md = (text: string | undefined): EventBody | undefined => (text && text.trim() ? { format: 'markdown', text: clip(text.trim()) } : undefined);
const plain = (text: string | undefined): EventBody | undefined => (text && text.trim() ? { format: 'text', text: clip(text.replace(/\s+$/, '')) } : undefined);
const json = (v: unknown): EventBody | undefined => {
  if (v === undefined || v === null) return undefined;
  if (typeof v === 'object' && !Array.isArray(v) && Object.keys(v as object).length === 0) return undefined;
  try {
    return { format: 'json', text: clip(JSON.stringify(v, null, 2)) };
  } catch {
    return undefined;
  }
};
const money = (n?: number) => (n !== undefined && n >= 0.0005 ? `$${n.toFixed(n < 0.1 ? 3 : 2)}` : undefined);
const dur = (ms?: number) => (ms !== undefined && ms >= 1000 ? formatDuration(ms) : undefined);
const kTokens = (n?: number) => (n === undefined ? '?' : n >= 1000 ? `${Math.round(n / 1000)}k` : String(n));

/** The first non-empty line without markdown heading / emphasis marks or emoji, ≤ 120 chars. */
export function firstLine(text: string | undefined): string {
  for (const raw of (text ?? '').split('\n')) {
    const l = raw
      .replace(/\*\*|__|`/g, '')
      .replace(/^\s*(#+|[-*>]|\d+\.)\s*/, '')
      .replace(/[\u{1F300}-\u{1FAFF}\u{2600}-\u{27BF}]\uFE0F?/gu, '')
      .trim();
    if (l && !/^<\/?[a-z_-]+[^>]*>$/i.test(l)) return l.length > 120 ? `${l.slice(0, 119)}…` : l;
  }
  return '';
}

/** `label · line`, or just `line` when it already starts with the label ("Next task: …"). */
const prefixed = (label: string, line: string): string => (!line ? label : line.toLowerCase().startsWith(label.toLowerCase()) ? line : `${label} · ${line}`);

const decode = (s: string) => s.replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&apos;/g, "'").replace(/&amp;/g, '&');

/** `<name>…</name>` of an XML-ish notification (first match, entity-decoded). */
export function xmlTag(text: string, name: string): string | undefined {
  const m = new RegExp(`<${name}(?:\\s[^>]*)?>([\\s\\S]*?)</${name}>`).exec(text);
  return m ? str(decode(m[1]!)) : undefined;
}

const xmlAttr = (text: string, tag: string, attr: string): string | undefined => {
  const m = new RegExp(`<${tag}\\b[^>]*\\b${attr}="([^"]*)"`).exec(text);
  return m ? str(decode(m[1]!)) : undefined;
};

/** The message text of a `custom_message` (top-level `content`, or the live `message.content`). */
export function contentText(e: SessionEntry): string {
  const raw = (e as { content?: unknown }).content ?? e.message?.content;
  return textOf(raw as string | Part[] | undefined);
}

const detailsOf = (e: SessionEntry): Record<string, unknown> => rec((e as { details?: unknown }).details ?? (e.message as { details?: unknown } | undefined)?.details);
const dataOf = (e: SessionEntry): Record<string, unknown> => rec(e.data);

/** `details.unipiHarness` / `oinoProvenance` meta (source, title, synopsis, severity). */
function harnessOf(e: SessionEntry): { source?: string; title?: string; synopsis?: string; warning: boolean } {
  const d = detailsOf(e);
  const m = (e.message ?? {}) as Record<string, unknown>;
  const h = rec(d.unipiHarness ?? d.oinoProvenance ?? m.unipiHarness ?? m.oinoProvenance);
  return { source: str(h.source), title: str(h.title), synopsis: str(h.synopsis), warning: h.severity === 'warning' };
}

const hiddenByDisplay = (e: SessionEntry): boolean =>
  (e as { display?: unknown }).display === false || (e.message as { display?: unknown } | undefined)?.display === false;

/** The context summarizer's system prompt (`oino-core` compaction prompts, pi's `compact.md`). */
const SUMMARIZER_PROMPT = /^you are a context summarization assistant\b/i;

/** A user message that is the summarizer request itself (a `compact` prompt template sent as a
 *  chat message) — compaction-internal text, rendered as a card instead of a bubble (OIN-19). */
export function isSummaryRequest(m: { role?: unknown; content?: unknown } | undefined): boolean {
  if (!m || m.role !== 'user') return false;
  return SUMMARIZER_PROMPT.test(textOf(m.content as string | Part[] | undefined).trimStart());
}

/** Determines whether an entry is visible to the user only, sent to the agent only, or shared with both. */
export function eventAudience(e: SessionEntry): EventAudience {
  if (e.type === 'compaction' || e.type === 'branch_summary') return 'both';
  if (e.type === 'custom') return 'user';
  if (e.type === 'message') {
    if (e.message?.role === 'system') return 'agent';
    if (e.message?.role === 'bashExecution') {
      return (e.message as Record<string, unknown>).excludeFromContext === true ? 'user' : 'both';
    }
    return 'both';
  }
  if (e.type === 'custom_message') {
    return hiddenByDisplay(e) ? 'agent' : 'both';
  }
  return 'both';
}

/**
 * Whether a `custom_message` / `custom` entry produces a transcript row at
 * all (turns.ts uses it before grouping; `display: false` hides only kinds
 * the app has no summary for).
 */
export function eventVisible(e: SessionEntry): boolean {
  if (e.type === 'message') return describeEvent(e) != null;
  const ct = String(e.customType ?? '');
  if (HIDDEN.has(ct)) return false;
  if (e.type === 'custom_message' && hiddenByDisplay(e) && !SHOW_WHEN_HIDDEN.has(ct)) return false;
  return describeEvent(e) !== null;
}

/** Icon per harness source for generic / `unipi-response` rows. */
function iconOfSource(source: string | undefined): EventIcon {
  switch ((source ?? '').toLowerCase()) {
    case 'memory':
      return 'memory';
    case 'long-horizon':
    case 'mode':
      return 'mode';
    case 'kanboard':
      return 'kanboard';
    case 'compactor':
      return 'compactor';
    case 'watchdog':
      return 'watchdog';
    case 'skills':
      return 'skills';
    case 'plan mode':
    case 'workflow':
      return 'plan';
    case 'subagents':
    case 'subagent':
      return 'agent';
    case 'fusion':
    case 'sidekick':
      return 'sidekick';
    case 'dream':
      return 'dream';
    case 'background tasks':
    case 'background task':
      return 'bgDone';
    default:
      return 'response';
  }
}

/** `<task-notification>` (old UniPi subagent fleet) → a Subagent row. */
function taskNotification(e: SessionEntry, text: string, kind: string): EventView {
  const d = detailsOf(e);
  const status = (xmlTag(text, 'status') ?? str(d.status) ?? 'completed').toLowerCase();
  const failed = /fail|error|cancel|kill|abort/.test(status);
  const summary = xmlTag(text, 'summary') ?? str(d.description) ?? 'Subagent finished';
  const name = /"([^"]+)"/.exec(summary)?.[1];
  const result = xmlTag(text, 'result') ?? str(d.resultPreview);
  const usage = xmlTag(text, 'usage') ?? '';
  const tokens = Number(/<total_tokens>(\d+)/.exec(usage)?.[1] ?? d.totalTokens ?? NaN);
  const ms = Number(/<duration_ms>(\d+)/.exec(usage)?.[1] ?? d.durationMs ?? NaN);
  return {
    kind,
    icon: 'agent',
    source: 'Subagent',
    title: name ? `${name} · ${failed ? 'failed' : 'done'}` : summary,
    meta: join(Number.isFinite(ms) ? dur(ms) : undefined, Number.isFinite(tokens) && tokens > 0 ? `${kTokens(tokens)} tokens` : undefined),
    tone: failed ? 'error' : 'ok',
    body: md(result) ?? plain(text),
  };
}

function bgNotification(e: SessionEntry): EventView {
  const text = contentText(e);
  const d = detailsOf(e);
  const rawStatus = (xmlTag(text, 'status') ?? str(d.status) ?? 'completed').toLowerCase();
  const status = rawStatus === 'failed' || rawStatus === 'error' ? 'failed' : rawStatus === 'killed' || rawStatus === 'stopped' ? 'stopped' : rawStatus === 'running' ? 'running' : 'done';
  const exit = num(d.exitCode) ?? (xmlTag(text, 'exit-code') !== undefined && xmlTag(text, 'exit-code') !== 'null' ? Number(xmlTag(text, 'exit-code')) : undefined);
  const name = xmlTag(text, 'task-name') ?? str(d.name) ?? str(d.id) ?? 'Background task';
  const error = xmlTag(text, 'error') ?? str(d.error);
  const tail = Array.isArray(d.outputTail) ? (d.outputTail as unknown[]).filter((l): l is string => typeof l === 'string') : [];
  const command = str(d.command);
  const body = [command ? `$ ${command}` : '', ...tail, error && !tail.length ? error : ''].filter(Boolean).join('\n');
  const failed = status === 'failed' || (exit !== undefined && exit !== 0);
  return {
    kind: 'background-task',
    icon: status === 'running' ? 'bgRunning' : failed ? 'bgFailed' : 'bgDone',
    source: 'Background task',
    title: name,
    meta: join(status === 'running' ? 'running' : status === 'stopped' ? 'stopped' : exit !== undefined ? `exit ${exit}` : status === 'failed' ? 'failed' : 'done', dur(num(d.endTime) !== undefined && num(d.startTime) !== undefined ? num(d.endTime)! - num(d.startTime)! : undefined)),
    tone: status === 'running' ? undefined : status === 'stopped' ? 'warning' : failed ? 'error' : 'ok',
    body: plain(body) ?? plain(error),
  };
}

function memoryRecall(e: SessionEntry): EventView {
  const text = contentText(e);
  const count = Number(/You have (\d+) memor/.exec(text)?.[1] ?? NaN);
  const project = /for project "([^"]+)"/.exec(text)?.[1];
  const listAt = text.indexOf('Available memories:');
  const first = listAt >= 0 ? /^\s*-\s+(.+)$/m.exec(text.slice(listAt))?.[1]?.trim() : undefined;
  const wake = /wake-up/i.test(text);
  const title = Number.isFinite(count)
    ? count > 0
      ? join(`recalled ${count}`, first)!
      : `no memories yet${project ? ` for ${project}` : ''}`
    : firstLine(text) || 'recall';
  return { kind: 'memory-recall', icon: 'memory', source: 'Memory', title, meta: join(project && count > 0 ? project : undefined, wake ? 'wake-up' : undefined), body: md(text) };
}

function compaction(e: SessionEntry): EventView {
  const d = dataOf(e);
  const before = num(d.tokensBefore);
  const after = num(d.tokensAfter);
  const sections = Array.isArray(d.sections) ? (d.sections as unknown[]).filter((s): s is string => typeof s === 'string') : [];
  const lines = [
    ...sections.map((s) => `- ${s}`),
    num(d.keptTurns) !== undefined ? `\nKept ${num(d.keptTurns)} of ${num(d.totalTurns) ?? '?'} turns (${kTokens(num(d.keptTokens))} tokens)` : '',
    num(d.summaryTokens) !== undefined ? `Summary ≈ ${kTokens(num(d.summaryTokens))} tokens` : '',
    str(d.command) ? `Command: \`${str(d.command)}\`` : '',
  ].filter(Boolean);
  return {
    kind: 'compaction-card',
    icon: 'compactor',
    source: 'Compactor',
    title: after !== undefined ? `${kTokens(before)} → ${kTokens(after)} tokens` : `compacted ${kTokens(before)} tokens`,
    meta: join(str(d.method), str(d.trigger), num(d.percent) !== undefined ? `${Math.round(num(d.percent)!)}%` : undefined),
    body: md(lines.join('\n')),
  };
}

function kanboard(ct: string, e: SessionEntry): EventView {
  const titles: Record<string, string> = {
    'unipi:kanboard-continue': 'unfinished task',
    'unipi:kanboard-next': 'next task',
    'unipi:kanboard-show': 'board',
    'unipi:kanboard-help': 'help',
    'unipi:kanboard-doctor': 'doctor',
    'unipi:kanboard-reminder': 'reminder',
  };
  if (e.type === 'custom') {
    const d = dataOf(e);
    const what = str(d.text) ?? str(d.message) ?? str(d.kind) ?? str(d.title) ?? (ct.endsWith('badge') ? 'badge' : 'notice');
    return { kind: ct.endsWith('badge') ? 'kanboard-badge' : 'kanboard-notice', icon: 'kanboard', source: 'Kanboard', title: join(str(d.id), what)!, body: json(d), renderer: 'customEntry' };
  }
  const text = contentText(e);
  const h = harnessOf(e);
  return {
    kind: `kanboard-${ct.split('-').pop()}`,
    icon: 'kanboard',
    source: 'Kanboard',
    title: prefixed((h.title && h.title !== 'Kanboard' ? h.title.toLowerCase() : titles[ct]) ?? 'kanboard', firstLine(text)),
    body: md(text),
    ...(ct === 'unipi:kanboard-show' ? { renderer: 'customMessage' as const } : {}),
  };
}

/** Kinds keyed by `customType` (custom_message and custom entries). */
function describeCustom(e: SessionEntry): EventView | null | undefined {
  const ct = String(e.customType ?? '');
  if (HIDDEN.has(ct)) return null;
  if (ct === 'oino.cwd_change' || ct === 'subagent-step' || ct === 'sidekick-step') return undefined;
  const text = e.type === 'custom_message' ? contentText(e) : '';
  const d = dataOf(e);
  const det = detailsOf(e);
  switch (ct) {
    // ── memory ────────────────────────────────────────────────────────
    case 'unipi-memory-recall-reminder':
      return memoryRecall(e);
    case 'unipi-memory-retro-reminder':
      return { kind: 'memory-nudge', icon: 'memory', source: 'Memory', title: 'save nudge · store what this task taught', body: md(text) };
    case 'unipi-memory-session-card': {
      const count = num(d.count) ?? 0;
      const issue = str(d.issue);
      return {
        kind: 'memory-wake',
        icon: 'memoryWake',
        source: 'Memory',
        title: join(`wake-up · ${plural(count, 'memory', 'memories')}`, str(d.project))!,
        ...(issue ? { meta: issue, tone: 'warning' as const } : {}),
        body: plain(str(d.wakeUp)) ?? plain(issue),
      };
    }
    case 'unipi-memory-save-card': {
      const titles = Array.isArray(d.titles) ? (d.titles as unknown[]).filter((t): t is string => typeof t === 'string') : [];
      if (!titles.length) return null;
      const u = rec(d.usage);
      const tok = (num(u.input) ?? 0) + (num(u.output) ?? 0) + (num(u.cacheRead) ?? 0);
      return {
        kind: 'memory-saved',
        icon: 'memorySave',
        source: 'Memory',
        title: `saved ${titles[0]}${titles.length > 1 ? ` +${titles.length - 1}` : ''}`,
        meta: tok ? `side agent · ${kTokens(tok)} tokens` : undefined,
        tone: 'ok',
        body: md(titles.map((t) => `- \`${t}\``).join('\n')),
      };
    }
    // ── subagents / fusion ───────────────────────────────────────────
    case 'subagent-completion': {
      const status = str(det.status) ?? 'completed';
      const u = rec(det.usage);
      return {
        kind: 'subagent-done',
        icon: 'agent',
        source: 'Subagent',
        title: join(str(det.title) ?? str(det.profile) ?? 'finished', status !== 'completed' ? status : undefined)!,
        meta: join(dur(num(det.durationMs)), money(num(u.cost))),
        tone: status === 'completed' ? 'ok' : 'error',
        body: md(str(det.report) ?? text),
      };
    }
    case 'subagent-notification':
      return taskNotification(e, text, 'subagent-notification');
    case 'sidekick-completion': {
      const status = str(det.status) ?? xmlAttr(text, 'subagent_completion_notification', 'status') ?? 'completed';
      const report = str(det.text) ?? text.replace(/<\/?subagent_completion_notification[^>]*>/g, '').trim();
      const u = rec(det.usage);
      return {
        kind: 'sidekick-done',
        icon: 'sidekick',
        source: 'Sidekick',
        title: firstLine(report) || (status === 'completed' ? 'finished' : status),
        meta: join(status !== 'completed' ? status : undefined, dur(num(det.durationMs)), money(num(u.cost))),
        tone: status === 'completed' ? 'ok' : 'error',
        body: md(report) ?? plain(str(det.error)),
      };
    }
    // ── background tasks ─────────────────────────────────────────────
    case 'background-task-notification':
      return bgNotification(e);
    // ── compactor ────────────────────────────────────────────────────
    case 'unipi-compaction':
      return compaction(e);
    case 'compactor-recall':
      return { kind: 'compactor-recall', icon: 'compactor', source: 'Compactor', title: prefixed('session recall', firstLine(text)), body: md(text) };
    case 'unipi-compactor-resume': {
      const events = xmlAttr(text, 'session_resume', 'events');
      const n = xmlAttr(text, 'session_resume', 'compact_count');
      const files = xmlAttr(text, 'files', 'count');
      return {
        kind: 'compactor-resume',
        icon: 'compactor',
        source: 'Compactor',
        title: join('resumed after compaction', events ? `${events} events` : undefined)!,
        meta: join(n ? `${n}× compacted` : undefined, files ? `${files} files` : undefined),
        body: plain(text),
      };
    }
    // ── long-horizon / workflow ──────────────────────────────────────
    case 'long-horizon-decision': {
      const mode = str(d.mode) ?? 'none';
      const source = str(d.source);
      if (mode === 'none' && source !== 'explicit') return null;
      const label = mode === 'none' ? 'Plain' : mode.charAt(0).toUpperCase() + mode.slice(1);
      return { kind: 'mode-switch', icon: 'mode', source: 'Mode', title: label, meta: source && source !== 'explicit' ? source : undefined };
    }
    case 'unipi-progress': {
      if (str(d.kind) !== 'goal') return null;
      const pct = num(d.percent);
      return { kind: 'goal-progress', icon: 'goal', source: 'Goal', title: join(pct !== undefined ? `~${Math.round(pct)}%` : undefined, str(d.summary))!, body: md(str(d.objective) ? `**Objective:** ${str(d.objective)}` : undefined) };
    }
    case 'unipi:lh-continue':
      return { kind: 'lh-continue', icon: 'mode', source: 'Long-horizon', title: prefixed('continuation', firstLine(text)), body: md(text) };
    case 'unipi-ralph-loop-reminder': {
      const m = /\[RALPH LOOP - (.+?) - Iteration (\d+)\/(\d+)\]/.exec(text);
      return { kind: 'ralph-iteration', icon: 'ralph', source: 'Ralph', title: m ? `iteration ${m[2]}/${m[3]} · ${m[1]}` : firstLine(text) || 'loop', body: md(text) };
    }
    case 'unipi:plan-mode-message':
      return { kind: 'plan-mode', icon: 'plan', source: 'Plan mode', title: harnessOf(e).title && harnessOf(e).title !== 'Plan mode' ? harnessOf(e).title! : firstLine(text) || 'plan mode', body: md(text) };
    case 'unipi-watchdog':
      return { kind: 'watchdog', icon: 'watchdog', source: 'Watchdog', title: firstLine(text) || 'warning', tone: 'warning', body: md(text) };
    case 'unipi-skills-revealed':
      return { kind: 'skills-revealed', icon: 'skills', source: 'Skills', title: firstLine(text) || 'skills revealed', body: md(text) };
    // ── dream / btw / browser / attachments ──────────────────────────
    case 'unipi-dream-report-card': {
      const pending = num(d.pending) ?? 0;
      return { kind: 'dream-report', icon: 'dream', source: 'Dream', title: `report ready · ${plural(pending, 'proposal')} pending`, body: plain(str(d.report)), renderer: 'customEntry' };
    }
    case 'btw-thread-entry':
      return { kind: 'btw', icon: 'btw', source: 'Side question', title: str(d.question) ?? 'aside', meta: str(d.model), body: md(str(d.answer)) };
    case 'unipi.browser/turn-shot':
      return { kind: 'browser-shot', icon: 'browser', source: 'Browser', title: str(d.url) ?? str(d.title) ?? 'screenshot', body: json(d), renderer: 'customEntry' };
    case 'unipi:attachments':
    case 'unipi-attachments': {
      const items = Array.isArray(d.items) ? (d.items as unknown[]).map(rec) : [];
      const names = items.map((i) => str(i.name) ?? str(i.path) ?? 'file');
      return { kind: 'attachments', icon: 'attach', source: 'Attachments', title: names.length ? join(plural(names.length, 'file'), names[0])! : 'attachments', body: md(names.map((n) => `- ${n}`).join('\n')), renderer: 'customEntry' };
    }
    case 'unipi-command-output': {
      const t = str(d.text) ?? '';
      return { kind: 'command-output', icon: 'command', source: 'Command', title: join(str(d.command), firstLine(t))!, tone: d.level === 'error' ? 'error' : d.level === 'warning' ? 'warning' : undefined, body: plain(t) };
    }
    case 'unipi-response': {
      if (text.includes('<task-notification>')) return taskNotification(e, text, 'subagent-notification');
      if (!text.trim()) return null;
      const h = harnessOf(e);
      return { kind: 'response', icon: iconOfSource(h.source), source: h.source ?? 'Unipi', title: firstLine(text) || h.synopsis || 'response', body: md(text) };
    }
  }
  if (ct.startsWith('unipi:kanboard-')) return kanboard(ct, e);
  if (ct === 'unipi-continue' || ct.endsWith('-continue')) {
    if (!text.trim()) return null;
  }
  if (e.type === 'custom') return undefined; // unknown state entry → extension renderer only
  // Unknown custom_message: a generic row from harness meta; body = its text (or details as JSON).
  if (!text.trim() && !Object.keys(det).length) return null;
  const h = harnessOf(e);
  const notif = /^<([a-z_-]+-notification)>/.exec(text.trim())?.[1];
  if (notif === 'task-notification') return taskNotification(e, text, 'subagent-notification');
  return {
    kind: 'custom',
    icon: iconOfSource(h.source),
    source: str(det.author) ?? h.source ?? humanType(ct),
    title: h.title && h.title !== h.source ? join(h.title, h.synopsis)! : firstLine(text) || h.synopsis || ct.split(/[:/]/).pop()!.replace(/[-_]+/g, ' '),
    ...(h.warning ? { tone: 'warning' as const } : {}),
    body: notif ? plain(text) : (md(text) ?? json(det)),
    renderer: 'customMessage',
  };
}

/** `acme:weekly-report` → `Acme`. */
function humanType(ct: string): string {
  const head = ct.split(/[:./]/)[0]!.replace(/^unipi-?/, '') || ct;
  const w = head.split('-')[0] || head;
  return w.charAt(0).toUpperCase() + w.slice(1);
}

/** Every visible non-prose entry (see the module doc for the answer kinds). */
function withAudience(e: SessionEntry, v: EventView | null | undefined): EventView | null | undefined {
  if (!v) return v;
  return v.audience ? v : { ...v, audience: eventAudience(e) };
}

export function describeEvent(e: SessionEntry): EventView | null | undefined {
  if (e.type === 'custom_message' || e.type === 'custom') return withAudience(e, describeCustom(e));
  if (e.type === 'branch_summary') {
    const summary = str((e as { summary?: unknown }).summary) ?? '';
    return { kind: 'branch-summary', icon: 'branch', source: 'Branch', title: firstLine(summary) || 'summary of the other branch', audience: 'both', body: md(summary) };
  }
  if (e.type === 'message' && e.message?.role === 'bashExecution') {
    const m = e.message as Record<string, unknown>;
    const command = str(m.command) ?? textOf(m.content as string | Part[] | undefined).trim();
    const exit = num(m.exitCode);
    const cancelled = m.cancelled === true;
    const output = typeof m.output === 'string' ? m.output : '';
    return {
      kind: 'user-bash',
      icon: 'shell',
      source: 'Shell',
      title: command || 'command',
      meta: join(cancelled ? 'cancelled' : exit !== undefined ? `exit ${exit}` : undefined, m.truncated === true ? 'truncated' : undefined, m.excludeFromContext === true ? 'not sent to the model' : undefined),
      tone: cancelled ? 'warning' : exit !== undefined && exit !== 0 ? 'error' : undefined,
      audience: m.excludeFromContext === true ? 'user' : 'both',
      body: plain(output) ?? { format: 'text', text: '(no output)' },
    };
  }
  if (e.type === 'message' && isSummaryRequest(e.message)) {
    // OIN-19: a summarizer prompt that went out as a chat message (`/compact` shadowed by a
    // prompt template): a compaction card, never a user bubble with the prompt verbatim.
    const text = textOf(e.message!.content as string | Part[] | undefined);
    return { kind: 'summary-request', icon: 'compactor', source: 'Compaction', title: 'summary requested · /compact', audience: 'both', body: plain(text) };
  }
  if (e.type === 'message' && e.message?.role === 'system') {
    const m = e.message as Record<string, unknown>;
    const sections = rec(m.sections);
    const text = [textOf(m.content as string | Part[] | undefined), ...Object.values(sections).filter((v): v is string => typeof v === 'string')].filter((s) => s.trim()).join('\n\n');
    if (!text.trim()) return null;
    const agent = /<active_agent name="([^"]+)"/.exec(text)?.[1];
    return { kind: 'system', icon: 'system', source: 'System', title: agent ? `agent prompt · ${agent}` : firstLine(text.replace(/<[^>]+>/g, '')) || 'instructions updated', audience: 'agent', body: plain(text) };
  }
  return undefined;
}
