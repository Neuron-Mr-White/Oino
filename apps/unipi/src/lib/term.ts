/**
 * Terminal pane support (WP-A31, Z-105..Z-107; host.md §9.3). The PTYs live in
 * the host; the app streams them through `HostClient.openTerm`
 * (`lib/host/tauri.ts` over `src-tauri/src/term.rs`). This module holds the
 * pure, unit-tested parts the `TerminalTab` view builds on:
 *
 * - wire types: the events `term_open` streams on `onEvent` (zod-parsed);
 * - tab naming (Z-105): cwd leaf, then "leaf 2", "leaf 3"… lowest free number;
 * - tab ↔ host terminal bindings, so a reload re-attaches (`term_attach`);
 * - buffer snapshots per terminal (the host keeps no scrollback), restored
 *   before a re-attach;
 * - foreground-process tracking, so closing a busy terminal asks first.
 */
import { z } from 'zod';
import { AppErrorSchema, type AppError } from '@unipi/protocol';

// ── Wire schemas ──────────────────────────────────────────────────────────

export const TermEventSchema = z.discriminatedUnion('type', [
  z.object({ type: z.literal('title'), title: z.string().min(1).max(512) }),
  z.object({ type: z.literal('exit'), code: z.number().int().nullable() }),
  z.object({
    type: z.literal('closed'),
    error: AppErrorSchema.optional(),
  }),
]);
export type TermEvent = z.infer<typeof TermEventSchema>;

/** The `term_open` reply (`src-tauri/src/term.rs` `TermOpenResult`). */
export const TermOpenResultSchema = z.object({
  termId: z.string().min(1),
  shell: z.string().default('sh'),
});
export type TermOpenResult = z.infer<typeof TermOpenResultSchema>;

export type TermEventParsed =
  | { ok: true; event: TermEvent }
  | { ok: false; error: AppError };

export function parseTermEvent(raw: unknown): TermEventParsed {
  const parsed = TermEventSchema.safeParse(raw);
  if (parsed.success) return { ok: true, event: parsed.data };
  return {
    ok: false,
    error: {
      code: 'E_SCHEMA',
      message: 'Invalid terminal event from host',
      retryable: false,
    },
  };
}

// ── Paths & tab naming (Z-105) ────────────────────────────────────────────

/** The `cwd` sent to `term_open`: never empty or relative (the host expands `~`). */
export function normalizeTermCwd(cwd: string | undefined): string {
  const trimmed = (cwd ?? '').trim();
  if (!trimmed || trimmed === '~') return '~';
  if (trimmed.startsWith('~/') || trimmed.startsWith('/') || /^[a-zA-Z]:[\\/]/.test(trimmed)) return trimmed;
  return '~';
}

/** Leaf of a workspace path for tab titles (`/a/b/oino` → `oino`, `~` → `~`, `/` → `/`). */
export function cwdLeaf(cwd: string | undefined): string {
  const norm = normalizeTermCwd(cwd);
  if (norm === '~') return '~';
  const parts = norm.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? '/';
}

/**
 * Z-105: `<leaf>` for the first tab (no "1"), then `<leaf> 2`, `<leaf> 3`…,
 * reusing the lowest free number among `existingTitles`.
 */
export function nextTerminalTitle(cwd: string | undefined, existingTitles: readonly string[]): string {
  const base = cwdLeaf(cwd);
  const used = new Set<number>();
  const prefix = `${base} `;
  for (const raw of existingTitles) {
    const title = raw.trim();
    if (title === base) used.add(1);
    else if (title.startsWith(prefix) && /^[1-9]\d*$/.test(title.slice(prefix.length))) used.add(Number(title.slice(prefix.length)));
  }
  let slot = 1;
  while (used.has(slot)) slot += 1;
  return slot === 1 ? base : `${base} ${slot}`;
}

