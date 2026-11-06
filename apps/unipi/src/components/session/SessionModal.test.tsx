/**
 * SessionModal tests (WP-A17 acceptance): each tab renders from MockHost
 * data, Running actions send the right commands, the reserved
 * `unipi.info-screen/dashboard` entry opens the modal (and `$close` answers
 * it), and the Limits section round-trips the watchdog caps services.
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { resetSessions } from '../../lib/session/store';
import { resetUiRegistries, installUiSinks } from '../../ui/registries';
import { setHostClientFactory } from '../../lib/host/factory';
import { setPlatformOverride } from '../../lib/platform';
import { setDesktopOverride } from '../../lib/layout';
import { createMockHost } from '../../dev/mock/host';
import { SCENARIOS } from '../../dev/mock/scenarios';
import { closeSessionModal, openSessionModal, SessionModalLayer, sessionModal } from './SessionModal';
import { breakdownSegments, cacheHitRate, CACHE_HIT_GOOD } from './ContextTab';
import { fetchRunning } from './RunningTab';
import { pinsOf } from './LimitsSection';

const tick = () => new Promise((r) => setTimeout(r, 0));

/** Boots a modal scenario and renders the layer (desktop shell). */
async function boot(name: 'modal-context' | 'modal-running' | 'modal-info' | 'modal-limits') {
  const scenario = SCENARIOS[name]!;
  const world = createMockHost({ ...scenario, speed: 0 });
  setHostClientFactory(world.client);
  // The shell installs the U9 sinks when it mounts, before any session opens; the
  // `modal-info` entry rides the session's first ui snapshot, so wire them first.
  installUiSinks();
  scenario.setup?.();
  await tick();
  const ui = render(() => <SessionModalLayer />);
  await waitFor(() => expect(sessionModal()).toBeDefined());
  return { world, ...ui };
}

beforeEach(() => {
  setPlatformOverride('tauri-desktop');
  setDesktopOverride(true);
});

afterEach(() => {
  cleanup();
  closeSessionModal();
  resetSessions();
  resetUiRegistries();
  setHostClientFactory(undefined);
  window.localStorage.clear();
});

describe('pure helpers', () => {
  it('cache hit rate and breakdown segments', () => {
    const tokens = { input: 10, output: 5, cacheRead: 30, cacheWrite: 0, total: 45 };
    expect(cacheHitRate(tokens)).toBeCloseTo(0.75);
    expect(cacheHitRate(tokens)! < CACHE_HIT_GOOD).toBe(true);
    const good = { input: 2, output: 0, cacheRead: 98, cacheWrite: 0, total: 100 };
    expect(cacheHitRate(good)! >= CACHE_HIT_GOOD).toBe(true);
    expect(cacheHitRate({ ...tokens, input: 0, cacheRead: 0 })).toBeUndefined();
    expect(breakdownSegments(tokens).map((s) => s.key)).toEqual(['in', 'out', 'read']);
  });

  it('pinsOf turns empty fields into nulls and numbers into pins', () => {
    expect(pinsOf({ maxTurns: '40', maxCostUsd: '', maxWallMinutes: '2.5', maxConsecutiveToolErrors: 'x' }, false)).toEqual({
      maxTurns: 40,
      maxCostUsd: null,
      maxWallMinutes: 2.5,
      maxConsecutiveToolErrors: null,
      summaryTurn: false,
    });
  });
});

describe('Context tab', () => {
  it('renders usage, stats and cache hit from the mock host', async () => {
    const { world } = await boot('modal-context');
    await waitFor(() => expect(screen.getByTestId('sm-context').textContent).toContain('42%'));
    expect(screen.getByTestId('sm-context').textContent).toContain('84.2k / 200.0k tokens');
    expect(screen.getByText('81%').tagName).toBe('SPAN'); // 52300 / (12400+52300) cache hit
    expect(world.log.some((l) => l.op === '_oino/get_context_usage')).toBe(true);
    expect(world.log.some((l) => l.op === 'get_session_stats')).toBe(true);
  });

  it('Compact now sends the compact command', async () => {
    const { world } = await boot('modal-context');
    const btn = await screen.findByTestId('sm-compact');
    fireEvent.click(btn);
    await waitFor(() => expect(world.log.some((l) => l.op === 'compact')).toBe(true));
  });
});

