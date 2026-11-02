// @vitest-environment jsdom
/**
 * Tool rows + bodies (WP-A8 acceptance): a **collapsed row mounts no body
 * DOM**; the native bodies render (run terminal, write `.md` preview,
 * todo, mcp, In/Out); groups (Z-38) hold their members behind the head;
 * the lazy full output pulls `get_entries`.
 */
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { ToolRun } from '../../../lib/session/turns';
import type { SessionEntry } from '../../../lib/session/types';
import { groupWork } from '../../../lib/session/turns';
import { clearFullOutputs } from './toolData';
import { resetSessionRenderCaches } from '../../../lib/session/renderCache';
import { ToolRow } from './ToolRow';
import { WorkNodes } from './WorkItem';
import { fakeCtx } from './testutil';
import { setDesktopOverride } from '../../../lib/layout';

beforeEach(() => {
  setDesktopOverride(true);
  clearFullOutputs();
  resetSessionRenderCaches();
});
afterEach(() => {
  cleanup();
  setDesktopOverride(undefined);
});

const run = (over: Partial<ToolRun> = {}): ToolRun => ({ id: 't1', name: 'bash', args: { command: 'npm test -- --run' }, state: 'ok', output: '✓ 12 tests', ...over });

describe('ToolRow (Z-37)', () => {
  it('collapsed rows mount no body DOM; expanding mounts exactly one', async () => {
    const ctx = fakeCtx([]);
    const r = render(() => <ToolRow tool={run()} ctx={ctx} itemKey="k1" />);
    const row = r.getByTestId('tl-tool-row');
    expect(row.getAttribute('aria-expanded')).toBe('false');
    expect(r.container.querySelector('.tool-body')).toBeNull();
    expect(r.container.querySelector('.tool-term')).toBeNull();
    fireEvent.click(row);
    await waitFor(() => expect(r.container.querySelector('.tool-body')).toBeTruthy());
    expect(r.container.textContent).toContain('$');
    expect(r.container.textContent).toContain('npm test -- --run');
    expect(row.getAttribute('aria-expanded')).toBe('true');
  });

  it('the error state shows the first error line in the subject', () => {
    const ctx = fakeCtx([]);
    const r = render(() => <ToolRow tool={run({ state: 'error', output: 'npm ERR! Missing --otp\nsecond line' })} ctx={ctx} itemKey="k2" />);
    expect(r.container.querySelector('.tool-subject')!.textContent).toBe('npm ERR! Missing --otp');
    expect(r.container.querySelector('.tool-subject')!.getAttribute('data-error')).toBe('');
  });

  it('an edit row carries +N −M from its args', () => {
    const ctx = fakeCtx([]);
    const tool = run({ id: 't2', name: 'edit', args: { path: 'src/a.ts', edits: [{ oldText: 'a\nb', newText: 'a\nb\nc\nd' }] }, output: 'Replaced.' });
    const r = render(() => <ToolRow tool={tool} ctx={ctx} itemKey="k3" />);
    expect(r.container.querySelector('.tool-stat')!.textContent).toMatch(/\+4/);
    expect(r.container.querySelector(".tool-stat")!.textContent).toMatch(/−2/);
  });
});

describe('write body (Z-39: `.md` previews as markdown)', () => {
  const write = (): ToolRun => ({ id: 't3', name: 'write', args: { path: 'docs/NOTES.md', content: '# Notes\n\n- `add` adds now\n' }, state: 'ok', output: 'Wrote docs/NOTES.md.' });
  it('opens on Preview and can switch to the diff', async () => {
    const ctx = fakeCtx([]);
    const r = render(() => <ToolRow tool={write()} ctx={ctx} itemKey="w1" />);
    fireEvent.click(r.getByTestId('tl-tool-row'));
    await waitFor(() => expect(r.container.querySelector('.tool-write-md')).toBeTruthy());
    expect(r.getByTestId('tool-write-preview').getAttribute('aria-selected')).toBe('true');
    fireEvent.click(r.getByTestId('tool-write-diff'));
    await waitFor(() => expect(r.container.querySelector('.tool-diff')).toBeTruthy());
    expect(r.container.querySelector('.tool-write-md')).toBeNull();
  });
  it('a non-markdown write shows the all-additions diff only', async () => {
    const ctx = fakeCtx([]);
    const tool = { ...write(), args: { path: 'src/c.ts', content: 'export const c = 3;\n' } };
    const r = render(() => <ToolRow tool={tool} ctx={ctx} itemKey="w2" />);
    fireEvent.click(r.getByTestId('tl-tool-row'));
    await waitFor(() => expect(r.container.querySelector('.tool-diff')).toBeTruthy());
    expect(r.container.querySelector('.tool-write-tabs')).toBeNull();
  });
});

