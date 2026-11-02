/**
 * Phone timeline pieces (phone spec §3.4 / §3.7, Package B): tool call detail in a
 * sheet (PH-27), the group's words summary + failure pill, and one error card per
 * failed turn with an actionable fix (PH-14 / PH-28).
 */
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { For } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { ToolRun, Turn, WorkItem } from '../../lib/session/turns';
import { setDesktopOverride } from '../../lib/layout';
import { clearFullOutputs } from './tools/toolData';
import { ToolRow } from './tools/ToolRow';
import { groupSummary } from './tools/ToolGroup';
import { fakeCtx } from './tools/testutil';
import { PhoneErrorCard } from './PhoneErrorCard';
import type { TimelineCtx } from './context';

beforeEach(() => {
  setDesktopOverride(false);
  clearFullOutputs();
});
afterEach(() => {
  cleanup();
  setDesktopOverride(undefined);
});

const run = (over: Partial<ToolRun> = {}): ToolRun => ({ id: 't1', name: 'bash', args: { command: 'npm test -- --run' }, state: 'ok', output: '✓ 12 tests', ...over });

describe('phone tool rows (PH-27)', () => {
  it('tapping a call opens its input/output in a full sheet, never inline', async () => {
    const r = render(() => <ToolRow tool={run()} ctx={fakeCtx([])} itemKey="k1" />);
    fireEvent.click(r.getByTestId('tl-tool-row'));
    await waitFor(() => expect(document.querySelector('[data-testid="tool-sheet"] .tool-body')).toBeTruthy());
    expect(r.container.querySelector('.tool-body')).toBeNull();
  });
  it('a failed call shows its target and a "failed" pill, not red sentence text', () => {
    const r = render(() => <ToolRow tool={run({ state: 'error', output: 'npm ERR! Missing --otp' })} ctx={fakeCtx([])} itemKey="k2" />);
    expect(r.container.querySelector('.tool-subject')!.textContent).toBe('npm test -- --run');
    expect(r.container.querySelector('.tool-status')!.textContent).toBe('failed');
  });
  it('the group summary says what it did', () => {
    expect(groupSummary([{ name: 'bash' }, { name: 'bash' }, { name: 'read' }])).toBe('Ran 2 commands, read 1 file');
    expect(groupSummary([{ name: 'edit' }])).toBe('Edited 1 file');
  });
});

describe('phone error card (PH-14 / PH-28)', () => {
  const errorCtx = (texts: string[]) => {
    const work: WorkItem[] = texts.map((text, i) => ({ kind: 'error', key: `e${i}-err`, text }));
    const turn = { turnId: 't1', kind: 'turn', segments: [{ id: 's', index: 0, work, nodes: [], live: false }] } as unknown as Turn;
    const base = fakeCtx([]);
    const session = Object.assign(Object.create(null), base.session, { turns: () => [turn], state: { ...base.session.state, run: { running: false } } });
    return { ctx: { ...base, session } as TimelineCtx, work };
  };
  it('four failed attempts render one card: "Failed after 4 attempts", Retry', () => {
    const { ctx, work } = errorCtx(Array(4).fill('OpenRouter returned a server error (502).'));
    const r = render(() => <For each={work}>{(w) => <PhoneErrorCard item={w as Extract<WorkItem, { kind: 'error' }>} ctx={ctx} />}</For>);
    expect(r.container.querySelectorAll('[data-testid="tl-error"]')).toHaveLength(1);
    expect(r.getByTestId('tl-error-attempts').textContent).toContain('Failed after 4 attempts');
    expect(r.getByTestId('tl-error-retry')).toBeTruthy();
  });
  it('"No model selected" offers Choose a model — no /login, no host path', () => {
    const { ctx, work } = errorCtx(['No model selected. Use /login to log into a provider via OAuth or API key. See: /root/qa/agent/docs/docs/providers.md']);
    const r = render(() => <PhoneErrorCard item={work[0] as Extract<WorkItem, { kind: 'error' }>} ctx={ctx} />);
    expect(r.getByTestId('tl-error-title').textContent).toBe('No model selected');
    expect(r.getByTestId('tl-error-fix').textContent).toContain('Choose a model');
    expect(r.container.textContent).not.toMatch(/\/login|\/root/);
  });
});
