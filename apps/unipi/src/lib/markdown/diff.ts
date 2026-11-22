/**
 * Unified diff rendering (UNI-156): ```diff / ```patch fences, and the
 * edit/write tool rows in the chat transcript build the same view from
 * their call args. Word-level marks within a paired -/+ line pair use the
 * `diff` package's `diffWordsWithSpace` (small LCS over words).
 */
import { diffWordsWithSpace } from 'diff';
import { escapeHtml } from './escape';

export type DiffLineKind = 'add' | 'del' | 'ctx' | 'hunk' | 'file' | 'meta';
export type DiffLine = { kind: DiffLineKind; text: string; oldNo?: number; newNo?: number };
export type DiffHunk = { header?: string; lines: DiffLine[] };
export type DiffFile = { oldPath?: string; newPath?: string; hunks: DiffHunk[] };
export type ParsedDiff = { files: DiffFile[] };

const HUNK_RE = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@(.*)$/;

/** Parses a unified diff (git-style, with or without `diff --git` headers). */
export function parseUnifiedDiff(text: string): ParsedDiff {
  const lines = text.split('\n');
  const files: DiffFile[] = [];
  let file: DiffFile | undefined;
  let hunk: DiffHunk | undefined;
  let oldNo = 0;
  let newNo = 0;

  const pushFile = () => {
    if (file && (file.oldPath || file.newPath || file.hunks.length > 0)) files.push(file);
    file = undefined;
    hunk = undefined;
  };
  const ensureFile = () => {
    if (!file) file = { hunks: [] };
    return file;
  };

  for (const raw of lines) {
    if (/^diff --git /.test(raw)) {
      pushFile();
      continue;
    }
    const oldMatch = /^--- (?:a\/)?(.+)$/.exec(raw);
    const newMatch = /^\+\+\+ (?:b\/)?(.+)$/.exec(raw);
    if (oldMatch) {
      if (!file || file.hunks.length > 0) pushFile();
      ensureFile().oldPath = oldMatch[1]!.trim() === '/dev/null' ? undefined : oldMatch[1]!.trim();
      continue;
    }
    if (newMatch) {
      ensureFile().newPath = newMatch[1]!.trim() === '/dev/null' ? undefined : newMatch[1]!.trim();
      continue;
    }
    const hm = HUNK_RE.exec(raw);
    if (hm) {
      oldNo = Number(hm[1]);
      newNo = Number(hm[3]);
      hunk = { header: raw, lines: [] };
      ensureFile().hunks.push(hunk);
      continue;
    }
    if (/^(rename|copy|old mode|new mode|new file mode|deleted file mode|index) /.test(raw)) {
      ensureFile();
      continue;
    }
    if (!hunk) continue; // preamble noise before the first hunk
    if (raw === '') continue; // trailing blank line from a final newline
    if (raw.startsWith('+')) {
      hunk.lines.push({ kind: 'add', text: raw.slice(1), newNo });
      newNo++;
    } else if (raw.startsWith('-')) {
      hunk.lines.push({ kind: 'del', text: raw.slice(1), oldNo });
      oldNo++;
    } else if (raw.startsWith('\\')) {
      // "\ No newline at end of file" — ignore
    } else {
      hunk.lines.push({ kind: 'ctx', text: raw.startsWith(' ') ? raw.slice(1) : raw, oldNo, newNo });
      oldNo++;
      newNo++;
    }
  }
  pushFile();
  return { files };
}

/** Change counts over a parsed diff (for the folded +N −M summary). */
export function diffCounts(parsed: ParsedDiff): { add: number; del: number } {
  let add = 0;
  let del = 0;
  for (const f of parsed.files) for (const h of f.hunks) for (const l of h.lines) {
    if (l.kind === 'add') add++;
    else if (l.kind === 'del') del++;
  }
  return { add, del };
}

export type WordMark = { text: string; changed: boolean };

/** Word-level diff between a removed and added line (for highlighting). */
export function wordMarks(oldText: string, newText: string): { oldMarks: WordMark[]; newMarks: WordMark[] } {
  const parts = diffWordsWithSpace(oldText, newText);
  const oldMarks: WordMark[] = [];
  const newMarks: WordMark[] = [];
  for (const part of parts) {
    if (part.added) newMarks.push({ text: part.value, changed: true });
    else if (part.removed) oldMarks.push({ text: part.value, changed: true });
    else {
      oldMarks.push({ text: part.value, changed: false });
      newMarks.push({ text: part.value, changed: false });
    }
  }
  return { oldMarks, newMarks };
}