describe('todo + mcp renderers (Z-39)', () => {
  it('todo_write renders the checklist with states', async () => {
    const ctx = fakeCtx([]);
    const tool = run({ id: 't8', name: 'todo_write', args: { todos: [
      { content: 'Fix the add test', status: 'completed' },
      { content: 'Run the suite', status: 'in_progress', activeForm: 'Running the suite' },
      { content: 'Update the README', status: 'pending' },
    ] }, output: 'Todos updated.' });
    const r = render(() => <ToolRow tool={tool} ctx={ctx} itemKey="td1" />);
    fireEvent.click(r.getByTestId('tl-tool-row'));
    await waitFor(() => expect(r.getByTestId('tool-todo')).toBeTruthy());
    const items = r.container.querySelectorAll('.tool-todo-item');
    expect(items).toHaveLength(3);
    expect(items[0]!.getAttribute('data-status')).toBe('completed');
    expect(items[1]!.getAttribute('data-active')).toBe('');
    expect(items[1]!.textContent).toContain('Running the suite');
  });
  it('mcp__server__tool shows server · fn over the In/Out body', async () => {
    const ctx = fakeCtx([]);
    const tool = run({ id: 't9', name: 'mcp__github__search_issues', args: { q: 'is:open' }, output: '{ "count": 2 }' });
    const r = render(() => <ToolRow tool={tool} ctx={ctx} itemKey="m1" />);
    fireEvent.click(r.getByTestId('tl-tool-row'));
    await waitFor(() => expect(r.container.querySelector('.tool-mcp-head')).toBeTruthy());
    expect(r.container.querySelector('.tool-mcp-server')!.textContent).toBe('github');
    expect(r.container.querySelector('.tool-mcp-fn')!.textContent).toBe('search_issues');
    expect(r.container.querySelector('.tool-io')).toBeTruthy();
  });
});

describe('groups (Z-38)', () => {
  const tools: ToolRun[] = [
    run({ id: 'a', name: 'bash', args: { command: 'npm test' }, output: 'ok' }),
    run({ id: 'b', name: 'bash', args: { command: 'npm run lint' }, output: 'clean' }),
  ];
  it('consecutive same-class tools group; members mount only when open', async () => {
    const ctx = fakeCtx([]);
    const nodes = groupWork([
      { kind: 'tool', key: 'a', tool: tools[0]! },
      { kind: 'tool', key: 'b', tool: tools[1]! },
    ]);
    expect(nodes).toHaveLength(1);
    const r = render(() => <WorkNodes nodes={nodes} ctx={ctx} />);
    const group = r.getByTestId('tl-group');
    expect(group.textContent).toContain('Execute');
    expect(group.textContent).toContain('2 steps');
    expect(r.container.querySelectorAll('[data-testid=tl-tool-row]')).toHaveLength(0);
    fireEvent.click(r.container.querySelector('.tl-group-head')!);
    await waitFor(() => expect(r.container.querySelectorAll('[data-testid=tl-tool-row]')).toHaveLength(2));
  });
  it('a live group defaults open (its segment runs)', () => {
    const ctx = fakeCtx([]);
    const nodes = groupWork([
      { kind: 'tool', key: 'a', tool: { ...tools[0]!, state: 'running' } },
      { kind: 'tool', key: 'b', tool: { ...tools[1]!, state: 'running' } },
    ]);
    const r = render(() => <WorkNodes nodes={nodes} ctx={ctx} live />);
    expect(r.container.querySelectorAll('[data-testid=tl-tool-row]')).toHaveLength(2);
  });
});

