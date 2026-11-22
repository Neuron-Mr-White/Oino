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
import { COLLAPSED_KEY, DENSITY_KEY, FILTER_KEY, RAIL_KEY, resetCanvasStore } from './canvasStore';

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
  it('titles nodes by role and never says "Agent"', async () => {
    await mount(); const canvas = await openCanvas();
    await screen.findByTestId('fleet-node-lead-oino');
    expect(screen.getByTestId('fleet-node-superagent-chat').getAttribute('aria-label')).toMatch(/^Superagent, /);
    expect(screen.getByTestId('fleet-node-lead-oino').getAttribute('aria-label')).toMatch(/^Lead, Working/);
    expect(screen.getByTestId('fleet-node-sub-oino-core').getAttribute('aria-label')).toMatch(/^explore, /);
    for (const node of canvas.querySelectorAll('.cn')) expect(node.getAttribute('aria-label')).not.toMatch(/^Agent\b/);
  });

  it('groups by project with counts, and has no Ungrouped group', async () => {
    await mount(); await openCanvas();
    await screen.findByTestId('fleet-node-lead-oino');
    expect(screen.getByTestId('fleet-group-oino').textContent).toMatch(/Oino/);
    expect(screen.getByTestId('fleet-group-folder:notes').textContent).toMatch(/No board/);
    expect(screen.getByTestId('fleet-group-other').textContent).toMatch(/Other chats/);
    expect(screen.queryByText('Ungrouped')).toBeNull();
  });

  it('filters with multi-select status chips and persists them', async () => {
    await mount(); const canvas = await openCanvas();
    await screen.findByTestId('fleet-node-lead-oino');
    fireEvent.click(chip(canvas, 'Needs you'));
    expect(JSON.parse(localStorage.getItem(FILTER_KEY)!).statuses).toEqual(['waiting']);
    expect(screen.getByTestId('fleet-node-lead-web')).toBeTruthy();
    expect(screen.queryByTestId('fleet-node-lead-oino')).toBeNull();
    fireEvent.click(chip(canvas, 'Stalled'));
    expect(JSON.parse(localStorage.getItem(FILTER_KEY)!).statuses).toEqual(['waiting', 'stalled']);
    expect(screen.getByTestId('fleet-node-lead-hp')).toBeTruthy();
    fireEvent.click(chip(canvas, 'Needs you')); fireEvent.click(chip(canvas, 'Stalled'));
    expect(JSON.parse(localStorage.getItem(FILTER_KEY)!).statuses).toEqual([]);
    expect(screen.getByTestId('fleet-node-lead-oino')).toBeTruthy();
  });

  it('turns chats off and hides a project from the Project menu', async () => {
    await mount(); const canvas = await openCanvas();
    await screen.findByTestId('fleet-node-chat-oino-auth');
    fireEvent.click(within(canvas).getByRole('button', { name: 'Chats' }));
    expect(JSON.parse(localStorage.getItem(FILTER_KEY)!).chats).toBe(false);
    expect(screen.queryByTestId('fleet-node-chat-oino-auth')).toBeNull();
    fireEvent.click(within(canvas).getByRole('button', { name: /All projects/ }));
    fireEvent.click(await screen.findByRole('menuitemcheckbox', { name: /Website/ }));
    expect(JSON.parse(localStorage.getItem(FILTER_KEY)!).hidden).toEqual(['website']);
    expect(screen.queryByTestId('fleet-node-lead-web')).toBeNull();
    expect(screen.getByText(/\d+ projects hidden/)).toBeTruthy(); // Website, plus the groups with no chats left
    fireEvent.click(screen.getByRole('button', { name: 'Clear filters' }));
    expect(await screen.findByTestId('fleet-node-lead-web')).toBeTruthy();
  });

  it('search dims non-matches and Enter moves on to the next match', async () => {
    await mount(); const canvas = await openCanvas();
    await screen.findByTestId('fleet-node-lead-oino');
    const box = within(canvas).getByRole('searchbox');
    fireEvent.input(box, { target: { value: 'OIN-14' } });
    expect(screen.getByTestId('fleet-node-lead-oino').hasAttribute('data-miss')).toBe(false);
    expect(screen.getByTestId('fleet-node-lead-hp').hasAttribute('data-miss')).toBe(true);
    expect(localStorage.getItem(FILTER_KEY) ?? '').not.toMatch(/OIN-14/);
    fireEvent.keyDown(box, { key: 'Enter' });
    await waitFor(() => expect(screen.getByTestId('fleet-node-lead-oino').hasAttribute('data-current')).toBe(true));
  });

  it('folds 4 or more done siblings into "+N done" and expands them on click', async () => {
    await mount(); await openCanvas();
    await screen.findByTestId('fleet-node-lead-oino');
    expect(screen.queryByTestId('fleet-node-sub-oino-old-0')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: /\+5 done/ }));
    expect(screen.getByTestId('fleet-node-sub-oino-old-0')).toBeTruthy();
    expect(screen.getByRole('button', { name: /Hide done/ })).toBeTruthy();
  });

  it('collapses a group to its header and remembers it', async () => {
    await mount(); const canvas = await openCanvas();
    await screen.findByTestId('fleet-node-lead-oino');
    fireEvent.click(within(canvas).getByLabelText('Collapse Oino'));
    expect(screen.queryByTestId('fleet-node-lead-oino')).toBeNull();
    expect(localStorage.getItem(COLLAPSED_KEY)).toContain('oino');
  });

  it('switches density with the toolbar and with D, and remembers it', async () => {
    await mount(); const canvas = await openCanvas();
    const node = await screen.findByTestId('fleet-node-lead-oino');
    expect(node.getAttribute('data-mode')).toBe('detailed');
    fireEvent.click(within(canvas).getByRole('radio', { name: 'Compact' }));
    expect(screen.getByTestId('fleet-node-lead-oino').getAttribute('data-mode')).toBe('compact');
    expect(localStorage.getItem(DENSITY_KEY)).toBe('compact');
    fireEvent.keyDown(canvas, { key: 'd' });
    expect(screen.getByTestId('fleet-node-lead-oino').getAttribute('data-mode')).toBe('detailed');
    expect(localStorage.getItem(DENSITY_KEY)).toBe('detailed');
  });

  it('zooms, fits and resets with the toolbar and the keyboard', async () => {
    await mount(); const canvas = await openCanvas(); await screen.findByTestId('fleet-node-lead-oino');
    const world = canvas.querySelector<HTMLElement>('.sac-world')!;
    const zoom = within(canvas).getByLabelText('Zoom level');
    fireEvent.click(within(canvas).getByLabelText('Zoom in')); expect(zoom.textContent).toBe('120%');
    fireEvent.click(within(canvas).getByLabelText('Zoom out')); expect(zoom.textContent).toBe('100%');
    fireEvent.keyDown(world, { key: '+' }); expect(zoom.textContent).toBe('120%');
    fireEvent.keyDown(world, { key: '-' }); expect(zoom.textContent).toBe('100%');
    fireEvent.keyDown(world, { key: '+' }); fireEvent.keyDown(world, { key: '0' }); expect(zoom.textContent).toBe('100%');
    fireEvent.click(within(canvas).getByLabelText('Fit to screen'));
  });

  it('opens the peek on click, closes it first on Escape, and opens the chat on double click', async () => {
    await mount(); const canvas = await openCanvas();
    const node = await screen.findByTestId('fleet-node-lead-hp');
    fireEvent.click(node);
    expect(await screen.findByRole('complementary', { name: 'Agent details' })).toBeTruthy();
    expect(screen.getByTestId('peek-health').textContent).toMatch(/Authentication failed/);
    fireEvent.keyDown(canvas, { key: 'Escape' });
    expect(screen.queryByRole('complementary', { name: 'Agent details' })).toBeNull();
    expect(screen.getByRole('dialog', { name: 'Agent canvas' })).toBeTruthy(); // the canvas stays
    fireEvent.dblClick(node);
    await waitFor(() => expect(location.hash).toContain('session=lead-hp'));
  });

  it('shows the lead question in the peek and offers Wake on an idle lead', async () => {
    await mount(); await openCanvas();
    fireEvent.click(await screen.findByTestId('fleet-node-lead-web'));
    const peek = await screen.findByTestId('peek');
    await waitFor(() => expect(peek.textContent).toMatch(/Needs you/));
    cleanupPeek();
    fireEvent.click(screen.getByTestId('fleet-node-lead-orchard'));
    expect(await screen.findByRole('button', { name: /Wake/ })).toBeTruthy();
  });

  it('retries a stalled lead and shows a refusal inline when the host cannot', async () => {
    await mount(); await openCanvas();
    fireEvent.click(await screen.findByTestId('fleet-node-lead-hp'));
    fireEvent.click(within(await screen.findByTestId('peek-health')).getByRole('button', { name: /Retry/ }));
    await waitFor(() => expect(screen.getByTestId('fleet-node-lead-hp').getAttribute('aria-label')).toMatch(/Working/));
  });

  it('says so when an older host answers "unknown method" to a lead action', async () => {
    setHostClientFactory(createMockHost(canvasScenario(3, { leadActionUnsupported: true })).client);
    setPlatformOverride('tauri-desktop'); setDesktopOverride(true);
    location.hash = '#/superagent?session=superagent-chat&host=pc-coffee';
    render(() => <Root />);
    await screen.findByTestId('superagent-page'); await openCanvas();
    fireEvent.click(await screen.findByTestId('fleet-node-lead-hp'));
    fireEvent.click(within(await screen.findByTestId('peek-health')).getByRole('button', { name: /Retry/ }));
    expect((await screen.findByTestId('peek-action-error')).textContent).toMatch(/can't do that yet/);
  });

  it('shows the older-host legend note and groups an old payload', async () => {
    setHostClientFactory(createMockHost(canvasScenario('old')).client);
    setPlatformOverride('tauri-desktop'); setDesktopOverride(true);
    location.hash = '#/superagent?session=superagent-chat&host=pc-coffee';
    render(() => <Root />);
    await screen.findByTestId('superagent-page'); await openCanvas();
    expect(await screen.findByTestId('older-host-note')).toBeTruthy();
    expect(screen.queryByText('Ungrouped')).toBeNull();
  });

  it('keeps focus moves on the node: ArrowDown goes to the first child, ArrowUp to the parent', async () => {
    await mount(); await openCanvas();
    const lead = await screen.findByTestId('fleet-node-lead-oino');
    lead.focus();
    fireEvent.keyDown(lead, { key: 'ArrowDown' });
    await waitFor(() => expect(document.activeElement?.getAttribute('data-id')).toBe('sub-oino-core'));
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowUp' });
    await waitFor(() => expect(document.activeElement?.getAttribute('data-id')).toBe('lead-oino'));
  });

  it('closes using the close toolbar button', async () => {
    await mount(); const canvas = await openCanvas();
    fireEvent.click(within(canvas).getByLabelText('Close canvas'));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Agent canvas' })).toBeNull());
  });

  it('on the phone: compact only, a filter button, and the peek in a sheet', async () => {
    await mount(3, { phone: true });
    fireEvent.click(await screen.findByLabelText('Agent canvas'));
    const canvas = await screen.findByRole('dialog', { name: 'Agent canvas' });
    const node = await screen.findByTestId('fleet-node-lead-oino');
    expect(node.getAttribute('data-mode')).toBe('compact');
    expect(within(canvas).queryByRole('radiogroup', { name: 'Density' })).toBeNull();
    fireEvent.click(within(canvas).getByLabelText('Filters'));
    expect(await screen.findByTestId('filters-sheet')).toBeTruthy();
  });
});

function cleanupPeek() { fireEvent.keyDown(screen.getByTestId('agent-canvas'), { key: 'Escape' }); }
