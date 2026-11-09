import { describe, expect, it } from 'vitest';
import { isLongReport, splitBlocked } from './blocked';
import { CLO6_QUESTION, CLO6_REPORT } from './clo6.fixture';

describe('OIN-62: splitBlocked', () => {
  it('pulls the **Need from you:** block out of a long build report', () => {
    const parts = splitBlocked(CLO6_REPORT);
    expect(parts.ask).toBe(CLO6_QUESTION);
    expect(parts.reason).toBe('CLO-6 — Invoice service v2: build report');
    expect(parts.report.startsWith('# CLO-6')).toBe(true);
    expect(parts.report).toContain('## Migrations');
    expect(parts.report).not.toContain('Need from you');
    expect(isLongReport(parts.report)).toBe(true);
  });
  it('reads a QUESTION: line, the block under it, and stops at the next heading', () => {
    const parts = splitBlocked('Built the export.\n\nQUESTION: EUR or USD?\n- EUR\n- USD\n\n## Notes\nSee the PR.');
    expect(parts.ask).toBe('EUR or USD?\n- EUR\n- USD');
    expect(parts.report).toBe('Built the export.\n\n## Notes\nSee the PR.');
  });
  it('reads a heading section, and the last marker wins', () => {
    expect(splitBlocked('Question: stale one\n\nlogs…\n\n## Need from you\nShip it today?').ask).toBe('Ship it today?');
  });
  it('falls back to a trailing question, with the list under it', () => {
    expect(splitBlocked('Status: done.\n\nAll tests pass.\n\nShould I ship now?').ask).toBe('Should I ship now?');
    const listed = splitBlocked('Done.\n\nWhich one?\n\n- staging\n- preview');
    expect(listed.ask).toBe('Which one?\n\n- staging\n- preview');
    expect(listed.report).toBe('Done.');
  });
  it('falls back to the first line when nothing looks like a question', () => {
    const parts = splitBlocked('## Build is stuck\nThe linker fails.\nSee the log.');
    expect(parts.ask).toBe('Build is stuck');
    expect(parts.report).toBe('The linker fails.\nSee the log.');
  });
  it('a short reason is just the question: no report, no reason line', () => {
    expect(splitBlocked('Which domain should the staging deploy use?')).toEqual({ ask: 'Which domain should the staging deploy use?', reason: '', report: '' });
    expect(splitBlocked('  ')).toEqual({ ask: '', reason: '', report: '' });
  });
  it('clips a long first line to one line', () => {
    const reason = splitBlocked(`${'word '.repeat(60)}\n\nmore\n\nOk?`).reason;
    expect(reason.length).toBeLessThanOrEqual(140);
    expect(reason.endsWith('…')).toBe(true);
  });
  it('a short report reads inline', () => {
    expect(isLongReport('One line of context.')).toBe(false);
  });
});
