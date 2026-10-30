// @vitest-environment jsdom
/**
 * The A16 status sections (WP-A16 acceptance): the LH section follows owner
 * enablement (LH off ⇒ gone), the Plan & todos section renders the todowrite
 * checklist + plan mode, the Running summary's counts equal the Session
 * modal's (shared selectors over the same mock data) and the capsule shows
 * branch ± / LH / running and opens the pane.
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { setHostClientFactory } from '../../lib/host/factory';
import { resetSessions } from '../../lib/session/store';
import { resetHubs } from '../../lib/session/hub';
import { setDesktopOverride, resetLayout } from '../../lib/layout';
import { createMockHost, type MockScenario } from '../../dev/mock/host';
import { SCENARIOS } from '../../dev/mock/scenarios';
import { StatusSections } from './StatusPane';
import { Capsule } from './Capsule';
import { fetchRunning } from '../session/RunningTab';
import { openSessionModal, sessionModal } from '../session/modalTarget';
import { runningCounts, runningTotal } from '../../lib/status/running';
import { sessions } from '../../lib/session/store';
const tick = () => new Promise((r) => setTimeout(r, 0));

/** A world booted from a scenario (desktop shell). */
async function boot(name: keyof typeof SCENARIOS | MockScenario) {
  const scenario = typeof name === 'string' ? SCENARIOS[name]! : name;
  const world = createMockHost({ ...scenario, speed: 0 });
  setHostClientFactory(world.client);
  scenario.setup?.();
  await tick();
  return world;
}

/** The same scenario without long-horizon in the extension list (LH off). */
function withoutLh(scenario: MockScenario): MockScenario {
  return {
    ...scenario,
    hosts: scenario.hosts.map((h) => ({
      ...h,
      sessions: h.sessions && Object.fromEntries(Object.entries(h.sessions).map(([id, s]) => [id, { ...s, cmd: { ...s.cmd, '_oino/extensions': () => ({ extensions: [] }) } }])),
    })),
  };
}

beforeEach(() => {
  setDesktopOverride(true);
});

afterEach(() => {
  cleanup();
  resetSessions();
  resetHubs();
  resetLayout();
  setHostClientFactory(undefined);
  window.localStorage.clear();
});

describe('LH section', () => {
  it('renders from the LH topics (graph counts, task, phase)', async () => {
    await boot('status-lh-graph');
    render(() => <StatusSections hostId="this-pc" sessionId="s001" />);
    await waitFor(() => expect(screen.getByTestId('lh-section')).toBeTruthy());
    expect(screen.getByTestId('lh-phase').textContent).toBe('Running');
    expect(screen.getByTestId('lh-section').textContent).toContain('Land the status pane sections');
    expect(screen.getByTestId('lh-graph').textContent).toContain('4/7 settled');
    expect(screen.getByTestId('lh-section').textContent).toContain('2 running');
    expect(screen.getByTestId('lh-history')).toBeTruthy();
  });

  it('LH disabled ⇒ the section is gone (owner enablement)', async () => {
    const world = await boot(withoutLh(SCENARIOS['status-lh-graph']!));
    render(() => <StatusSections hostId="this-pc" sessionId="s001" />);
    await tick();
    await waitFor(() => expect(world.log.some((l) => l.op === '_oino/extensions')).toBe(true));
    expect(screen.queryByTestId('lh-section')).toBeNull();
    // the other sections still render (the pane is not empty)
    expect(screen.getByTestId('running-summary')).toBeTruthy();
  });

  it('Stop asks for a second tap, then sends the mode command as a prompt', async () => {
    const world = await boot('status-lh-graph');
    render(() => <StatusSections hostId="this-pc" sessionId="s001" />);
    await waitFor(() => expect(screen.getByTestId('lh-stop')).toBeTruthy());
    fireEvent.click(screen.getByTestId('lh-stop'));
    expect(world.log.filter((l) => l.op === 'prompt')).toHaveLength(0);
    expect(screen.getByTestId('lh-stop').textContent).toContain('Tap again');
    fireEvent.click(screen.getByTestId('lh-stop'));
    await waitFor(() => expect(world.log.some((l) => l.op === 'prompt' && (l.args as { message?: string }).message === '/graph stop')).toBe(true));
  });
});

