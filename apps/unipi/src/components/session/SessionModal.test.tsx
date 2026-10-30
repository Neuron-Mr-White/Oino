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
  it('shows bash, subagents, sidekick and the work-tray trees', async () => {
    await boot('modal-running');
    await waitFor(() => expect(screen.getByTestId('sm-running')).toBeTruthy());
    expect(await screen.findByText('cargo test -p unipi-watchdog')).toBeTruthy();
    expect(await screen.findByText('Subagent · explore the layout code')).toBeTruthy();
    expect(await screen.findByText(/Handoff running/)).toBeTruthy();
    expect((await screen.findByTestId('sm-tray-bg')).textContent).toContain('Background tasks');
    expect(screen.getByTestId('sm-tray-dream').textContent).toContain('Dream');
  });

  it('actions send the right commands', async () => {
    const { world } = await boot('modal-running');
    await waitFor(() => expect(screen.getByTestId('sm-running')).toBeTruthy());

    // Foreground bash → the run's Stop (pi `abort`).
    fireEvent.click(await screen.findByText('Stop run'));
    await waitFor(() => expect(world.log.some((l) => l.op === 'abort')).toBe(true));

    // Subagent → `_oino/agent_abort{agentId}`; all to background → the shortcut.
    const agentRow = await screen.findByText('Subagent · explore the layout code');
    fireEvent.click(agentRow.closest('.sm-row')!.querySelector('button')!);
    await waitFor(() => {
      const cmd = world.log.find((l) => l.op === '_oino/agent_abort');
      expect(cmd?.args).toMatchObject({ type: '_oino/agent_abort', agentId: 'ag-1' });
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

    // Tray tree action (bg Stop) → `work-tray.action` with the owner target
    // and the `<tab>:` prefix stripped.
    fireEvent.click(await screen.findByText('Re-run'));
    await waitFor(() => {
      const cmd = world.log.filter((l) => l.op === '_oino/service_call').at(-1);
      expect(cmd?.args).toMatchObject({ type: '_oino/service_call', target: 'unipi.background-tasks', name: 'work-tray.action', payload: { tab: 'bg', actionId: 'rerun:bg-2', value: null } });
    });
  });

  it('fetchRunning fans work-tray.tab out to every loaded extension', async () => {
    const world = createMockHost({ ...SCENARIOS['modal-running']!, speed: 0 });
    setHostClientFactory(world.client);
    const { sessions } = await import('../../lib/session/store');
    const s = sessions().ensure('this-pc', 's001');
    await tick();
    const data = await fetchRunning(s);
    expect(data.tabs.map((t) => t.id).sort()).toEqual(['bg', 'dream']);
    expect(data.agents).toHaveLength(2);
    expect(data.bash[0]!.pid).toBe(4242);
    expect(data.sidekick?.detail).toBe('Handoff running');
    resetSessions();
  });
});

describe('reserved entry (/info)', () => {
  it('the snapshot overlay entry opens the modal on its page; closing sends $close', async () => {
    const { world } = await boot('modal-info');
    // The entry params say page usage.
    await waitFor(() => expect(screen.getByTestId('sm-info')).toBeTruthy());
    expect(screen.getByTestId('sm-page-usage').getAttribute('aria-selected')).toBe('true');
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
    const input = await screen.findByLabelText('Max turns');
    fireEvent.input(input, { target: { value: '25' } });
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
