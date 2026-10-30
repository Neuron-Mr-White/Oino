/**
 * `Diff` (ui-protocol §4.5.2): the chat diff view (`.diffview`, `--md-diff-*`
 * tokens, word marks on paired lines). `patch` is a unified diff; `old`/`new`
 * are diffed here with `context` lines (default 3). `Split` shows two columns
 * on the desktop at ≥ 900 px; the phone is always unified (§13.6 rule 7).
 */
import { For, Show, createMemo, createSignal, onCleanup, onMount } from 'solid-js';
import { createTwoFilesPatch } from 'diff';
import { parseUnifiedDiff, wordMarks, type DiffHunk, type DiffLine, type ParsedDiff } from '../../lib/markdown';

type WordMark = ReturnType<typeof wordMarks>['oldMarks'][number];
import { observeSize, shell } from '../layout/units';
import type { DiffNode } from '../types';
import { baseStyle, cx, type NodeProps } from './props';

export function parsedDiff(node: DiffNode): ParsedDiff {
  if (typeof node.patch === 'string') return parseUnifiedDiff(node.patch);
  if (typeof node.old === 'string' && typeof node.new === 'string') {
    const path = node.path ?? 'file';
    const patch = createTwoFilesPatch(path, path, node.old, node.new, '', '', { context: node.context ?? 3 });
    const parsed = parseUnifiedDiff(patch);
    // The header names the same path twice; show it once.
    for (const f of parsed.files) f.oldPath = f.newPath = node.path;
    return parsed;
  }
  return { files: [] };
}

type Row = { line: DiffLine; marks?: WordMark[] };

/** Pairs equal-length del/add runs for word marks (same rule as the chat renderer). */
export function markedRows(hunk: DiffHunk): Row[] {
  const out: Row[] = [];
  const lines = hunk.lines;
  let i = 0;
  while (i < lines.length) {
    if (lines[i]!.kind === 'del') {
      let j = i;
      while (j < lines.length && lines[j]!.kind === 'del') j++;
      let k = j;
      while (k < lines.length && lines[k]!.kind === 'add') k++;
      if (j - i === k - j) {
        const dels: Row[] = [];
        const adds: Row[] = [];
        for (let p = 0; p < j - i; p++) {
          const { oldMarks, newMarks } = wordMarks(lines[i + p]!.text, lines[j + p]!.text);
          dels.push({ line: lines[i + p]!, marks: oldMarks });
          adds.push({ line: lines[j + p]!, marks: newMarks });
        }
        out.push(...dels, ...adds);
        i = k;
        continue;
      }
    }
    out.push({ line: lines[i]! });
    i++;
  }
  return out;
}

const KIND_CLASS: Record<string, string> = { add: 'diff-add', del: 'diff-del', hunk: 'diff-hunkline', ctx: 'diff-ctx', file: 'diff-ctx', meta: 'diff-ctx' };

function Marks(props: { row: Row }) {
  return (
    <Show when={props.row.marks} fallback={props.row.line.text || ' '}>
      <For each={props.row.marks}>{(m) => (m.changed ? <mark>{m.text}</mark> : m.text)}</For>
    </Show>
  );
}

function UnifiedRow(props: { row: Row }) {
  const l = () => props.row.line;
  return (
    <div class={cx('diff-row', KIND_CLASS[l().kind])}>
      <span class="diff-no diff-no-old">{l().oldNo ?? ''}</span>
      <span class="diff-no diff-no-new">{l().newNo ?? ''}</span>
      <span class="diff-mark">{l().kind === 'add' ? '+' : l().kind === 'del' ? '−' : ' '}</span>
      <span class="diff-text">
        <Marks row={props.row} />
      </span>
    </div>
  );
}

/** Side-by-side pairs: context on both sides, del runs beside the add runs that follow them. */
function splitPairs(rows: Row[]): Array<[Row | undefined, Row | undefined]> {
  const out: Array<[Row | undefined, Row | undefined]> = [];
  let i = 0;
  while (i < rows.length) {
    const r = rows[i]!;
    if (r.line.kind === 'del' || r.line.kind === 'add') {
      const dels: Row[] = [];
      const adds: Row[] = [];
      while (i < rows.length && rows[i]!.line.kind === 'del') dels.push(rows[i++]!);
      while (i < rows.length && rows[i]!.line.kind === 'add') adds.push(rows[i++]!);
      for (let p = 0; p < Math.max(dels.length, adds.length); p++) out.push([dels[p], adds[p]]);
      continue;
    }
    out.push([r, r]);
    i++;
  }
  return out;
}

function SplitCell(props: { row: Row | undefined; side: 'old' | 'new' }) {
  const l = () => props.row?.line;
  return (
    <div class={cx('diff-row', l() ? KIND_CLASS[l()!.kind] : 'diff-empty')}>
      <span class="diff-no">{(props.side === 'old' ? l()?.oldNo : l()?.newNo) ?? ''}</span>
      <span class="diff-text">{props.row ? <Marks row={props.row} /> : ' '}</span>
    </div>
  );
}

export function Diff(props: NodeProps<DiffNode>) {
  let el!: HTMLDivElement;
  const [width, setWidth] = createSignal(0);
  onMount(() => onCleanup(observeSize(el, (s) => setWidth(s.width))));
  const parsed = createMemo(() => parsedDiff(props.node));
  const split = () => props.node.mode === 'Split' && shell() === 'desktop' && width() >= 900;
  const counts = createMemo(() => {
    let add = 0;
    let del = 0;
    for (const f of parsed().files) for (const h of f.hunks) for (const l of h.lines) l.kind === 'add' ? add++ : l.kind === 'del' && del++;
    return { add, del };
  });
  return (
    <div ref={el} class={cx('ui-diff', baseStyle(props.node).class)} data-mode={props.mode} data-split={split() ? '' : undefined} style={baseStyle(props.node).style}>
      <div class="diffview" role="group" aria-label={`Diff${props.node.path ? ` of ${props.node.path}` : ''}: ${counts().add} added, ${counts().del} removed`}>
        <For each={parsed().files}>
          {(file) => (
            <div class="diff-file-block">
              <Show when={file.newPath ?? file.oldPath ?? props.node.path}>
                {(p) => (
                  <div class="diff-file">
                    {p()}
                    <span class="ui-diff-counts">
                      <span class="ui-diff-add">+{counts().add}</span> <span class="ui-diff-del">−{counts().del}</span>
                    </span>
                  </div>
                )}
              </Show>
              <For each={file.hunks}>
                {(hunk) => (
                  <div class="diff-hunk">
                    <Show when={hunk.header}>
                      <div class="diff-hunk-head">{hunk.header}</div>
                    </Show>
                    <Show when={split()} fallback={<For each={markedRows(hunk)}>{(r) => <UnifiedRow row={r} />}</For>}>
                      <div class="ui-diff-split">
                        <For each={splitPairs(markedRows(hunk))}>
                          {([a, b]) => (
                            <>
                              <SplitCell row={a} side="old" />
                              <SplitCell row={b} side="new" />
                            </>
                          )}
                        </For>
                      </div>
                    </Show>
                  </div>
                )}
              </For>
            </div>
          )}
        </For>
      </div>
    </div>
  );
}
