import { describe, expect, it } from 'vitest';
import { allAdditions, diffCounts, editDiff, isDiffLang, parseUnifiedDiff, renderDiffHtml, wordMarks } from './diff';

const SAMPLE = `diff --git a/a.ts b/a.ts
index 111..222 100644
--- a/a.ts
+++ b/a.ts
@@ -1,3 +1,3 @@
 const a = 1;
-const b = 2;
+const b = 3;
 const c = 4;
diff --git a/b.ts b/b.ts
new file mode 100644
index 000..333
--- /dev/null
+++ b/b.ts
@@ -0,0 +1,2 @@
+export const x = 1;
+export const y = 2;
`;

describe('parseUnifiedDiff', () => {
  it('splits a multi-file diff into files/hunks/lines', () => {
    const parsed = parseUnifiedDiff(SAMPLE);
    expect(parsed.files).toHaveLength(2);
    expect(parsed.files[0]!.oldPath).toBe('a.ts');
    expect(parsed.files[0]!.newPath).toBe('a.ts');
    expect(parsed.files[0]!.hunks).toHaveLength(1);
    const lines = parsed.files[0]!.hunks[0]!.lines;
    expect(lines.map((l) => l.kind)).toEqual(['ctx', 'del', 'add', 'ctx']);
    expect(lines[1]!.text).toBe('const b = 2;');
    expect(lines[2]!.text).toBe('const b = 3;');
  });

  it('handles a new file (no oldPath)', () => {
    const parsed = parseUnifiedDiff(SAMPLE);
    expect(parsed.files[1]!.oldPath).toBeUndefined();
    expect(parsed.files[1]!.newPath).toBe('b.ts');
    expect(parsed.files[1]!.hunks[0]!.lines.every((l) => l.kind === 'add')).toBe(true);
  });

  it('counts + and - lines', () => {
    const counts = diffCounts(parseUnifiedDiff(SAMPLE));
    expect(counts).toEqual({ add: 3, del: 1 });
  });

  it('isDiffLang recognises diff/patch fences', () => {
    expect(isDiffLang('diff')).toBe(true);
    expect(isDiffLang('patch')).toBe(true);
    expect(isDiffLang('js')).toBe(false);
  });
});

describe('wordMarks', () => {
  it('marks only the changed words between two similar lines', () => {
    const { oldMarks, newMarks } = wordMarks('const b = 2;', 'const b = 3;');
    expect(oldMarks.filter((m) => m.changed).map((m) => m.text)).toEqual(['2']);
    expect(newMarks.filter((m) => m.changed).map((m) => m.text)).toEqual(['3']);
    expect(oldMarks.some((m) => !m.changed && m.text.includes('const'))).toBe(true);
  });

  it('marks every word changed for unrelated lines', () => {
    const { oldMarks, newMarks } = wordMarks('foo bar', 'baz qux');
    expect(oldMarks.filter((m) => m.text.trim()).every((m) => m.changed)).toBe(true);
    expect(newMarks.filter((m) => m.text.trim()).every((m) => m.changed)).toBe(true);
  });
});

describe('allAdditions / editDiff', () => {
  it('builds an all-add diff from plain text (write tool)', () => {
    const parsed = allAdditions('line 1\nline 2\n');
    expect(parsed.files).toHaveLength(1);
    const lines = parsed.files[0]!.hunks[0]!.lines;
    expect(lines).toHaveLength(2);
    expect(lines.every((l) => l.kind === 'add')).toBe(true);
    expect(diffCounts(parsed)).toEqual({ add: 2, del: 0 });
  });

  it('builds a del/add hunk from an {oldText, newText} edit pair', () => {
    const hunk = editDiff('a\nb\n', 'a\nc\nd\n');
    const kinds = hunk.lines.map((l) => l.kind);
    expect(kinds.filter((k) => k === 'del')).toHaveLength(2);
    expect(kinds.filter((k) => k === 'add')).toHaveLength(3);
  });
});

describe('renderDiffHtml', () => {
  it('renders rows with gutters, +/- marks and word-level <mark>s', () => {
    const html = renderDiffHtml(parseUnifiedDiff(SAMPLE));
    expect(html).toContain('diff-add');
    expect(html).toContain('diff-del');
    expect(html).toContain('<mark>2</mark>');
    expect(html).toContain('<mark>3</mark>');
    expect(html).toContain('a.ts');
    expect(html).toContain('b.ts');
  });

  it('skips word marks when disabled', () => {
    const html = renderDiffHtml(parseUnifiedDiff(SAMPLE), { wordMarks: false });
    expect(html).not.toContain('<mark>');
  });
});