/** Clamps a PTY dimension to what the wire accepts. */
export function clampDim(value: number, fallback: number): number {
  if (!Number.isFinite(value)) return fallback;
  return Math.max(1, Math.min(1000, Math.round(value)));
}

// ── Tab ↔ host terminal bindings (re-attach across reloads) ───────────────

const BINDINGS_KEY = 'unipi.term.tabs';

export type TabTermBinding = { termId: string; hostId: string; cwd: string; shell?: string };

function storage(kind: 'local' | 'session'): Storage | undefined {
  try {
    return kind === 'local' ? globalThis.localStorage : globalThis.sessionStorage;
  } catch {
    return undefined;
  }
}

function readBindings(): Record<string, TabTermBinding> {
  try {
    const raw = storage('local')?.getItem(BINDINGS_KEY);
    const parsed = raw ? (JSON.parse(raw) as unknown) : null;
    if (!parsed || typeof parsed !== 'object') return {};
    const out: Record<string, TabTermBinding> = {};
    for (const [tabId, v] of Object.entries(parsed as Record<string, unknown>)) {
      const b = v as Partial<TabTermBinding> | null;
      if (b && typeof b.termId === 'string' && typeof b.hostId === 'string') out[tabId] = { termId: b.termId, hostId: b.hostId, cwd: typeof b.cwd === 'string' ? b.cwd : '~', shell: typeof b.shell === 'string' ? b.shell : undefined };
    }
    return out;
  } catch {
    return {};
  }
}

function writeBindings(next: Record<string, TabTermBinding>): void {
  try {
    const st = storage('local');
    if (!st) return;
    if (Object.keys(next).length === 0) st.removeItem(BINDINGS_KEY);
    else st.setItem(BINDINGS_KEY, JSON.stringify(next));
  } catch {
    // storage full / blocked: the tab just opens a fresh shell after a reload
  }
}

export function tabTermBinding(tabId: string): TabTermBinding | undefined {
  return readBindings()[tabId];
}

export function bindTabTerm(tabId: string, binding: TabTermBinding): void {
  writeBindings({ ...readBindings(), [tabId]: binding });
}

export function unbindTabTerm(tabId: string): void {
  const cur = readBindings();
  if (!(tabId in cur)) return;
  delete cur[tabId];
  writeBindings(cur);
}

// ── Buffer snapshots (the host keeps no scrollback) ───────────────────────

const SNAPSHOT_PREFIX = 'unipi.term.snap:';
/** Characters kept per terminal across a reload (serialized xterm buffer tail). */
export const SNAPSHOT_MAX_CHARS = 200_000;

export function saveTermSnapshot(termId: string, data: string): void {
  try {
    const st = storage('session');
    if (!st) return;
    if (!data) {
      st.removeItem(SNAPSHOT_PREFIX + termId);
      return;
    }
    st.setItem(SNAPSHOT_PREFIX + termId, data.length > SNAPSHOT_MAX_CHARS ? data.slice(-SNAPSHOT_MAX_CHARS) : data);
  } catch {
    // quota: the re-attached tab starts with new output only
  }
}

export function readTermSnapshot(termId: string): string | undefined {
  try {
    return storage('session')?.getItem(SNAPSHOT_PREFIX + termId) ?? undefined;
  } catch {
    return undefined;
  }
}

export function clearTermSnapshot(termId: string): void {
  try {
    storage('session')?.removeItem(SNAPSHOT_PREFIX + termId);
  } catch {
    // ignore
  }
}

// ── Foreground process tracking (confirm before kill) ─────────────────────