describe('Plan & todos section', () => {
  it('renders the plan-mode chip, the plan file and the todowrite checklist', async () => {
    await boot('status-todos');
    render(() => <StatusSections hostId="this-pc" sessionId="s001" />);
    await waitFor(() => expect(screen.getByTestId('plan-section')).toBeTruthy());
    expect(screen.getByTestId('plan-mode').textContent).toContain('Plan mode');
    expect(screen.getByTestId('plan-mode').textContent).toContain('2026-02-14-a16status.md');
    const items = screen.getAllByTestId('todo-item');
    expect(items).toHaveLength(5);
    expect(items[0]!.dataset.status).toBe('completed');
    expect(items[1]!.dataset.status).toBe('in_progress');
    expect(screen.getByTestId('todo-count').textContent).toBe('1/5');
  });
});

describe('Running summary ≡ Session modal', () => {
  it("the counts equal the modal's over the same data (shared selector)", async () => {
    const world = await boot('status-running');
    render(() => <StatusSections hostId="this-pc" sessionId="s001" />);
    await waitFor(() => expect(screen.getByTestId('running-counts').textContent).not.toBe('nothing'));
    // the summary's DOM says what the shared selector says for the same session
    const s = sessions().ensure('this-pc', 's001');
    const counts = runningCounts(await fetchRunning(s));
    const text = screen.getByTestId('running-counts').textContent ?? '';
    expect(text).toContain(`${counts.bash} shell`);
    expect(text).toContain(`${counts.agents} agents`);
    expect(text).toContain(`${counts.tray} tray`);
    expect(text).toContain(String(runningTotal(counts)));
    // and the rows are the modal's first rows (+ the sidekick counts into the total)
    const rows = screen.getAllByTestId('running-row');
    expect(rows[0]!.textContent).toContain('cargo test -p unipi-host');
    expect(rows).toHaveLength(3);
    expect(screen.getByTestId('running-more').textContent).toContain(`+${runningTotal(counts) - 3} more`);
    expect(world.log.some((l) => l.op === '_oino/running_bash')).toBe(true);
  });

  it('"Open in Session" opens the Session modal on Running', async () => {
    await boot('status-running');
    render(() => <StatusSections hostId="this-pc" sessionId="s001" />);
    await waitFor(() => expect(screen.getByTestId('running-open')).toBeTruthy());
    fireEvent.click(screen.getByTestId('running-open'));
    expect(sessionModal()?.tab).toBe('running');
    expect(sessionModal()?.sessionId).toBe('s001');
  });
});

describe('capsule', () => {
  it('shows branch ±, LH progress and the running count; opens the pane', async () => {
    const world = await boot('capsule');
    // the workspace's conversation holds the session open; do the same here
    sessions().ensure('this-pc', 's001');
    render(() => <Capsule hostId="this-pc" sessionId="s001" cwd="/home/you/projects/unipi" />);
    await waitFor(() => expect(screen.getByTestId('cap-branch')).toBeTruthy(), { timeout: 3000 });
    expect(screen.getByTestId('cap-branch').textContent).toContain('v2/status');
    expect(screen.getByTestId('cap-branch').textContent).toContain('+268');
    expect(screen.getByTestId('cap-lh').textContent).toContain('Graph');
    expect(screen.getByTestId('cap-lh').textContent).toContain('4/7');
    expect(screen.getByTestId('cap-running').textContent).toContain('running');
    fireEvent.click(screen.getByTestId('status-capsule').querySelector('button')!);
    // desktop: the side pane's git tab opens (one Git UI, Z-100)
    await waitFor(() => expect(world.log.some((l) => l.op === 'git_op')).toBe(true));
  });

  it('renders nothing without data (no redundant UI)', async () => {
    const capsule = SCENARIOS['capsule']!;
    await boot({ ...capsule, hosts: capsule.hosts.map((h) => ({ ...h, git: undefined })) });
    // no repo at this path, no session topics reachable
    const { container } = render(() => <Capsule hostId="this-pc" sessionId="s999" cwd="/tmp" />);
    await waitFor(() => expect(container.querySelector('.status-capsule')).toBeNull());
  });
});
