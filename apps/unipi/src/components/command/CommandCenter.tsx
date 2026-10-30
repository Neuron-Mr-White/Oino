/**
 * Command center (WP-A18, Z-77/Z-78/Z-80/Z-81): Cmd/Ctrl+K (and
 * Cmd/Ctrl+Shift+P) opens the one palette surface — scope tabs
 * All / Actions / Sessions / Files, prefixes `>` `#` `@`, the empty state
 * (Recent sessions + grouped commands, ≤ 3 per group + "show more"),
 * per-workspace search-history chips (20, dedup), session `/` commands in
 * Actions (insert `/cmd ` into the composer) and `fs_search` files in Files.
 *
 * Pure helpers (`parsePrefixQuery`, `scoreRows`, history) are exported for
 * tests. The look is the palette's (unipi tokens); ZCode behaviour.
 */
import { createEffect, createMemo, createResource, createSignal, For, Match, on, onCleanup, onMount, Show, Switch } from 'solid-js';
import { useLocation, useNavigate } from '@solidjs/router';
import { Portal } from 'solid-js/web';
import { ChevronRight, FileText, MessageSquare, Search, SquareSlash } from 'lucide-solid';
import { allCommands, paletteOpen, paletteScope, rankCommands, setPaletteOpen, type Command, type PaletteScope } from '../../lib/shell';
import { hubs } from '../../lib/session/hub';
import { sessions } from '../../lib/session/store';
import { sessionTitle, lastActivity } from '../../lib/session/selectors';
import type { SessionCard } from '../../lib/host/client';
import { fsSearch, fsRoots } from '../../lib/fs';
import { useHostsOptional } from '../../hosts/context';
import { setComposerText, focusComposer } from '../../lib/timeline/actions';
import { layout } from '../../lib/layout';
import { currentWorkspace } from '../shell/Workspace';
import { panes } from '../../lib/panes';
import { setEditorTarget } from '../../lib/editorTarget';
import { agoText } from '../status/lhProgress';
import { showActionNotice } from '../../lib/timeline/actions';
import { pushBack } from '../../lib/back';
import './command.css';

export const SCOPES: Array<{ id: PaletteScope; label: string }> = [
  { id: 'all', label: 'All' },
  { id: 'actions', label: 'Actions' },
  { id: 'sessions', label: 'Sessions' },
  { id: 'files', label: 'Files' },
];

const PREFIX_SCOPE: Record<string, PaletteScope> = { '>': 'actions', '#': 'sessions', '@': 'files' };

/** Pure: a query with a scope prefix (`>git`) → the scope it forces + the bare query. */
export function parsePrefixQuery(query: string): { scope: PaletteScope | undefined; text: string } {
  const trimmed = query.replace(/^\s+/, '');
  const prefix = PREFIX_SCOPE[trimmed[0] ?? ''];
  return prefix ? { scope: prefix, text: trimmed.slice(1).replace(/^\s+/, '') } : { scope: undefined, text: query.trim() };
}

/** Pure ranking shared by sessions and files: every word must match; earlier + stronger hits win. */
export function scoreRows<T>(rows: readonly T[], query: string, fields: (row: T) => { title: string; hay: string }): Array<{ row: T; score: number; index: number }> {
  const words = query.toLowerCase().trim().split(/\s+/).filter(Boolean);
  if (!words.length) return rows.map((row, index) => ({ row, score: 0, index }));
  const scored: Array<{ row: T; score: number; index: number }> = [];
  rows.forEach((row, index) => {
    const { title, hay } = fields(row);
    const t = title.toLowerCase();
    const h = hay.toLowerCase();
    let score = 0;
    for (const word of words) {
      if (t.startsWith(word)) score += 100;
      else if (new RegExp(`\\b${word.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}`).test(t)) score += 60;
      else if (t.includes(word)) score += 40;
      else if (h.includes(word)) score += 20;
      else return;
    }
    scored.push({ row, score, index });
  });
  return scored.sort((a, b) => b.score - a.score || a.index - b.index);
}

// ── per-workspace search history (Z-78) ─────────────────────────────────

export type HistoryEntry = { q: string; scope: PaletteScope };
const HISTORY_KEY = 'unipi.cmdk.history';
export const HISTORY_MAX = 20;

