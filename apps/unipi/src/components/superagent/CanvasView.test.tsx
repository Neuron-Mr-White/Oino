// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setHostClientFactory } from '../../lib/host/factory';
import { createMockHost } from '../../dev/mock/host';
import { canvasScenario } from '../../dev/mock/scenarios/superagent-canvas';
import { Root } from '../../main';
import { resetSessions } from '../../lib/session/store';
import { resetHubs } from '../../lib/session/hub';
import { resetLayout, setDesktopOverride } from '../../lib/layout';
import { setPlatformOverride } from '../../lib/platform';
import { FleetSchema, superagentApi } from './api';
import { DISPLAY_KEY, FILTER_KEY, RAIL_KEY, resetCanvasStore } from './canvasStore';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri')), transformCallback: () => 0 }));

beforeEach(() => { resetCanvasStore(); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); setHostClientFactory(undefined); resetSessions(); resetHubs(); resetLayout(); setDesktopOverride(undefined); setPlatformOverride(undefined); resetCanvasStore(); });

async function mount(fleet = 3, opts: { phone?: boolean; hash?: string } = {}) {
  setHostClientFactory(createMockHost(canvasScenario(fleet)).client);
  setPlatformOverride(opts.phone ? 'tauri-mobile' : 'tauri-desktop'); setDesktopOverride(!opts.phone);
  location.hash = opts.hash ?? '#/superagent?session=superagent-chat&host=pc-coffee';
  render(() => <Root />);
  await screen.findByTestId('superagent-page');
}
const openCanvas = async () => { fireEvent.click(await screen.findByLabelText('Agent canvas')); return screen.findByRole('dialog', { name: 'Agent canvas' }); };
const chip = (canvas: HTMLElement, name: string) => within(canvas).getByRole('button', { name: new RegExp(`^${name}`) });
const nodeIds = () => screen.queryAllByTestId(/^fleet-node-/).map((el) => el.getAttribute('data-testid')!.slice('fleet-node-'.length));

describe('Superagent right rail', () => {
  it('collapses to a slim rail, keeps the conversation, and remembers it', async () => {
    await mount();
    const toggle = await screen.findByLabelText('Collapse panel');
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    const side = screen.getByTestId('sa-side');
    const center = document.querySelector('.sa-center')!;
    expect(side.hasAttribute('data-collapsed')).toBe(false);
    fireEvent.click(toggle);
    expect(side.hasAttribute('data-collapsed')).toBe(true);
    expect(document.querySelector('.sa-center')).toBe(center); // same DOM: no remount, no transcript jump
    expect(screen.getByLabelText('Expand panel').getAttribute('aria-expanded')).toBe('false');
    expect(localStorage.getItem(RAIL_KEY)).toBe('1');
    expect(screen.queryByTestId('sa-slim-needs')).toBeNull();
    expect(side.querySelector('.sa-side-body')!.hasAttribute('inert')).toBe(true);
    // Mounted again: read from storage.
    cleanup();
    await mount();
    expect(screen.getByTestId('sa-side').hasAttribute('data-collapsed')).toBe(true);
    fireEvent.click(await screen.findByLabelText('Expand panel'));
    expect(screen.getByTestId('sa-side').hasAttribute('data-collapsed')).toBe(false);
    expect(localStorage.getItem(RAIL_KEY)).toBe('0');
    expect(screen.queryByTestId('sa-slim-needs')).toBeNull();
  });

  it('shows no rail toggle on the phone', async () => {
    await mount(3, { phone: true });
    await screen.findByTestId('sa-sheet-open');
    expect(screen.queryByLabelText('Collapse panel')).toBeNull();
    expect(screen.queryByLabelText('Expand panel')).toBeNull();
    expect(screen.queryByTestId('sa-side')).toBeNull();
  });
});