const PROMPT_SUFFIX_RE = /(?:[$#%>❯➜›λ]|PS [^>]+>)\s*$/;
const SHELL_EXIT_RE = /^\s*(?:exit|logout)(?:\s+\d+)?\s*$/;

/**
 * Whether a terminal is busy with a foreground program, from what the app
 * can see (the host does not report the PTY's foreground process group):
 * - the alternate screen is on (`vi`, `top`, `less`, `htop`, `tmux`…);
 * - OSC 133 shell integration says a command is executing (`C` … `D`);
 * - a submitted command line has not given the prompt back yet
 *   (`sleep 30`, `tail -f`, builds).
 */
export class TermProcessTracker {
  private altScreen = false;
  private osc133Executing = false;
  private hasOsc133 = false;
  private commandRunning = false;
  private exited = false;
  private lineDraft = '';
  private lastCommand: string | undefined;

  onBufferType(type: 'normal' | 'alternate'): void {
    this.altScreen = type === 'alternate';
  }

  onOsc133(payload: string): void {
    const code = payload.split(';')[0]?.trim();
    if (!code) return;
    this.hasOsc133 = true;
    if (code === 'C') this.osc133Executing = true;
    else if (code === 'A' || code === 'B' || code === 'D') {
      this.osc133Executing = false;
      this.commandRunning = false;
    }
  }

  /** Bytes typed into the PTY: tracks the command line the shell will run. */
  onInput(data: string): void {
    if (this.exited || this.altScreen) return;
    for (let i = 0; i < data.length; i += 1) {
      const ch = data[i]!;
      const code = ch.charCodeAt(0);
      if (ch === '\x1b') {
        const next = data[i + 1];
        if (next === '[') {
          i += 2;
          while (i < data.length && (data.charCodeAt(i) < 0x40 || data.charCodeAt(i) > 0x7e)) i += 1;
        } else if (next === 'O') i += 2;
        continue;
      }
      if (code === 0x03 || code === 0x15) {
        this.lineDraft = '';
        continue;
      }
      if (code === 0x7f || code === 0x08) {
        this.lineDraft = this.lineDraft.slice(0, -1);
        continue;
      }
      if (ch === '\r' || ch === '\n') {
        const line = this.lineDraft.trim();
        this.lineDraft = '';
        if (line && !line.startsWith('#') && !SHELL_EXIT_RE.test(line)) {
          this.commandRunning = true;
          this.lastCommand = line;
        }
        continue;
      }
      if (code >= 0x20) this.lineDraft += ch;
    }
  }

  /** The cursor row's text before the cursor once output settles: a prompt there means the command finished. */
  onCursorLine(textBeforeCursor: string): void {
    if (this.altScreen || this.hasOsc133 || !this.commandRunning) return;
    const trimmed = textBeforeCursor.trimEnd();
    if (trimmed && PROMPT_SUFFIX_RE.test(trimmed)) this.commandRunning = false;
  }

  onExit(): void {
    this.exited = true;
    this.altScreen = false;
    this.osc133Executing = false;
    this.commandRunning = false;
  }

  isRunning(): boolean {
    return !this.exited && (this.altScreen || this.osc133Executing || this.commandRunning);
  }

  /** What is running, for the close confirmation (the last command line). */
  hint(): string | undefined {
    return this.isRunning() ? this.lastCommand : undefined;
  }
}

// ── Phone terminal composer history ───────────────────────────────────────

const HISTORY_PREFIX = 'unipi.term.history:';
const HISTORY_MAX = 50;

export function readTermHistory(hostId: string): string[] {
  try {
    const raw = storage('local')?.getItem(HISTORY_PREFIX + hostId);
    const parsed = raw ? (JSON.parse(raw) as unknown) : null;
    return Array.isArray(parsed) ? parsed.filter((x): x is string => typeof x === 'string' && x.length > 0) : [];
  } catch {
    return [];
  }
}

export function pushTermHistory(hostId: string, command: string): string[] {
  const trimmed = command.trim();
  if (!trimmed) return readTermHistory(hostId);
  const next = [trimmed, ...readTermHistory(hostId).filter((c) => c !== trimmed)].slice(0, HISTORY_MAX);
  try {
    storage('local')?.setItem(HISTORY_PREFIX + hostId, JSON.stringify(next));
  } catch {
    // ignore
  }
  return next;
}
