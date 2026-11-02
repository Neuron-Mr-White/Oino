import { afterEach, describe, expect, it } from 'vitest';
import { isReport, humanCommandNotice, noteCommandRun, openReport, reportHeadline, resetReports, routeNotify } from './reports';
import { toasts } from './toasts';

afterEach(() => {
  resetReports();
  toasts().clear();
});

describe('P1-02: command reports', () => {
  it('a newline or > 160 chars is a report; one short line is not', () => {
    expect(isReport('Saved')).toBe(false);
    expect(isReport('MCP servers\n  fs  connected')).toBe(true);
    expect(isReport('x'.repeat(161))).toBe(true);
  });

  it('a short notice stays a toast', () => {
    routeNotify('Plan mode on', 'info', 'h/s');
    expect(openReport()).toBeNull();
    expect(toasts().list().map((t) => t.message)).toEqual(['Plan mode on']);
  });

  it('a report answering a command just run opens the panel titled with the command', () => {
    noteCommandRun('h/s', 'mcp-status', 1000);
    routeNotify('MCP servers\n  fs  connected\n  git  stopped', 'info', 'h/s', 2000);
    expect(openReport()?.title).toBe('/mcp-status');
    expect(openReport()?.body).toContain('git  stopped');
    expect(toasts().list()).toHaveLength(0);
  });

  it('an unprompted report toasts its first line with Show, which opens the panel', () => {
    routeNotify('\nDream finished\n3 proposals', 'info', 'h/other');
    const [toast] = toasts().list();
    expect(toast?.message).toBe('Dream finished');
    expect(openReport()).toBeNull();
    void toast?.action?.run();
    expect(openReport()?.body).toContain('3 proposals');
  });

  it('headline trims to one toast line', () => {
    expect(reportHeadline(`${'a'.repeat(120)}\nb`)).toHaveLength(90);
    expect(reportHeadline(`${'word '.repeat(25)}\nb`).endsWith('…')).toBe(true);
    expect(reportHeadline(`${'word '.repeat(25)}\nb`)).not.toMatch(/wor…$/);
  });

  it('G-10: ignores empty/whitespace notices and formats permission status on one readable line', () => {
    noteCommandRun('h/s', 'plan status', 1000);
    routeNotify('   \n  ', 'info', 'h/s', 1200);
    expect(toasts().list()).toHaveLength(0);
    expect(openReport()).toBeNull();
    expect(humanCommandNotice('Permission mode: full (this chat: - · default: full)')).toBe('Permission: Full (default)');
    expect(humanCommandNotice('Permission mode: ask (this chat: ask · default for new chats: auto)')).toBe('Permission: Ask (this chat · default: Auto)');
    expect(humanCommandNotice('Plan mode: on — the plan goes to docs/plans/2026-11-02-x.md (/plan view · /plan off)')).toBe('Plan mode: on (2026-11-02-x.md)');
    expect(humanCommandNotice('Plan mode: off (/plan to plan before acting)')).toBe('Plan mode: off');
  });
});
