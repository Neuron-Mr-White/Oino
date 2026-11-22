// @vitest-environment jsdom
/**
 * UNI-269: the working-mode chip (hidden in plain mode, icon + name + count,
 * live updates, paused) and its popover (description, live state, switching,
 * stop confirm, errors, older bridge), the ⋯ "Working mode" row, and the
 * chat list's mode badge.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createStore } from 'solid-js/store';
import { ModeChip, ModePopover } from './ModeChip';
import type { Chat } from '../../lib/chat/store';
import type { LhProgress, LhRun } from '../status/lhProgress';

afterEach(() => {
  cleanup();
  document.body.innerHTML = '';
});

function fakeChat(extra: Record<string, unknown> = {}, setLhMode = vi.fn(async () => {})) {
  const [state, setState] = createStore<Record<string, unknown>>({ conn: 'live', run: { running: false }, work: [], ...extra });
  return { chat: { state, setLhMode } as unknown as Chat, setState, setLhMode };
}
const anchor = new DOMRect(300, 10, 80, 30);
const graph: LhRun = { mode: 'graph', title: 'Repo health', status: 'running', items: [], counts: { total: 7, done: 3, running: 2, failed: 0, queued: 2 } };
const prog = (run: LhRun): LhProgress => ({ v: 1, mode: run.mode, current: run, log: [], updatedAt: 1 });
/** A fresh object per use: a Solid store mutates what it wraps. */
const activeGraph = () => ({ lhMode: { mode: 'graph', owner: { kind: 'graph', status: 'active' } }, lhProgress: prog(structuredClone(graph)) });

describe('ModeChip', () => {
  it('plain mode: no chip', () => {
    const f = fakeChat({ lhMode: { mode: 'none' } });
    render(() => <ModeChip chat={f.chat} onOpen={() => {}} />);
    expect(screen.queryByTestId('mode-chip')).toBeNull();
  });

  it('active graph: name, count, pulse; follows live changes; disappears when turned off', async () => {
    const f = fakeChat(activeGraph());
    const onOpen = vi.fn();
    render(() => <ModeChip chat={f.chat} onOpen={onOpen} />);
    const chip = screen.getByTestId('mode-chip');
    expect(chip.dataset.mode).toBe('graph');
    expect(chip.dataset.phase).toBe('active');
    expect(chip.textContent).toContain('Graph');
    expect(chip.textContent).toContain('3/7');
    expect(chip.querySelector('.mode-chip-pulse')).toBeTruthy();
    expect(chip.getAttribute('aria-label')).toContain('Working mode: Graph');
    fireEvent.click(chip);
    expect(onOpen).toHaveBeenCalledTimes(1);
    f.setState('lhProgress', prog({ ...graph, counts: { ...graph.counts, done: 5 } }));
    await waitFor(() => expect(screen.getByTestId('mode-chip').textContent).toContain('5/7'));
    // Like the store's lh_mode handler: a whole new value (setState would merge).
    f.setState('lhMode', undefined);
    f.setState('lhMode', { mode: 'swarm' });
    await waitFor(() => expect(screen.getByTestId('mode-chip').dataset.mode).toBe('swarm'));
    expect(screen.getByTestId('mode-chip').dataset.phase).toBe('selected');
    f.setState('lhMode', { mode: 'none' });
    await waitFor(() => expect(screen.queryByTestId('mode-chip')).toBeNull());
  });

  it('parked owner: "Goal paused", dashed, no pulse', () => {
    const f = fakeChat({ lhMode: { mode: 'none', paused: 'goal', owner: { kind: 'goal', status: 'parked' } } });
    render(() => <ModeChip chat={f.chat} onOpen={() => {}} />);
    const chip = screen.getByTestId('mode-chip');
    expect(chip.dataset.phase).toBe('paused');
    expect(chip.textContent).toBe('Goalpaused');
    expect(chip.querySelector('.mode-chip-pulse')).toBeNull();
    render(() => <ModePopover anchor={anchor} chat={f.chat} onDismiss={() => {}} />);
    expect(screen.getByTestId('mode-paused-note').textContent).toContain('/unipi:goal resume');
  });
});