function readHistoryStore(): Record<string, HistoryEntry[]> {
  try {
    const parsed = JSON.parse(localStorage.getItem(HISTORY_KEY) ?? '{}') as Record<string, unknown>;
    const out: Record<string, HistoryEntry[]> = {};
    for (const [key, list] of Object.entries(parsed)) {
      if (!Array.isArray(list)) continue;
      out[key] = list.filter((e): e is HistoryEntry => !!e && typeof (e as HistoryEntry).q === 'string' && typeof (e as HistoryEntry).scope === 'string').slice(0, HISTORY_MAX);
    }
    return out;
  } catch {
    return {};
  }
}

/** Pure: the workspace's history, newest first, deduped by scope+query, capped. */
export function historyFor(store: Record<string, HistoryEntry[]>, workspace: string): HistoryEntry[] {
  return store[workspace] ?? [];
}

/** Pure: `entry` moved to the front (dedup by scope+q, cap `HISTORY_MAX`). */
export function pushHistory(list: readonly HistoryEntry[], entry: HistoryEntry): HistoryEntry[] {
  const key = (e: HistoryEntry) => `${e.scope}:${e.q}`;
  return [entry, ...list.filter((e) => key(e) !== key(entry))].slice(0, HISTORY_MAX);
}

function saveHistory(workspace: string, next: HistoryEntry[]): void {
  try {
    const store = readHistoryStore();
    if (next.length) store[workspace] = next;
    else delete store[workspace];
    localStorage.setItem(HISTORY_KEY, JSON.stringify(store));
  } catch {
    // not remembered
  }
}

/** The workspace the history + files scope key on (§3.15: `hostId|projectKey`). */
export function historyWorkspace(): string {
  const ws = currentWorkspace();
  return ws ? `${ws.scope.hostId}|${ws.scope.projectKey}` : 'app';
}

// ── rows ────────────────────────────────────────────────────────────────

export type SessionRow = { kind: 'session'; key: string; hostId: string; hostName: string; card: SessionCard };
export type FileRow = { kind: 'file'; key: string; hostId: string; path: string; root: string };
export type SlashRow = { kind: 'slash'; key: string; name: string; description?: string };
export type Selectable = SessionRow | FileRow | SlashRow | { kind: 'command'; key: string; command: Command };
export type Row = Selectable | { kind: 'header'; label: string } | { kind: 'more'; key: string; group: string; count: number } | { kind: 'note'; text: string };

export function CommandCenter() {
  return (
    <Show when={paletteOpen()}>
      <CenterDialog />
    </Show>
  );
}

