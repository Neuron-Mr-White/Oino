// @vitest-environment jsdom
/**
 * [lh-ui] graph view over the mock `long-horizon.graph` service: newest-first rows with chips,
 * errored ≠ failed with a Retry that calls the retry op, "Show errors" (persisted), History →
 * read-only snapshot, the timeline's "continued by user", goal Retry.
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { setHostClientFactory } from '../../lib/host/factory';
import { resetSessions } from '../../lib/session/store';
import { resetHubs } from '../../lib/session/hub';
import { resetLayout, setDesktopOverride } from '../../lib/layout';
import { resetLhPrefs } from '../../lib/status/lhPrefs';
import { createMockHost } from '../../dev/mock/host';
import { SCENARIOS } from '../../dev/mock/scenarios';
import { StatusSections } from './StatusPane';

async function boot(name: keyof typeof SCENARIOS) {
  const world = createMockHost({ ...SCENARIOS[name]!, speed: 0 });
  setHostClientFactory(world.client);
  SCENARIOS[name]!.setup?.();
  await new Promise((r) => setTimeout(r, 0));
  return world;
}

beforeEach(() => {
  setDesktopOverride(true);
  window.localStorage.clear();
  resetLhPrefs();
});
afterEach(() => {
  cleanup();
  resetSessions();
  resetHubs();
  resetLayout();
  setHostClientFactory(undefined);
  window.localStorage.clear();
});

const mount = () => render(() => <StatusSections section="lh" hostId="this-pc" sessionId="s001" />);

describe('LhView', () => {
  it('lists newest first with status / profile / model chips; errored is not failed and has Retry', async () => {
    await boot('lh-graph-big');
    mount();
    await waitFor(() => expect(screen.getAllByTestId('lh-node').length).toBeGreaterThan(30));
    const rows = screen.getAllByTestId('lh-node');
    expect(rows[0]!.textContent).toContain('w39'); // created last
    const errored = rows.find((r) => r.dataset.status === 'errored')!;
    expect(errored.textContent).toContain('Error');
    expect(errored.querySelector('[data-testid=lh-node-retry]')).toBeTruthy();
    const failed = rows.find((r) => r.dataset.status === 'failed')!;
    expect(failed.textContent).toContain('Failed');
    expect(failed.querySelector('[data-testid=lh-node-retry]')).toBeNull();
    expect(errored.querySelector('[data-testid=lh-chip-profile]')).toBeTruthy();
    expect(errored.querySelector('[data-testid=lh-chip-model]')!.textContent).toContain('Claude');
    expect(screen.getByTestId('lh-counts').textContent).toContain('2 error');
  });

  it('Retry calls the retry op and the timeline says "Continued by user"', async () => {
    const world = await boot('lh-graph-big');
    mount();
    await waitFor(() => expect(screen.getAllByTestId('lh-node-retry').length).toBe(2));
    fireEvent.click(screen.getAllByTestId('lh-node-retry')[0]!);
    await waitFor(() => expect(world.log.some((l) => l.op === '_oino/service_call' && (l.args as { payload?: { op?: string } }).payload?.op === 'retry')).toBe(true));
    await waitFor(() => expect(screen.getAllByTestId('lh-node-retry').length).toBe(1), { timeout: 3000 });
    fireEvent.click(screen.getByTestId('lh-tab-timeline'));
    await waitFor(() => expect(screen.getAllByTestId('lh-tl-item').some((i) => i.textContent?.includes('Continued by user'))).toBe(true));
  });

  it('filters list and diagram, and persists per graph', async () => {
    await boot('lh-graph-big');
    const view = mount();
    await waitFor(() => expect(screen.getAllByTestId('lh-node').length).toBeGreaterThan(30));
    const header = screen.getByTestId('lh-counts').textContent;
    fireEvent.click(screen.getByTestId('lh-filter-active'));
    expect(screen.getAllByTestId('lh-node').every((r) => ['running', 'queued'].includes(r.dataset.status!))).toBe(true);
    expect(screen.getByTestId('lh-counts').textContent).toBe(header);
    view.unmount();
    mount();
    await waitFor(() => expect(screen.getByTestId('lh-filter-active').getAttribute('aria-pressed')).toBe('true'));
    fireEvent.click(screen.getByTestId('lh-filter-active'));
    fireEvent.click(screen.getByTestId('lh-filter-stopped'));
    expect(screen.queryAllByTestId('lh-node').every((r) => ['stopped', 'cancelled'].includes(r.dataset.status!))).toBe(true);
    fireEvent.click(screen.getByTestId('lh-tab-diagram'));
    expect(document.querySelectorAll('.lh-dia-node:not([data-status="stopped"]):not([data-status="cancelled"])')).toHaveLength(0);
    fireEvent.click(screen.getByTestId('lh-filter-stopped'));
    fireEvent.click(screen.getByTestId('lh-filter-errors'));
    expect(Array.from(document.querySelectorAll('.lh-dia-node')).every((n) => ['errored', 'failed'].includes(n.getAttribute('data-status')!))).toBe(true);
  });

  it('shows whole-graph finished counts while filtering stopped nodes in both views', async () => {
    await boot('lh-finished');
    const view = mount();
    await waitFor(() => expect(screen.getAllByTestId('lh-node')).toHaveLength(7));
    const header = screen.getByTestId('lh-counts').textContent!;
    expect(header).toContain('Finished');
    for (const count of ['0 done', '5 stopped', '1 replaced', '1 blocked']) expect(header).toContain(count);
    expect(header).not.toContain('0/7');
    fireEvent.click(screen.getByTestId('lh-filter-stopped'));
    expect(screen.getAllByTestId('lh-node')).toHaveLength(5);
    expect(screen.getAllByTestId('lh-node').every((n) => n.dataset.status === 'stopped')).toBe(true);
    expect(screen.getByTestId('lh-counts').textContent).toBe(header);
    fireEvent.click(screen.getByTestId('lh-tab-diagram'));
    await waitFor(() => expect(screen.getAllByTestId('lh-dia-node')).toHaveLength(5));
    expect(screen.getAllByTestId('lh-dia-node').every((n) => n.dataset.status === 'stopped')).toBe(true);
    view.unmount();
    mount();
    await waitFor(() => expect(screen.getByTestId('lh-filter-stopped').getAttribute('aria-pressed')).toBe('true'));
    expect(screen.getAllByTestId('lh-node')).toHaveLength(5);
    expect(JSON.parse(localStorage.getItem('lh-filters:graph-oin57')!)).toEqual(['stopped']);
  });

  it('History lists earlier graphs; a row opens a read-only snapshot with no Retry', async () => {
    await boot('lh-history');
    mount();
    await waitFor(() => expect(screen.getByTestId('lh-history')).toBeTruthy());
    fireEvent.click(screen.getByTestId('lh-history'));
    await waitFor(() => expect(screen.getAllByTestId('lh-history-row')).toHaveLength(4));
    expect(screen.getAllByTestId('lh-history-row')[0]!.textContent).toContain('Land the status pane');
    expect(screen.getAllByTestId('lh-history-row')[3]!.textContent).toContain('Stopped');
    fireEvent.click(screen.getAllByTestId('lh-history-row')[3]!);
    await waitFor(() => expect(screen.getByTestId('lh-snapshot-back')).toBeTruthy());
    await waitFor(() => expect(screen.getAllByTestId('lh-node').length).toBe(3));
    expect(screen.queryByTestId('lh-node-retry')).toBeNull();
    fireEvent.click(screen.getByTestId('lh-snapshot-back'));
    await waitFor(() => expect(screen.getAllByTestId('lh-history-row').length).toBe(4));
  });

  it('diagram: one card per visible node with fit control', async () => {
    await boot('lh-swarm');
    mount();
    await waitFor(() => expect(screen.getByTestId('lh-tab-diagram')).toBeTruthy());
    fireEvent.click(screen.getByTestId('lh-tab-diagram'));
    await waitFor(() => expect(screen.getAllByTestId('lh-dia-node')).toHaveLength(8));
    expect(screen.getByTestId('lh-dia-fit')).toBeTruthy();
  });
});

describe('Goal / ralph', () => {
  it('an errored goal shows Retry that calls the service', async () => {
    const world = await boot('lh-goal-errored');
    mount();
    await waitFor(() => expect(screen.getByTestId('lh-loop-retry')).toBeTruthy());
    expect(screen.getByTestId('lh-loop-state').textContent).toBe('Error');
    fireEvent.click(screen.getByTestId('lh-loop-retry'));
    await waitFor(() => expect(world.log.some((l) => l.op === '_oino/service_call' && (l.args as { payload?: { target?: string } }).payload?.target === 'goal')).toBe(true));
  });
  it('ralph shows iteration and the checklist bar', async () => {
    await boot('lh-ralph');
    mount();
    await waitFor(() => expect(screen.getByTestId('lh-ralph-view')).toBeTruthy());
    expect(screen.getByTestId('lh-ralph-view').textContent).toContain('5/9 checked');
    expect(screen.getByTestId('lh-bar')).toBeTruthy();
  });
});