describe('Running tab', () => {
  it('shows bash, ONE subagents list, sidekick and the other work-tray trees', async () => {
    await boot('modal-running');
    await waitFor(() => expect(screen.getByTestId('sm-running')).toBeTruthy());
    // The status filter opens on "Running" (owner 2026-11-06); show every row below.
    expect((await screen.findByTestId('sm-filter-running')).getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(screen.getByTestId('sm-filter-all'));
    expect(await screen.findByText('cargo test -p unipi-watchdog')).toBeTruthy();
    expect(await screen.findByText('explore the layout code')).toBeTruthy();
    expect(await screen.findByText(/Handoff running/)).toBeTruthy();
    // Background tasks are the native section (topic rows), not the extension's tray tree.
    expect((await screen.findByTestId('sm-sec-bg')).textContent).toContain('Background tasks');
    expect(screen.queryByTestId('sm-tray-bg')).toBeNull();
    expect(screen.getByTestId('sm-tray-dream').textContent).toContain('Dream');
    // The subagents producer's TUI pane is never listed: one row per run, no 'RUN ' badge text.
    expect(screen.queryByTestId('sm-tray-subagents')).toBeNull();
    const rows = screen.getAllByTestId('sm-run-row');
    // 5 records + the btw side agent; the records' own host agents (ag-1, ag-2, ag-3) are not extra rows.
    expect(rows.map((r) => r.getAttribute('data-run'))).toEqual(['agent:btw-1', 'run:b2c3d4e5', 'run:a1b2c3d4', 'run:c3d4e5f6', 'run:d4e5f6a7', 'run:e5f6a7b8']);
    const body = screen.getByTestId('sm-running').textContent ?? '';
    expect(body).not.toMatch(/RUN |DONE|FAIL|STOP/);
    expect(body).not.toContain('Subagent · explore the layout code');
    // Status from the structured record status, AgentCard icons.
    expect(rows.map((r) => r.getAttribute('data-status'))).toEqual(['running', 'running', 'running', 'done', 'failed', 'cancelled']);
    // Profile chip, short model (full id in the tooltip), stats on their own line.
    const done = rows[3]!;
    expect(done.querySelector('.sm-run-chip')!.textContent).toBe('General');
    expect(done.querySelector('.sm-run-model')!.getAttribute('title')).toBe('gateway/cc/claude-opus-5-5');
    expect(done.querySelector('.sm-run-model')!.textContent).not.toContain('/');
    expect([...done.querySelectorAll('.sm-run-stat')].map((e) => e.textContent)).toEqual(['122 tools', '238 in · 88k out', '$14.23']);
    expect(done.querySelector('.sm-elapsed')!.textContent).toBe('26m 36s');
  });

  it('status chips filter the list (All / Running / Done / Failed, Cancelled when present)', async () => {
    await boot('modal-running');
    // The filter opens on "Running": the three runs that are still going.
    await waitFor(() => expect(screen.getAllByTestId('sm-run-row').length).toBe(3));
    expect(screen.getByTestId('sm-filter-all').textContent).toBe('All6');
    expect(screen.getByTestId('sm-filter-running').textContent).toBe('Running3');
    fireEvent.click(screen.getByTestId('sm-filter-failed'));
    await waitFor(() => expect(screen.getAllByTestId('sm-run-row').map((r) => r.getAttribute('data-run'))).toEqual(['run:d4e5f6a7']));
    expect(screen.getByTestId('sm-filter-failed').getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(screen.getByTestId('sm-filter-cancelled'));
    await waitFor(() => expect(screen.getAllByTestId('sm-run-row').map((r) => r.getAttribute('data-run'))).toEqual(['run:e5f6a7b8']));
    fireEvent.click(screen.getByTestId('sm-filter-all'));
    await waitFor(() => expect(screen.getAllByTestId('sm-run-row').length).toBe(6));
  });

  it('actions send the right commands', async () => {
    const { world } = await boot('modal-running');
    await waitFor(() => expect(screen.getByTestId('sm-running')).toBeTruthy());
    // Every row, not just the running ones (the filter opens on "Running").
    fireEvent.click(await screen.findByTestId('sm-filter-all'));

    // Foreground bash → the run's Stop (pi `abort`).
    fireEvent.click(await screen.findByText('Stop run'));
    await waitFor(() => expect(world.log.some((l) => l.op === 'abort')).toBe(true));

    // A record run → `subagents.cancel{id}`; To background → `subagents.background{id}` (that run only).
    const row = () => screen.getAllByTestId('sm-run-row').find((r) => r.getAttribute('data-run') === 'run:a1b2c3d4')!;
    await waitFor(() => expect(row()).toBeTruthy());
    fireEvent.click(row().querySelector('button[aria-label="Stop"]')!);
    await waitFor(() => {
      const cmd = world.log.find((l) => l.op === '_oino/service_call' && (l.args as { name?: string }).name === 'subagents.cancel');
      expect(cmd?.args).toMatchObject({ target: 'unipi.subagents', payload: { id: 'a1b2c3d4' } });
    });
    fireEvent.click(row().querySelector('button[aria-label="To background"]')!);
    await waitFor(() => {
      const cmd = world.log.find((l) => l.op === '_oino/service_call' && (l.args as { name?: string }).name === 'subagents.background');
      expect(cmd?.args).toMatchObject({ target: 'unipi.subagents', payload: { id: 'a1b2c3d4' } });
    });
    // A background run has no To background; a finished one has no Stop.
    const bg = screen.getAllByTestId('sm-run-row').find((r) => r.getAttribute('data-run') === 'run:b2c3d4e5')!;
    expect(bg.querySelector('button[aria-label="To background"]')).toBeNull();
    const fin = screen.getAllByTestId('sm-run-row').find((r) => r.getAttribute('data-run') === 'run:c3d4e5f6')!;
    expect(fin.querySelector('button[aria-label="Stop"]')).toBeNull();
    expect(fin.querySelector('button[aria-label="Open transcript"]')).toBeTruthy();
    // A host-only agent → `_oino/agent_abort{agentId}`.
    const btw = screen.getAllByTestId('sm-run-row').find((r) => r.getAttribute('data-run') === 'agent:btw-1')!;
    fireEvent.click(btw.querySelector('button[aria-label="Stop"]')!);
    await waitFor(() => {
      const cmd = world.log.find((l) => l.op === '_oino/agent_abort');
      expect(cmd?.args).toMatchObject({ type: '_oino/agent_abort', agentId: 'btw-1' });
    });
    // Running bash → Send to background = the watchdog's `/bg-detach` command.
    fireEvent.click(await screen.findByText('Send to background'));
    await waitFor(() => {
      const cmd = world.log.find((l) => l.op === 'prompt');
      expect((cmd?.args as { message?: string }).message).toBe('/bg-detach');
    });

    fireEvent.click(screen.getByText('All to background'));
    await waitFor(() => {
      const cmd = world.log.find((l) => l.op === '_oino/invoke_shortcut');
      expect(cmd?.args).toMatchObject({ shortcutId: 'unipi.subagents.foreground' });
    });

    // Bg row Stop → `work-tray.action` with the owner target and the `<tab>:` prefix stripped.
    fireEvent.click((await screen.findByTestId('sm-sec-bg')).querySelector('[data-testid="sm-bg-row"] button[data-tone="danger"]')!);
    await waitFor(() => {
      const cmd = world.log.filter((l) => l.op === '_oino/service_call').at(-1);
      expect(cmd?.args).toMatchObject({ type: '_oino/service_call', target: 'unipi.background-tasks', name: 'work-tray.action', payload: { tab: 'bg', actionId: 'stop:bg-1', value: null } });
    });
  });

  it('fetchRunning: records from subagents.list, work-tray.tab from every other loaded extension', async () => {
    const world = createMockHost({ ...SCENARIOS['modal-running']!, speed: 0 });
    setHostClientFactory(world.client);
    const { sessions } = await import('../../lib/session/store');
    const s = sessions().ensure('this-pc', 's001');
    await tick();
    const data = await fetchRunning(s);
    expect(data.tabs.map((t) => t.id).sort()).toEqual(['bg', 'dream']);
    expect(data.runs?.map((r) => r.status)).toEqual(['running', 'running', 'completed', 'failed', 'cancelled']);
    // The subagents tray pane (TUI text, reads every transcript) is not asked for.
    expect(world.log.some((l) => l.op === '_oino/service_call' && (l.args as { name?: string; target?: string }).name === 'work-tray.tab' && (l.args as { target?: string }).target === 'unipi.subagents')).toBe(false);
    expect(data.agents).toHaveLength(4);
    expect(data.bash[0]!.pid).toBe(4242);
    expect(data.sidekick?.detail).toBe('Handoff running');
    resetSessions();
  });
});

describe('reserved entry (/info)', () => {
  it('the snapshot overlay entry opens the modal on its page; closing sends $close', async () => {
    const { world } = await boot('modal-info');
    // The entry params say page usage: cross-session usage lives in Settings now, the Session page shows this session's.
    await waitFor(() => expect(screen.getByTestId('sm-info')).toBeTruthy());
    expect(screen.queryByTestId('sm-page-usage')).toBeNull();
    expect(screen.getByTestId('sm-page-session').getAttribute('aria-selected')).toBe('true');
    expect(screen.getByTestId('sm-info-all-usage')).toBeTruthy();
    expect(sessionModal()?.entryId).toBe('unipi.info-screen/dashboard');

    fireEvent.click(screen.getByTestId('sm-close'));
    await waitFor(() => {
      const cmd = world.log.filter((l) => l.op === '_oino/ui_action').at(-1);
      expect(cmd?.args).toMatchObject({ entryId: 'unipi.info-screen/dashboard', actionId: '$close' });
    });
    expect(sessionModal()).toBeUndefined();
  });

  it('Info pages render stats, commands, skills and extensions', async () => {
    await boot('modal-info');
    await waitFor(() => expect(screen.getByTestId('sm-info')).toBeTruthy());
    // Usage page (the entry's param).
    expect(screen.getByTestId('sm-info').textContent).toContain('Sessions');
    // Session page
    fireEvent.click(screen.getByTestId('sm-page-session'));
    await waitFor(() => expect(screen.getByTestId('sm-info').textContent).toContain('claude-sonnet-4-5'));
    // Tools page: commands grouped, skills excluded there
    fireEvent.click(screen.getByTestId('sm-page-tools'));
    await waitFor(() => expect(screen.getByTestId('sm-info').textContent).toContain('/compact'));
    // Skills page: the three skills with their groups
    fireEvent.click(screen.getByTestId('sm-page-skills'));
    await waitFor(() => expect(screen.getByTestId('sm-info').textContent).toContain('refactor-plan'));
    expect(screen.getByTestId('sm-info').textContent).toContain('package');
    // Modules page
    fireEvent.click(screen.getByTestId('sm-page-modules'));
    await waitFor(() => expect(screen.getByTestId('sm-info').textContent).toContain('unipi.watchdog'));
  });
});

describe('Limits (G-16)', () => {
  it('saves pins through caps.set and reflects them', async () => {
    const { world } = await boot('modal-limits');
    const input = await screen.findByLabelText('Max turns value');
    fireEvent.input(input, { target: { value: '25' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    fireEvent.click(screen.getByTestId('sm-limits-save'));
    await waitFor(() => {
      const cmd = world.log.filter((l) => l.op === '_oino/service_call').at(-1);
      expect(cmd?.args).toMatchObject({ type: '_oino/service_call', target: 'unipi.watchdog', name: 'caps.set' });
      expect((cmd?.args as { payload?: Record<string, unknown> }).payload).toMatchObject({ maxTurns: 25, maxCostUsd: null, summaryTurn: true });
    });
    await waitFor(() => expect(screen.getByText('set for this session')).toBeTruthy());
  });

  it('hides when the watchdog service is not there', async () => {
    // modal-context answers caps.get with E_NOT_FOUND: no dead UI.
    await boot('modal-context');
    fireEvent.click(screen.getByTestId('sm-tab-info'));
    await tick();
    await waitFor(() => expect(screen.getByTestId('sm-info')).toBeTruthy());
    expect(screen.queryByTestId('sm-limits')).toBeNull();
  });
});

describe('openSessionModal', () => {
  it('opens on the asked tab (the A14/A16/A18 entry point)', async () => {
    const world = createMockHost({ ...SCENARIOS['modal-context']!, speed: 0 });
    setHostClientFactory(world.client);
    render(() => <SessionModalLayer />);
    openSessionModal({ hostId: 'this-pc', sessionId: 's001', tab: 'running' });
    await waitFor(() => expect(sessionModal()?.tab).toBe('running'));
    expect(screen.getByTestId('sm-tab-running').getAttribute('aria-selected')).toBe('true');
  });
});

describe('Running tab badge and empty states ([ui-chat-bits])', () => {
  it('nothing running: no badge, one quiet "Nothing running" line instead of empty sections', async () => {
    const world = createMockHost({ ...SCENARIOS['modal-context']!, speed: 0 });
    setHostClientFactory(world.client);
    render(() => <SessionModalLayer />);
    openSessionModal({ hostId: 'this-pc', sessionId: 's001', tab: 'running' });
    await waitFor(() => expect(screen.getByTestId('sm-running')).toBeTruthy());
    await waitFor(() => expect(screen.getByTestId('sm-nothing').textContent).toContain('Nothing running'));
    expect(screen.queryByTestId('sm-running-badge')).toBeNull();
    expect(screen.queryByTestId('sm-sec-bash')).toBeNull();
    expect(screen.queryByTestId('sm-sec-agents')).toBeNull();
    expect(screen.queryByTestId('sm-sec-tray')).toBeNull();
  });

  it('running work: the badge counts it (shell + live runs + sidekick + running tray)', async () => {
    await boot('modal-running');
    const badge = await screen.findByTestId('sm-running-badge');
    // 1 shell + 3 live runs (2 records + btw) + the fusion sidekick + 1 running tray tab.
    await waitFor(() => expect(Number(badge.textContent)).toBe(6));
    expect(badge.getAttribute('aria-label')).toBe(`${badge.textContent} running`);
    expect(screen.getByTestId('sm-sec-bash').textContent).toContain('Started');
    expect(screen.queryByTestId('sm-nothing')).toBeNull();
  });

  it('startedText: time today, date + time otherwise', async () => {
    const { startedText } = await import('./RunningTab');
    const now = new Date(2026, 10, 2, 12, 0).getTime();
    expect(startedText(new Date(2026, 10, 2, 9, 5).getTime(), now)).toMatch(/09:05/);
    expect(startedText(new Date(2026, 10, 1, 19, 40).getTime(), now)).toMatch(/Nov 1.*19:40/);
  });
});