function markHtml(marks: WordMark[]): string {
  return marks.map((m) => (m.changed ? `<mark>${escapeHtml(m.text)}</mark>` : escapeHtml(m.text))).join('');
}

/** Builds a diff from a plain "all additions" text (write tool, new file). */
export function allAdditions(text: string): ParsedDiff {
  const body = text.replace(/\n$/, '').split('\n');
  return {
    files: [
      {
        hunks: [{ lines: body.map((l, i) => ({ kind: 'add' as const, text: l, newNo: i + 1 })) }],
      },
    ],
  };
}

/** Builds a diff from one `{oldText, newText}` edit pair, word-marked. */
export function editDiff(oldText: string, newText: string): DiffHunk {
  const oldLines = oldText.replace(/\n$/, '').split('\n');
  const newLines = newText.replace(/\n$/, '').split('\n');
  const lines: DiffLine[] = [];
  const maxPairs = Math.min(oldLines.length, newLines.length);
  // Pair line-by-line for word marks when both sides have content; extra
  // lines on either side render as plain add/del.
  for (let i = 0; i < maxPairs; i++) lines.push({ kind: 'del', text: oldLines[i]!, oldNo: i + 1 });
  for (let i = 0; i < oldLines.length - maxPairs; i++) lines.push({ kind: 'del', text: oldLines[maxPairs + i]!, oldNo: maxPairs + i + 1 });
  for (let i = 0; i < maxPairs; i++) lines.push({ kind: 'add', text: newLines[i]!, newNo: i + 1 });
  for (let i = 0; i < newLines.length - maxPairs; i++) lines.push({ kind: 'add', text: newLines[maxPairs + i]!, newNo: maxPairs + i + 1 });
  return { lines };
}

/** Renders a parsed diff (or a single hunk) to sanitised-ready HTML. */
export function renderDiffHtml(parsed: ParsedDiff, opts: { wordMarks?: boolean } = {}): string {
  const useWordMarks = opts.wordMarks !== false;
  const files = parsed.files.map((file) => {
    const head =
      file.oldPath || file.newPath
        ? `<div class="diff-file">${escapeHtml(file.newPath ?? file.oldPath ?? '')}</div>`
        : '';
    const hunks = file.hunks
      .map((hunk) => {
        const headerHtml = hunk.header ? `<div class="diff-hunk-head">${escapeHtml(hunk.header)}</div>` : '';
        const rows: string[] = [];
        const lines = hunk.lines;
        let i = 0;
        while (i < lines.length) {
          const line = lines[i]!;
          // Pair a run of dels immediately followed by an equal-length run of
          // adds for word-level marks (the common "replace these N lines" case).
          if (useWordMarks && line.kind === 'del') {
            let j = i;
            while (j < lines.length && lines[j]!.kind === 'del') j++;
            const delCount = j - i;
            let k = j;
            while (k < lines.length && lines[k]!.kind === 'add') k++;
            const addCount = k - j;
            if (delCount === addCount && delCount > 0) {
              for (let p = 0; p < delCount; p++) {
                const del = lines[i + p]!;
                const add = lines[j + p]!;
                const { oldMarks, newMarks } = wordMarks(del.text, add.text);
                rows.push(gutterRow('del', del.oldNo, undefined, markHtml(oldMarks)));
                rows.push(gutterRow('add', undefined, add.newNo, markHtml(newMarks)));
              }
              i = k;
              continue;
            }
          }
          rows.push(gutterRow(line.kind, line.oldNo, line.newNo, escapeHtml(line.text)));
          i++;
        }
        return `<div class="diff-hunk">${headerHtml}${rows.join('')}</div>`;
      })
      .join('');
    return `<div class="diff-file-block">${head}${hunks}</div>`;
  });
  return `<div class="diffview">${files.join('')}</div>`;
}

function gutterRow(kind: DiffLineKind, oldNo: number | undefined, newNo: number | undefined, innerHtml: string): string {
  const mark = kind === 'add' ? '+' : kind === 'del' ? '−' : ' ';
  const cls = kind === 'add' ? 'diff-add' : kind === 'del' ? 'diff-del' : kind === 'hunk' ? 'diff-hunkline' : 'diff-ctx';
  return (
    `<div class="diff-row ${cls}">` +
    `<span class="diff-no diff-no-old">${oldNo ?? ''}</span>` +
    `<span class="diff-no diff-no-new">${newNo ?? ''}</span>` +
    `<span class="diff-mark">${mark}</span>` +
    `<span class="diff-text">${innerHtml || ' '}</span>` +
    `</div>`
  );
}

/** True when a fence language is a diff/patch fence. */
export function isDiffLang(lang: string): boolean {
  return lang === 'diff' || lang === 'patch';
}