function CenterDialog() {
  const navigate = useNavigate();
  const location = useLocation();
  const hosts = useHostsOptional();
  const [query, setQuery] = createSignal('');
  const [index, setIndex] = createSignal(0);
  const [scope, setScope] = createSignal<PaletteScope>(paletteScope());
  const [expanded, setExpanded] = createSignal<Set<string>>(new Set());
  const [historyOpen, setHistoryOpen] = createSignal(false);
  const [history, setHistory] = createSignal<HistoryEntry[]>([]);
  let input: HTMLInputElement | undefined;
  let list: HTMLDivElement | undefined;

  const close = () => setPaletteOpen(false);
  onCleanup(pushBack(close));
  onMount(() => {
    setHistory(historyFor(readHistoryStore(), historyWorkspace()));
    queueMicrotask(() => input?.focus());
  });
  // The shell can retarget the scope while open (Mod+P → Files, Z-81).
  createEffect(on(paletteScope, (s) => setScope(s), { defer: true }));
  createEffect(() => {
    query();
    setIndex(0);
    setExpanded(new Set<string>());
  });
  createEffect(() => {
    const row = list?.querySelector<HTMLElement>(`[data-index="${index()}"]`);
    row?.scrollIntoView?.({ block: 'nearest' });
  });

  const parsed = createMemo(() => parsePrefixQuery(query()));
  // A prefix both forces the scope and shows the tab as active.
  const activeScope = () => parsed().scope ?? scope();
  const bare = () => parsed().text;

  // ── sources ───────────────────────────────────────────────────────
  const orderedHosts = createMemo(() => {
    const all = hosts?.hosts() ?? [];
    return [...all].sort((a, b) => Number(b.transport === 'local') - Number(a.transport === 'local'));
  });
  const sessionRows = createMemo<SessionRow[]>(() => {
    const out: SessionRow[] = [];
    for (const host of orderedHosts()) {
      const hub = hubs().ensure(host.id);
      for (const card of Object.values(hub.state.cards)) {
        out.push({ kind: 'session', key: `${host.id}/${card.session_id}`, hostId: host.id, hostName: host.name, card });
      }
    }
    return out.sort((a, b) => lastActivity(b.card) - lastActivity(a.card));
  });
  const currentSession = createMemo(() => {
    const m = /^\/s\/([^/]+)\/([^/?#]+)/.exec(location.pathname);
    return m ? { hostId: decodeURIComponent(m[1]!), sessionId: decodeURIComponent(m[2]!) } : undefined;
  });
  /** The session's `/` commands in Actions (Z-80): non-builtin, run inserts `/cmd `. */
  const slashRows = createMemo<SlashRow[]>(() => {
    const cur = currentSession();
    if (!cur) return [];
    const handle = sessions().peek(cur.hostId, cur.sessionId);
    if (!handle) return [];
    return handle.state.commands
      .filter((c) => c.source !== 'builtin' && c.name.startsWith('/'))
      .map((c) => ({ kind: 'slash' as const, key: `slash:${c.name}`, name: c.name, description: c.description }));
  });
  /** The files scope's host + root: the workspace's folder, else This computer's first root. */
  const fileSource = createMemo(() => {
    const ws = currentWorkspace();
    if (ws && ws.scope.projectKey !== '~') return { hostId: ws.scope.hostId, root: ws.scope.projectKey };
    const local = orderedHosts().find((h) => h.transport === 'local') ?? orderedHosts()[0];
    return local ? { hostId: local.id, root: undefined as string | undefined } : undefined;
  });
  /** `fs_roots` once when there is no workspace folder to key on (Z-81); refetches when the host resolves. */
  const [rootFallback] = createResource(
    () => fileSource()?.hostId,
    async (hostId) => {
      if (!hostId) return null;
      const src = fileSource();
      if (src?.root) return null;
      const roots = await fsRoots(hostId);
      return roots.ok && roots.value.roots[0] ? { hostId, root: roots.value.roots[0]! } : null;
    },
  );
  const searchRoot = createMemo((): { hostId: string; root: string } | undefined => {
    const src = fileSource();
    if (src?.root) return { hostId: src.hostId, root: src.root };
    const fb = rootFallback.latest;
    return fb && typeof fb === 'object' ? { hostId: fb.hostId, root: fb.root } : undefined;
  });
  let fileSeq = 0;
  const [fileRows] = createResource(
    () => ({ q: bare(), scope: activeScope(), root: searchRoot() }),
    async (src) => {
      const { q, scope } = src;
      const root = src.root;
      if (!q || q.length < 2 || !root || scope === 'actions' || scope === 'sessions') return [];
      const seq = ++fileSeq;
      await new Promise((r) => setTimeout(r, 140)); // debounce
      if (seq !== fileSeq) return []; // a newer query superseded this one
      const res = await fsSearch(root.hostId, root.root, q, 'name');
      if (seq !== fileSeq) return [];
      return res.ok ? res.value.results.slice(0, 50).map((hit) => ({ kind: 'file' as const, key: `file:${hit.path}`, hostId: root.hostId, path: hit.path, root: root.root })) : [];
    },
  );

  // ── the row list (grouped, ≤ 3 + show more) ───────────────────────
  const rows = createMemo<Row[]>(() => buildRows({
    scope: activeScope(),
    query: bare(),
    commands: allCommands(),
    slash: slashRows(),
    sessions: sessionRows(),
    files: (fileRows() ?? []) as FileRow[],
    expanded: expanded(),
    filesBusy: fileRows.loading,
  }));
  const navigable = createMemo(() => rows().filter((r) => r.kind !== 'header' && r.kind !== 'note'));

  const run = (row: Row | undefined) => {
    if (!row || row.kind === 'header' || row.kind === 'note') return;
    if (row.kind === 'more') {
      setExpanded((s) => new Set([...s, row.group]));
      return;
    }
    const q = bare();
    if (q) {
      const next = pushHistory(history(), { q, scope: activeScope() });
      setHistory(next);
      saveHistory(historyWorkspace(), next);
    }
    close();
    queueMicrotask(() => runRow(row, navigate));
  };
  const onKey = (event: KeyboardEvent) => {
    const items = navigable();
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      setIndex((i) => (items.length ? (i + 1) % items.length : 0));
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      setIndex((i) => (items.length ? (i - 1 + items.length) % items.length : 0));
    } else if (event.key === 'Enter') {
      event.preventDefault();
      run(items[index()]);
    }
  };

  return (
    <Portal>
      <div class="palette-backdrop" onClick={close} />
      <div class="palette cmdk" role="dialog" aria-modal="true" aria-label="Command center" data-testid="command-center">
        <div class="cmdk-head">
          <div class="palette-search cmdk-search">
            <Search size={18} />
            <input
              ref={input}
              class="palette-input"
              placeholder="Search commands, sessions, files…"
              aria-label="Search"
              value={query()}
              onInput={(e) => setQuery(e.currentTarget.value)}
              onKeyDown={onKey}
              autocomplete="off"
              spellcheck={false}
            />
            <kbd class="kbd">Esc</kbd>
          </div>
          <div class="cmdk-scopes" role="tablist" aria-label="Scope">
            <For each={SCOPES}>
              {(s) => (
                <button
                  type="button"
                  role="tab"
                  class="cmdk-scope"
                  aria-selected={activeScope() === s.id}
                  onClick={() => {
                    // A tab is a filter (ZCode): keep the typed words, drop a
                    // scope prefix (`>go` → `go`) since the tab now governs.
                    setScope(s.id);
                    if (parsed().scope) setQuery(parsed().text);
                    queueMicrotask(() => input?.focus());
                  }}
                >
                  {s.label}
                </button>
              )}
            </For>
          </div>
          <Show when={!bare() && history().length > 0}>
            <HistoryChips history={history()} open={historyOpen()} onToggle={() => setHistoryOpen((o) => !o)} onPick={(e) => (setScope(e.scope), setQuery(e.q))} />
          </Show>
        </div>
        <div class="palette-list" ref={list} data-testid="cmdk-list">
          <RowList rows={rows()} index={index()} onHover={setIndex} onRun={run} onMore={(group) => setExpanded((s) => new Set([...s, group]))} />
        </div>
        <div class="palette-foot">
          <span><kbd class="kbd">↑</kbd><kbd class="kbd">↓</kbd> move</span>
          <span><kbd class="kbd">↵</kbd> open</span>
          <span class="cmdk-prefixes"><kbd class="kbd">&gt;</kbd> actions <kbd class="kbd">#</kbd> sessions <kbd class="kbd">@</kbd> files</span>
        </div>
      </div>
    </Portal>
  );
}

/** Runs a picked row after the dialog unmounts (focus/scroll land on the target). */
function runRow(row: Selectable, navigate: (to: string) => void): void {
  if (row.kind === 'session') {
    navigate(`/s/${encodeURIComponent(row.hostId)}/${encodeURIComponent(row.card.session_id)}`);
    return;
  }
  if (row.kind === 'slash') {
    const cur = currentSessionOf();
    if (!cur) return;
    const key = `${cur.hostId}/${cur.sessionId}`;
    setComposerText(key, `${row.name} `);
    focusComposer(key);
    return;
  }
  if (row.kind === 'file') {
    openFileFromCenter(row.hostId, row.path);
    return;
  }
  row.command.run();
}

function currentSessionOf(): { hostId: string; sessionId: string } | undefined {
  const m = /^\/s\/([^/]+)\/([^/?#]+)/.exec(location.hash);
  return m ? { hostId: decodeURIComponent(m[1]!), sessionId: decodeURIComponent(m[2]!) } : undefined;
}

/** A Files-scope hit opens like GitSection does: the Files tab seeded with the path. */
export function openFileFromCenter(hostId: string, path: string): void {
  const dir = path.slice(0, path.lastIndexOf('/'));
  if (layout().desktop()) {
    const ws = currentWorkspace();
    if (!ws) {
      showActionNotice('Open a chat in a project to browse files');
      return;
    }
    panes().open(ws.scope, 'side', ws.view, { kind: 'files', params: { hostId, root: dir, path } });
    layout().toggleSide(true);
    return;
  }
  setEditorTarget({ hostId, path });
  window.location.hash = '#/files/edit';
}

// ── pure row building (exported for tests) ─────────────────────────────

export type BuildRowsInput = {
  scope: PaletteScope;
  query: string;
  commands: Command[];
  slash: SlashRow[];
  sessions: SessionRow[];
  files: FileRow[];
  expanded: ReadonlySet<string>;
  filesBusy?: boolean;
};

/** ≤ 3 rows per group + "show more" (expanded groups show everything). */
function group(rows: Row[], label: string, items: Row[], expanded: ReadonlySet<string>): void {
  if (!items.length) return;
  rows.push({ kind: 'header', label });
  const limit = expanded.has(label) ? items.length : Math.min(3, items.length);
  rows.push(...items.slice(0, limit));
  if (items.length > limit) rows.push({ kind: 'more', key: `more:${label}`, group: label, count: items.length - limit });
}

const commandRow = (c: Command): Row => ({ kind: 'command', key: `cmd:${c.id}`, command: c });

/** Pure: the center's rows for a scope + query (Z-77: empty = recent sessions + grouped commands). */
export function buildRows(input: BuildRowsInput): Row[] {
  const { scope, query, commands, slash, sessions, files, expanded } = input;
  const rankedCommands = query ? rankCommands(commands, query) : commands;
  const rankedSessions = query
    ? scoreRows(sessions, query, (r) => ({ title: sessionTitle(r.card), hay: `${r.card.cwd ?? ''} ${r.hostName}` })).map((s) => s.row)
    : sessions;
  const rankedSlash = query ? scoreRows(slash, query, (r) => ({ title: r.name, hay: r.description ?? '' })).map((s) => s.row) : slash;
  const rankedFiles = query
    ? scoreRows(files, query, (r) => ({ title: r.path.split('/').pop() ?? r.path, hay: r.path })).map((s) => s.row)
    : files;
  const out: Row[] = [];

  if (scope === 'sessions') {
    if (!query) out.push({ kind: 'header', label: 'Recent sessions' });
    out.push(...rankedSessions);
    if (!rankedSessions.length) out.push({ kind: 'note', text: query ? 'No matching sessions' : 'No sessions yet' });
    return out;
  }
  if (scope === 'files') {
    if (!query || query.length < 2) {
      out.push({ kind: 'note', text: 'Type at least 2 characters to search files' });
      return out;
    }
    out.push(...rankedFiles);
    if (!rankedFiles.length) out.push({ kind: 'note', text: input.filesBusy ? 'Searching…' : 'No matching files' });
    return out;
  }
  if (scope === 'actions') {
    if (!query) {
      const groups: string[] = [];
      for (const c of commands) if (!groups.includes(c.group)) groups.push(c.group);
      for (const g of groups) group(out, g, commands.filter((c) => c.group === g).map(commandRow), expanded);
      group(out, 'Chat commands', rankedSlash, expanded);
      return out;
    }
    out.push(...rankedCommands.map(commandRow));
    group(out, 'Chat commands', rankedSlash, expanded);
    if (!out.length) out.push({ kind: 'note', text: 'No matching commands' });
    return out;
  }

  // All
  if (!query) {
    group(out, 'Recent sessions', rankedSessions.slice(0, 3), new Set());
    const groups: string[] = [];
    for (const c of commands) if (!groups.includes(c.group)) groups.push(c.group);
    for (const g of groups) group(out, g, commands.filter((c) => c.group === g).map(commandRow), expanded);
    group(out, 'Chat commands', rankedSlash, expanded);
    return out;
  }
  group(out, 'Commands', rankedCommands.map(commandRow), expanded);
  group(out, 'Chat commands', rankedSlash, expanded);
  group(out, 'Sessions', rankedSessions, expanded);
  group(out, 'Files', rankedFiles, expanded);
  if (!out.some((r) => r.kind !== 'header')) out.push({ kind: 'note', text: 'Nothing found' });
  return out;
}

// ── rendering ──────────────────────────────────────────────────────────

function HistoryChips(props: { history: HistoryEntry[]; open: boolean; onToggle: () => void; onPick: (entry: HistoryEntry) => void }) {
  const visible = () => (props.open ? props.history : props.history.slice(0, 4));
  return (
    <div class="cmdk-history" data-testid="cmdk-history">
      <For each={visible()}>
        {(entry) => (
          <button type="button" class="cmdk-chip" onClick={() => props.onPick(entry)} title={entry.q}>
            {entry.q}
          </button>
        )}
      </For>
      <Show when={!props.open && props.history.length > 4}>
        <button type="button" class="cmdk-chip cmdk-chip-more" aria-label={`Show ${props.history.length - 4} more searches`} onClick={props.onToggle}>
          +{props.history.length - 4}
        </button>
      </Show>
    </div>
  );
}

function RowList(props: { rows: Row[]; index: number; onHover: (i: number) => void; onRun: (row: Row) => void; onMore: (group: string) => void }) {
  let n = -1;
  return (
    <For each={props.rows} fallback={<p class="palette-empty">Nothing found</p>}>
      {(row) => {
        if (row.kind === 'header') return <p class="palette-group">{row.label}</p>;
        if (row.kind === 'note') return <p class="cmdk-note" data-testid="cmdk-note">{row.text}</p>;
        n += 1;
        const at = n;
        return (
          <button
            type="button"
            class="palette-row cmdk-row"
            data-index={at}
            data-kind={row.kind}
            aria-selected={props.index === at}
            onMouseMove={() => props.onHover(at)}
            onClick={() => (row.kind === 'more' ? props.onMore(row.group) : props.onRun(row))}
          >
            <RowBody row={row} />
          </button>
        );
      }}
    </For>
  );
}

function RowBody(props: { row: Row }) {
  const row = () => props.row;
  return (
    <Switch>
      <Match when={row().kind === 'session'}>{SessionBody(row() as SessionRow)}</Match>
      <Match when={row().kind === 'file'}>{FileBody(row() as FileRow)}</Match>
      <Match when={row().kind === 'slash'}>{SlashBody(row() as SlashRow)}</Match>
      <Match when={row().kind === 'command'}>{CommandBody(row() as { kind: 'command'; key: string; command: Command })}</Match>
      <Match when={row().kind === 'more'}>{<span class="cmdk-more">Show {(row() as { count: number }).count} more</span>}</Match>
    </Switch>
  );
}

function SessionBody(r: SessionRow) {
  return (
    <>
      <span class="palette-row-icon"><MessageSquare size={16} /></span>
      <span class="cmdk-row-main">
        <span class="palette-row-title">{sessionTitle(r.card)}</span>
        <span class="cmdk-row-sub">
          {r.hostName !== 'This computer' ? `${r.hostName} · ` : ''}
          {(r.card.cwd ?? '').split('/').pop() || 'No folder'} · {agoText(Date.now() - (r.card.updated_at ?? 0))}
        </span>
      </span>
      <Show when={r.card.unread}>
        <span class="cmdk-dot" title="Unread" />
      </Show>
    </>
  );
}

function FileBody(r: FileRow) {
  return (
    <>
      <span class="palette-row-icon"><FileText size={16} /></span>
      <span class="cmdk-row-main">
        <span class="palette-row-title">{r.path.split('/').pop()}</span>
        <span class="cmdk-row-sub">{r.path}</span>
      </span>
    </>
  );
}

function SlashBody(r: SlashRow) {
  return (
    <>
      <span class="palette-row-icon"><SquareSlash size={16} /></span>
      <span class="cmdk-row-main">
        <span class="palette-row-title">{r.name}</span>
        <Show when={r.description}>
          <span class="cmdk-row-sub">{r.description}</span>
        </Show>
      </span>
    </>
  );
}

function CommandBody(r: { command: Command }) {
  return (
    <>
      <span class="palette-row-icon">{r.command.icon?.() ?? <ChevronRight size={16} />}</span>
      <span class="palette-row-title">{r.command.title}</span>
      <Show when={r.command.keys}>
        <span class="palette-row-keys">
          <For each={r.command.keys!.split(' ')}>{(key) => <kbd class="kbd">{key}</kbd>}</For>
        </span>
      </Show>
    </>
  );
}
