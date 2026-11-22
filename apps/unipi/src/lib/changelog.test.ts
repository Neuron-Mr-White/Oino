import { describe, expect, it } from 'vitest';
import { changelog, notesFor, parseChangelog } from './changelog';

describe('the bundled changelog', () => {
  it('parses `## <version> — <date>` sections, newest first', () => {
    const list = parseChangelog('# T\n\nintro\n\n## 2.0.0 — 2026-01-02\n\n- b\n\n## 1.0.0\n- a\n');
    expect(list).toEqual([
      { version: '2.0.0', date: '2026-01-02', notes: '- b' },
      { version: '1.0.0', notes: '- a' },
    ]);
  });
  it('ships notes for this build and falls back to the newest entry', () => {
    expect(changelog().length).toBeGreaterThan(0);
    expect(notesFor('1.0.0')?.version).toBe('1.0.0');
    expect(notesFor('9.9.9')?.version).toBe(changelog()[0]!.version);
  });
});