describe('ModePopover', () => {
  it('shows the description (the live state lives in the status pane now); current mode is checked', () => {
    const f = fakeChat(activeGraph());
    const onDismiss = vi.fn();
    render(() => <ModePopover anchor={anchor} chat={f.chat} onDismiss={onDismiss} />);
    const pop = screen.getByTestId('mode-popover');
    expect(pop.textContent).toContain('Graph mode');
    expect(pop.textContent).toContain('later steps depend on earlier results');
    expect(screen.queryByTestId('mode-live')).toBeNull();
    expect(screen.getByTestId('mode-pick-graph').getAttribute('aria-checked')).toBe('true');
    expect(screen.getByTestId('mode-pick-goal').textContent).toContain('Pauses the graph');
    expect(screen.queryByText('Show progress')).toBeNull();
  });

  it('switching sends set_lh_mode and closes on ack; tapping the current mode does nothing', async () => {
    const f = fakeChat(activeGraph());
    const onDismiss = vi.fn();
    render(() => <ModePopover anchor={anchor} chat={f.chat} onDismiss={onDismiss} />);
    fireEvent.click(screen.getByTestId('mode-pick-graph'));
    expect(f.setLhMode).not.toHaveBeenCalled();
    fireEvent.click(screen.getByTestId('mode-pick-swarm'));
    expect(f.setLhMode).toHaveBeenCalledWith('swarm');
    await waitFor(() => expect(onDismiss).toHaveBeenCalled());
  });

  it('turning running work off asks for a second tap', async () => {
    const f = fakeChat(activeGraph());
    render(() => <ModePopover anchor={anchor} chat={f.chat} onDismiss={() => {}} />);
    expect(screen.getByTestId('mode-pick-none').textContent).toContain('Stops the graph');
    fireEvent.click(screen.getByTestId('mode-pick-none'));
    expect(f.setLhMode).not.toHaveBeenCalled();
    expect(screen.getByTestId('mode-pick-none').textContent).toContain('Tap again to stop the graph');
    fireEvent.click(screen.getByTestId('mode-pick-none'));
    expect(f.setLhMode).toHaveBeenCalledWith('none');
  });

  it('plain mode: picking a mode needs no confirm', () => {
    const f = fakeChat({ lhMode: { mode: 'none' } });
    render(() => <ModePopover anchor={anchor} chat={f.chat} onDismiss={() => {}} />);
    expect(screen.getByTestId('mode-popover').textContent).toContain('Plain chat');
    expect(screen.queryByText('Show progress')).toBeNull();
    expect(screen.getByTestId('mode-pick-none').getAttribute('aria-checked')).toBe('true');
    fireEvent.click(screen.getByTestId('mode-pick-goal'));
    expect(f.setLhMode).toHaveBeenCalledWith('goal');
  });

  it('a refused switch shows the reason inline and stays open', async () => {
    const f = fakeChat(activeGraph(), vi.fn(async () => Promise.reject(new Error('A goal ("x") is already parked — resume or clear it first.'))));
    const onDismiss = vi.fn();
    render(() => <ModePopover anchor={anchor} chat={f.chat} onDismiss={onDismiss} />);
    fireEvent.click(screen.getByTestId('mode-pick-ralph'));
    await waitFor(() => expect(screen.getByTestId('mode-error').textContent).toContain('already parked'));
    expect(onDismiss).not.toHaveBeenCalled();
  });

  it('older bridge (no lhMode): explains, switching disabled; the chip still shows a running run', () => {
    const f = fakeChat({ lhProgress: prog(graph) });
    render(() => (
      <>
        <ModeChip chat={f.chat} onOpen={() => {}} />
        <ModePopover anchor={anchor} chat={f.chat} onDismiss={() => {}} />
      </>
    ));
    expect(screen.getByTestId('mode-chip').textContent).toContain('Graph');
    expect(screen.getByTestId('mode-unsupported').textContent).toContain('newer unipi');
    expect((screen.getByTestId('mode-pick-goal') as HTMLButtonElement).disabled).toBe(true);
  });

  it('offline: switching disabled', () => {
    const f = fakeChat({ ...activeGraph(), conn: 'connecting' });
    render(() => <ModePopover anchor={anchor} chat={f.chat} onDismiss={() => {}} />);
    expect((screen.getByTestId('mode-pick-swarm') as HTMLButtonElement).disabled).toBe(true);
  });
});