describe('Agent canvas', () => {
  it('opens on Working with live counts, records canvas=1, and closes on Escape', async () => {
    await mount();
    const canvas = await openCanvas();
    expect(location.hash).toContain('canvas=1');
    expect(canvas.getAttribute('aria-modal')).toBe('true');
    await waitFor(() => expect(chip(canvas, 'Working').textContent).toBe('Working8'));
    expect(chip(canvas, 'Working').getAttribute('aria-pressed')).toBe('true');
    expect(chip(canvas, 'Waiting').textContent).toBe('Waiting1');
    expect(chip(canvas, 'Idle').textContent).toBe('Idle3');
    expect(chip(canvas, 'Ended').textContent).toBe('Ended1');
    expect(chip(canvas, 'All').textContent).toBe('All13');
    // Working nodes, plus the ancestors that keep their tree connected: the waiting node's lead is already working.
    expect(nodeIds().sort()).toEqual(['lead-hp', 'lead-oino', 'lead-web', 'sub-oino-core', 'sub-oino-migrate', 'sub-oino-tests', 'sub-orphan', 'sub-web-pricing'].sort());
    expect(screen.getByTestId('fleet-region-oino')).toBeTruthy();
    expect(screen.getByTestId('fleet-region-Ungrouped')).toBeTruthy();
    expect(within(screen.getByTestId('fleet-node-lead-oino')).getByText('Graph')).toBeTruthy();
    expect(within(screen.getByTestId('fleet-node-lead-web')).getByText('Swarm')).toBeTruthy();
    expect(within(screen.getByTestId('fleet-node-lead-hp')).getByText('Plain')).toBeTruthy();
    fireEvent.keyDown(canvas, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Agent canvas' })).toBeNull());
    expect(location.hash).not.toContain('canvas=1');
  });

  it('restores from the URL', async () => {
    await mount(3, { hash: '#/superagent?session=superagent-chat&host=pc-coffee&canvas=1' });
    expect(await screen.findByRole('dialog', { name: 'Agent canvas' })).toBeTruthy();
  });

  it('filters, keeps ancestors dimmed, and remembers the filter', async () => {
    await mount();
    const canvas = await openCanvas();
    await waitFor(() => expect(nodeIds().length).toBeGreaterThan(0));
    fireEvent.click(chip(canvas, 'Waiting'));
    expect(localStorage.getItem(FILTER_KEY)).toBe('waiting');
    expect(nodeIds().sort()).toEqual(['lead-web', 'sub-web-deploy']);
    expect(screen.getByTestId('fleet-node-lead-web').hasAttribute('data-dim')).toBe(true);
    expect(screen.getByTestId('fleet-node-sub-web-deploy').hasAttribute('data-dim')).toBe(false);
    expect(screen.queryByTestId('fleet-region-oino')).toBeNull();
    fireEvent.click(chip(canvas, 'All'));
    expect(nodeIds()).toHaveLength(13);
  });

  it('the Display menu hides and shows fields, and persists them', async () => {
    await mount();
    const canvas = await openCanvas();
    const node = await screen.findByTestId('fleet-node-lead-oino');
    expect(within(node).getByText('GPT 5')).toBeTruthy();
    expect(within(node).getByText('Working')).toBeTruthy();
    fireEvent.click(within(canvas).getByRole('button', { name: /Display/ }));
    const menu = screen.getByRole('dialog', { name: 'Display' });
    expect(within(menu).getAllByRole('checkbox')).toHaveLength(3);
    fireEvent.click(within(menu).getByLabelText('Model name'));
    expect(within(node).queryByText('GPT 5')).toBeNull();
    expect(JSON.parse(localStorage.getItem(DISPLAY_KEY)!)).toEqual({ name: true, model: false, status: true });
    fireEvent.click(within(menu).getByLabelText('Status'));
    expect(within(node).queryByText('Working')).toBeNull();
    fireEvent.click(within(menu).getByLabelText('Agent name'));
    expect(within(node).queryByText('Lead · Oino')).toBeNull();
    // Escape closes the menu first, then the canvas.
    fireEvent.keyDown(menu, { key: 'Escape' });
    expect(screen.queryByRole('dialog', { name: 'Display' })).toBeNull();
    expect(screen.getByRole('dialog', { name: 'Agent canvas' })).toBeTruthy();
  });

  it('clicking a node opens its session, closes the canvas, and Back to canvas returns to the same filter', async () => {
    await mount();
    const canvas = await openCanvas();
    fireEvent.click(chip(canvas, 'Idle'));
    await screen.findByTestId('fleet-node-lead-orchard');
    fireEvent.click(screen.getByTestId('fleet-node-lead-orchard'));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Agent canvas' })).toBeNull());
    await waitFor(() => expect(location.hash).toContain('session=lead-orchard'));
    expect(location.hash).not.toContain('canvas=1');
    fireEvent.click(await screen.findByRole('button', { name: 'Back to canvas' }));
    const again = await screen.findByRole('dialog', { name: 'Agent canvas' });
    expect(chip(again, 'Idle').getAttribute('aria-pressed')).toBe('true');
    expect(screen.queryByRole('button', { name: 'Back to canvas' })).toBeNull();
  });

  it('a subagent opens the lead above it', async () => {
    await mount();
    await openCanvas();
    fireEvent.click(await screen.findByTestId('fleet-node-sub-web-pricing'));
    await waitFor(() => expect(location.hash).toContain('session=lead-web'));
  });

  it('shows the empty state, and Show all switches to All', async () => {
    await mount(0);
    const canvas = await openCanvas();
    expect(await screen.findByText('No agents working')).toBeTruthy();
    expect(chip(canvas, 'Working').textContent).toBe('Working0');
    fireEvent.click(screen.getByRole('button', { name: 'Show all' }));
    expect(chip(canvas, 'All').getAttribute('aria-pressed')).toBe('true');
    expect(screen.queryByRole('button', { name: 'Show all' })).toBeNull();
  });

  it('says so when a filter matches nothing, and Show all brings the agents back', async () => {
    await mount(1);
    const canvas = await openCanvas();
    await waitFor(() => expect(nodeIds().length).toBeGreaterThan(0));
    fireEvent.click(chip(canvas, 'Waiting'));
    expect(screen.getByText('No agents match this filter')).toBeTruthy();
    expect(nodeIds()).toEqual([]);
    fireEvent.click(screen.getByRole('button', { name: 'Show all' }));
    expect(nodeIds()).toHaveLength(7);
  });

  it('zoom buttons stay within 25% to 200%; Reset goes back to 100%', async () => {
    await mount();
    const canvas = await openCanvas();
    const zoom = within(canvas).getByLabelText('Zoom level');
    for (let i = 0; i < 12; i++) fireEvent.click(within(canvas).getByLabelText('Zoom in'));
    expect(zoom.textContent).toBe('200%');
    for (let i = 0; i < 20; i++) fireEvent.click(within(canvas).getByLabelText('Zoom out'));
    expect(zoom.textContent).toBe('25%');
    fireEvent.click(within(canvas).getByLabelText('Reset view'));
    expect(zoom.textContent).toBe('100%');
  });

  it('picks up the next poll: a new agent appears, a finished one leaves, no duplicates', async () => {
    await mount();
    const canvas = await openCanvas();
    await screen.findByTestId('fleet-node-lead-hp');
    const next = { nodes: [{ id: 'lead-hp', project: 'Heartpick', projectId: 'heartpick', title: 'Lead · Heartpick', mode: 'plain', role: 'lead', status: 'working' }, { id: 'lead-hp', project: 'Heartpick', projectId: 'heartpick', title: 'Lead · Heartpick', mode: 'plain', role: 'lead', status: 'working' }, { id: 'fresh', parentId: 'lead-hp', project: 'Heartpick', projectId: 'heartpick', title: 'Fresh helper', mode: 'plain', role: 'subagent', status: 'working' }] };
    vi.spyOn(superagentApi, 'fleet').mockResolvedValue({ ok: true, value: FleetSchema.parse(next) });
    fireEvent.click(screen.getByLabelText('Refresh')); // the same refresh the 5 s poll runs
    await waitFor(() => expect(nodeIds().sort()).toEqual(['fresh', 'lead-hp']));
    expect(chip(canvas, 'Working').textContent).toBe('Working2');
  });
});
