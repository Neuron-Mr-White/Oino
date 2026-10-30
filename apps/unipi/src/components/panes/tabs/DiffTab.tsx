/**
 * The `diff` side-pane tab (WP-A10, Z-40): the `ReviewViewer` port — the
 * full diff of one edit/write tool call, with the pager across the call's
 * edits, unified **and split** (desktop) views and "Open in editor" at the
 * edit's line. Opened by "View diff" from the tool rows; the inline diff in
 * the row body stays as the lightweight preview (Z-40 hybrid).
 *
 * The tab is addressed by the tool call id (`toolId`), so a persisted tab
 * re-resolves its diff from the session after a reload; `args` is the
 * capture/test escape hatch (a scenario seeds a call without a live session).
 */
import { createMemo, createSignal, Show } from 'solid-js';
import { ChevronLeft, ChevronRight, Columns2, ExternalLink, Rows3 } from 'lucide-solid';
import { setEditorTarget, switchEditorTarget, type EditorTarget } from '../../../lib/editorTarget';
import { resolvePathToken } from '../../../lib/media/pathTokens';
import { allAdditions, editDiff, renderDiffHtml, type DiffHunk, type DiffLine } from '../../../lib/markdown';
import { escapeHtml } from '../../../lib/markdown/escape';
import { desktopShell } from '../../../lib/layout';
import { sessions } from '../../../lib/session/store';
import type { SessionEntry } from '../../../lib/session/types';
import { openSideTab } from '../SidePane';
import { currentWorkspace } from '../../shell/Workspace';
import { setPaneComponent, type PaneTabProps } from '../../../lib/panes/kinds';
import { tabBaseOf } from './sessionTabs';
import './tabs.css';

const obj = (v: unknown): Record<string, unknown> | undefined => (v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : undefined);

/** The toolCall arguments of one entry part (`{edits}` / `{oldText,newText}` / write `{content}`). */
function callArgsOf(entry: SessionEntry | undefined, part: number): Record<string, unknown> | undefined {
  const content = entry?.message?.content;
  if (!Array.isArray(content)) return undefined;
  const block = content[part] as { type?: string; arguments?: unknown } | undefined;
  if (!block || block.type !== 'toolCall') return undefined;
  const args = block.arguments;
  if (obj(args)) return args as Record<string, unknown>;
  if (typeof args === 'string') {
    try {
      const parsed = JSON.parse(args);
      if (obj(parsed)) return parsed as Record<string, unknown>;
    } catch {
      // partial JSON: no full diff yet
    }
  }
  return undefined;
}

/** Where a tool call lives in the branch: its entry, its part index and its arguments. */
export type ToolCallSite = { entryId: string; part: number; args: Record<string, unknown> };

/** Finds the `toolCall` block of `toolId` in the session entries (newest last wins). */
export function findToolCall(entries: readonly SessionEntry[], toolId: string): ToolCallSite | undefined {
  for (let i = entries.length - 1; i >= 0; i--) {
    const entry = entries[i]!;
    const content = entry.message?.content;
    if (!Array.isArray(content)) continue;
    for (let part = content.length - 1; part >= 0; part--) {
      const block = content[part] as { type?: string; id?: unknown } | undefined;
      if (block?.type !== 'toolCall' || block.id !== toolId) continue;
      const args = callArgsOf(entry, part);
      if (args) return { entryId: entry.id, part, args };
    }
  }
  return undefined;
}

/** One edit pair of the call (write = one synthetic all-additions pair). */
export type DiffEdit = { hunk: DiffHunk; /** First changed new line (the "Open in editor" target). */ line?: number };

/** The diff source of a `diff` tab: an edit/write toolCall's arguments. */
export function editsOfArgs(args: Record<string, unknown> | undefined): DiffEdit[] {
  if (!args) return [];
  const raw = Array.isArray(args.edits) ? (args.edits as Array<{ oldText?: string; newText?: string }>) : undefined;
  const pairs =
    raw ??
    (typeof args.oldText === 'string' || typeof args.newText === 'string'
      ? [{ oldText: args.oldText as string | undefined, newText: args.newText as string | undefined }]
      : undefined);
  if (pairs) {
    return pairs.map((e) => {
      const hunk = editDiff(e.oldText ?? '', e.newText ?? '');
      const first = hunk.lines.find((l) => l.kind === 'add');
      return { hunk, line: first?.newNo };
    });
  }
  if (typeof args.content === 'string') return [{ hunk: allAdditions(args.content).files[0]!.hunks[0]!, line: 1 }];
  return [];
}