describe('lazy full output (get_entries)', () => {
  it('a long run output offers "Show full output", shows an inline loading pill while fetching, and never hides truncated text', async () => {
    const long = `${'x'.repeat(7000)}`;
    const result: SessionEntry = { type: 'message', id: 'r1', message: { role: 'toolResult', toolCallId: 't1', content: [{ type: 'text', text: long }] } } as SessionEntry;
    let resolveEntries!: (entries: SessionEntry[]) => void;
    const ctx = fakeCtx([result], {
      getEntries: () => new Promise<SessionEntry[]>((res) => (resolveEntries = res)),
    });
    const tool = run({ resultId: 'r1', output: long.slice(0, 6200) });
    const r = render(() => <ToolRow tool={tool} ctx={ctx} itemKey="f1" />);
    fireEvent.click(r.getByTestId('tl-tool-row'));
    await waitFor(() => expect(r.getByTestId('tool-full-output')).toBeTruthy());
    const trimmed = r.container.querySelector('.tool-term-out')!.textContent!;
    expect(trimmed.startsWith('…')).toBe(true);
    fireEvent.click(r.getByTestId('tool-full-output'));
    // While fetching, the truncated output stays visible alongside an inline loading indicator (never "Working…")
    expect(r.getByTestId('tool-loading-output').textContent).toContain('Loading full output');
    expect(r.container.querySelector('.tool-term-out')!.textContent!.length).toBeGreaterThan(6000);
    expect(r.container.textContent).not.toContain('Working');
    resolveEntries([result]);
    await waitFor(() => expect(r.container.querySelector('.tool-term-out')!.textContent).toBe(long));
    expect(r.queryByTestId('tool-loading-output')).toBeNull();
  });
});

describe('extension tool renderer (render-only override)', () => {
  it('an answering renderer replaces the fallback body for extension tools', async () => {
    const ctx = fakeCtx([], { renderers: { 'tool/bg_run': { t: 'Paragraph', text: '◆ Background task started' } } });
    const r = render(() => <ToolRow tool={run({ name: 'bg_run', args: { command: 'cargo check' } })} ctx={ctx} itemKey="x1" />);
    fireEvent.click(r.getByTestId('tl-tool-row'));
    await waitFor(() => expect(r.getByTestId('tool-render')).toBeTruthy());
    expect(r.container.textContent).toContain('Background task started');
  });
  it('built-in GUI tools (bash, edit, write, read) always render natively without waiting on _oino/render', async () => {
    const ctx = fakeCtx([], { renderers: { 'tool/bash': { t: 'Paragraph', text: '◆ Ran command' } } });
    const r = render(() => <ToolRow tool={run({ durationMs: 420 })} ctx={ctx} itemKey="x2" />);
    expect(r.getByTestId('tl-tool-dur').textContent).toBe('420ms');
    fireEvent.click(r.getByTestId('tl-tool-row'));
    await waitFor(() => expect(r.container.querySelector('.tool-term')).toBeTruthy());
    expect(r.queryByTestId('tool-render')).toBeNull();
  });

  it('bg_run with task details renders a clean native card without raw temp log paths (D-06)', async () => {
    const ctx = fakeCtx([], { renderers: { 'tool/bg_run': { t: 'Paragraph', text: 'broken-wasm-card' } } });
    const tool = run({
      name: 'bg_run',
      args: { name: 'Read calc and list dir', command: 'cat calc.py; ls -la', isAgent: false },
      output: 'Started background task Read calc and list dir (t95714ebc0001)\nStatus: running\nPID: 42\nOutput: /home/oi/.cache/oino-qa/agent/tmp/tasks/t95714ebc0001/output.log',
      details: {
        task: {
          id: 't95714ebc0001',
          name: 'Read calc and list dir',
          command: 'cat calc.py; ls -la',
          status: 'running',
          outputPath: '/home/oi/.cache/oino-qa/agent/tmp/tasks/t95714ebc0001/output.log',
          notifyOnCompletion: true,
          triggerOnCompletion: true,
        },
      },
    });
    const r = render(() => <ToolRow tool={tool} ctx={ctx} itemKey="bg1" />);
    fireEvent.click(r.getByTestId('tl-tool-row'));
    await waitFor(() => expect(r.getByTestId('tool-bg')).toBeTruthy());
    expect(r.queryByTestId('tool-render')).toBeNull();
    expect(r.container.textContent).toContain('Background');
    expect(r.container.textContent).toContain('Read calc and list dir');
    expect(r.container.textContent).toContain('started · wakes agent');
    expect(r.container.textContent).toContain('cat calc.py; ls -la');
    expect(r.container.textContent).not.toContain('output.log');
  });
});