/** The file a call writes (`write` uses `path`/`file_path`, `edit` the same). */
export function pathOfArgs(args: Record<string, unknown> | undefined): string {
  const p = args?.path ?? args?.file_path;
  return typeof p === 'string' ? p : '';
}

/* ── split rendering: one row per old/new line pair, ctx on both sides ── */

type SplitRow = { kind: 'hunk' | 'pair'; head?: string; old?: DiffLine; new?: DiffLine };

/** Pairs a hunk's del runs with the add runs that follow them (side-by-side). */
export function splitRows(hunks: readonly DiffHunk[]): SplitRow[] {
  const out: SplitRow[] = [];
  for (const hunk of hunks) {
    if (hunk.header) out.push({ kind: 'hunk', head: hunk.header });
    const lines = hunk.lines;
    let i = 0;
    while (i < lines.length) {
      const line = lines[i]!;
      if (line.kind === 'del') {
        const dels: DiffLine[] = [];
        while (i < lines.length && lines[i]!.kind === 'del') dels.push(lines[i++]!);
        const adds: DiffLine[] = [];
        while (i < lines.length && lines[i]!.kind === 'add') adds.push(lines[i++]!);
        for (let k = 0; k < Math.max(dels.length, adds.length); k++) out.push({ kind: 'pair', old: dels[k], new: adds[k] });
        continue;
      }
      if (line.kind === 'add') out.push({ kind: 'pair', new: lines[i++] });
      else out.push({ kind: 'pair', old: line, new: line });
    }
  }
  return out;
}

const cell = (line: DiffLine | undefined, side: 'old' | 'new'): string => {
  if (!line) return `<span class="diff-cell diff-gap"></span>`;
  const own = side === 'old' ? line.kind !== 'add' : line.kind !== 'del';
  const no = side === 'old' ? (line.oldNo ?? '') : (line.newNo ?? '');
  const cls = !own ? 'diff-gap' : line.kind === 'add' ? 'diff-add' : line.kind === 'del' ? 'diff-del' : 'diff-ctx';
  return `<span class="diff-cell ${cls}" data-kind="${line.kind}"><span class="diff-no">${own ? no : ''}</span><span class="diff-mark">${own ? (line.kind === 'add' ? '+' : line.kind === 'del' ? '−' : ' ') : ''}</span><span class="diff-text">${escapeHtml(line.text)}</span></span>`;
};

/** The split view's HTML (sanitised input only, like `renderDiffHtml`). */
export function renderSplitHtml(hunks: readonly DiffHunk[]): string {
  const rows = splitRows(hunks)
    .map((r) =>
      r.kind === 'hunk'
        ? `<div class="diff-row diff-hunkline diff-split-hunk">${escapeHtml(r.head ?? '')}</div>`
        : `<div class="diff-row diff-split">${cell(r.old, 'old')}${cell(r.new, 'new')}</div>`,
    )
    .join('');
  return `<div class="diffview diffview-split">${rows}</div>`;
}

/** The first line the editor should land on (the pager's current edit). */
export function editLine(edits: readonly DiffEdit[], index: number): number | undefined {
  return edits[index]?.line ?? edits[0]?.line;
}

const leafOf = (path: string): string => path.split('/').filter(Boolean).pop() ?? path;

export function DiffTab(props: PaneTabProps) {
  const base = () => {
    const fallback = { hostId: props.hostId, sessionId: props.sessionId };
    const b = tabBaseOf(props.tab.params, fallback);
    return {
      ...b,
      toolId: typeof props.tab.params?.toolId === 'string' ? (props.tab.params.toolId as string) : undefined,
      entryId: typeof props.tab.params?.entryId === 'string' ? (props.tab.params.entryId as string) : undefined,
      part: typeof props.tab.params?.part === 'number' ? (props.tab.params.part as number) : -1,
      path: typeof props.tab.params?.path === 'string' ? (props.tab.params.path as string) : '',
    };
  };
  const cwd = () => (props.projectKey && props.projectKey !== '~' ? props.projectKey : undefined);

  const args = createMemo<Record<string, unknown> | undefined>(() => {
    const seeded = obj(props.tab.params?.args);
    if (seeded) return seeded;
    const b = base();
    if (!b.sessionId) return undefined;
    const entries = sessions().ensure(b.hostId, b.sessionId).state.entries;
    if (b.toolId) return findToolCall(entries, b.toolId)?.args;
    if (b.entryId && b.part >= 0) return callArgsOf(entries.find((e) => e.id === b.entryId), b.part);
    return undefined;
  });
  const edits = createMemo<DiffEdit[]>(() => editsOfArgs(args()));
  const path = () => base().path || pathOfArgs(args());
  const [index, setIndex] = createSignal(Math.max(0, typeof props.tab.params?.edit === 'number' ? (props.tab.params.edit as number) : 0));
  // `split` seeds the view (a persisted tab keeps the mode the user picked).
  const [split, setSplit] = createSignal(props.tab.params?.split === true);
  const single = () => edits().length <= 1;
  const hunk = () => edits()[index()]?.hunk;
  const html = createMemo(() => (hunk() ? (split() ? renderSplitHtml([hunk()!]) : renderDiffHtml({ files: [{ hunks: [hunk()!] }] })) : ''));
  const stat = createMemo(() => {
    let add = 0;
    let del = 0;
    for (const e of edits()) {
      for (const l of e.hunk.lines) {
        if (l.kind === 'add') add++;
        else if (l.kind === 'del') del++;
      }
    }
    return { add, del };
  });

  /** Z-40/Z-5: the editor at the edit's first changed line (dirty editors ask first). */
  const openInEditor = () => {
    const b = base();
    const file = path();
    const line = (typeof props.tab.params?.line === 'number' ? (props.tab.params.line as number) : undefined) ?? editLine(edits(), index());
    const next: EditorTarget = { hostId: b.hostId, path: resolvePathToken(file, cwd()), ...(line ? { line } : {}) };
    if (desktopShell()) {
      switchEditorTarget(next); // a dirty editor asks through its guard
      const ws = currentWorkspace();
      const scope = ws?.scope ?? { hostId: b.hostId, projectKey: props.projectKey };
      const view = ws?.view ?? { sessionId: b.sessionId };
      openSideTab(scope, view, { kind: 'files', params: { hostId: b.hostId, root: cwd() ?? '~', path: next.path } });
      return;
    }
    setEditorTarget(next);
    // The phone's editor layer is a route; navigate by hash (this view also
    // renders in the `SessionTabLayer`, which sits above the router).
    if (typeof location !== 'undefined') location.hash = '#/files/edit';
  };

  const noDiff = () => edits().length === 0;

  return (
    <div class="dtab" data-testid="diff-tab">
      <header class="dtab-head">
        <div class="dtab-title">
          <span class="dtab-path truncate" title={path()}>
            {leafOf(path())}
          </span>
          <Show when={!noDiff()}>
            <span class="dtab-stat" data-testid="diff-stat">
              <span class="add">+{stat().add}</span> <span class="del">−{stat().del}</span>
            </span>
          </Show>
        </div>
        <div class="dtab-tools">
          <Show when={desktopShell() && !single()}>
            <div class="dtab-mode" role="radiogroup" aria-label="Diff view">
              <button type="button" role="radio" aria-checked={!split()} class="dtab-mode-btn" title="Unified" onClick={() => setSplit(false)} data-testid="diff-unified">
                <Rows3 size={14} />
              </button>
              <button type="button" role="radio" aria-checked={split()} class="dtab-mode-btn" title="Split" onClick={() => setSplit(true)} data-testid="diff-split">
                <Columns2 size={14} />
              </button>
            </div>
          </Show>
          <button type="button" class="dtab-open" disabled={!path()} onClick={openInEditor} data-testid="diff-open-editor">
            <ExternalLink size={14} />
            <span>Open in editor</span>
          </button>
        </div>
      </header>
      <div class="dtab-body">
        <Show when={!noDiff()} fallback={<p class="dtab-empty">This edit has no diff to show.</p>}>
          <div class="tool-diff diffview-wrap" innerHTML={html()} />
        </Show>
      </div>
      <Show when={!single()}>
        <footer class="dtab-foot">
          <button type="button" class="ghost-icon-button" aria-label="Previous edit" disabled={index() === 0} onClick={() => setIndex((i) => Math.max(0, i - 1))} data-testid="diff-prev">
            <ChevronLeft size={18} />
          </button>
          <span class="dtab-count">
            edit {index() + 1} of {edits().length}
          </span>
          <button
            type="button"
            class="ghost-icon-button"
            aria-label="Next edit"
            disabled={index() >= edits().length - 1}
            onClick={() => setIndex((i) => Math.min(edits().length - 1, i + 1))}
            data-testid="diff-next"
          >
            <ChevronRight size={18} />
          </button>
        </footer>
      </Show>
    </div>
  );
}

setPaneComponent('diff', DiffTab);
