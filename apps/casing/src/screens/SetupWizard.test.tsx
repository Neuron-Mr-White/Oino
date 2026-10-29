// @vitest-environment jsdom
/** UNI-241: the first-run "Set up this PC" takeover — the consent list, the
 *  ticking, the progress rows, the error sheet and the done screen. */
import { render, screen, cleanup, fireEvent, waitFor } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SetupWizard } from './SetupWizard';
import { dismissSetup, forgetSetupDismissal, resetSetup, setSetupTransport, type SetupTransport } from '../lib/setup';

const HOSTS = '/';

function report(ready = false) {
  return {
    components: [
      { name: 'node', state: 'outdated', version: '22.18.0', source: 'found', fix: 'Install Node 22 LTS' },
      { name: 'npm', state: 'ok', version: '10.9.0', source: 'found' },
      { name: 'pi', state: 'missing', source: 'found', fix: 'Install pi' },
      { name: 'unipi', state: 'missing', source: 'found', fix: 'pi install npm:@pi-unipi/unipi@alpha' },
      { name: 'kanboard', state: 'unsupported', source: 'found', note: 'Comes with the UniPi extensions' },
      { name: 'herdr', state: 'missing', source: 'found' },
      { name: 'tmux', state: 'unsupported', source: 'found', fix: 'sudo apt install tmux', note: 'Optional' },
    ],
    path: { value: '/usr/bin', fromShell: [], localBinOnLoginPath: false, probed: true },
    ready,
    updateAvailable: !ready,
    at: 1,
  };
}

const PLAN = {
  steps: [
    { id: 'node', label: 'Node (JavaScript runtime)', action: 'update', sizeHint: '~30 MB', needs: [] },
    { id: 'pi', label: 'pi (coding agent)', action: 'install', sizeHint: '~440 MB', needs: ['node'] },
    { id: 'unipi', label: 'UniPi extensions (channel: alpha)', action: 'install', needs: ['pi'] },
    { id: 'kanboard', label: 'Board binary', action: 'install', needs: ['unipi'] },
    { id: 'herdr', label: 'herdr (terminals)', action: 'install', sizeHint: '~5 MB', needs: [] },
    { id: 'tmux', label: 'tmux', action: 'manual', needs: [] },
  ],
  needsInternet: ['node', 'pi', 'unipi', 'herdr'],
  offline: false,
  consent: 'Node, pi, UniPi, herdr',
};

function transport(over: Partial<SetupTransport> = {}): SetupTransport {
  let only: string[] | undefined;
  return {
    detect: async () => report(),
    plan: async () => PLAN,
    run: async (_p, o) => {
      only = o;
      return { steps: PLAN.steps.map((s) => ({ step: s.id, state: { state: 'done' }, logTail: [], attempted: [] })), cancelled: false };
    },
    cancel: async () => {},
    checkUpdates: async () => ({ components: [], anyAvailable: false, offline: false }),
    installComponent: async () => ({ steps: [], cancelled: false }),
    ...over,
  } as SetupTransport & { only?: string[] };
}

/** The `only` the last `run` was called with (the ticked steps). */
let lastOnly: string[] | undefined;

/** The wizard is a route, so it renders inside one. */
function wizard() {
  return render(() => (
    <HashRouter>
      <Route path="*" component={SetupWizard} />
    </HashRouter>
  ));
}

/** Wait for the plan to land and the consent list to be seeded: Install is
 *  disabled until then, so every test has to start here. */
async function ready() {
  await waitFor(() => expect(screen.getByTestId('setup-row-node').dataset.ticked).toBe('true'));
}

beforeEach(() => {
  localStorage.clear();
  forgetSetupDismissal();
  resetSetup();
  lastOnly = undefined;
});
afterEach(() => {
  cleanup();
  resetSetup();
});

describe('Set up this PC (UNI-241)', () => {
  it('lists every component with what it would do, and ticks what the plan would touch', async () => {
    setSetupTransport(transport());
    wizard();
    await ready();
    // Pre-ticked exactly what the core planned to do, and nothing else.
    expect(screen.getByTestId('setup-row-node').dataset.ticked).toBe('true');
    expect(screen.getByTestId('setup-row-pi').dataset.ticked).toBe('true');
    // npm is not a row: it comes with Node. And tmux is never the user's to
    // tick — its step is Manual, so its row is inert and shows the command.
    expect(screen.queryByTestId('setup-row-npm')).toBeNull();
    expect(screen.getByTestId('setup-row-tmux')).toHaveProperty('disabled', true);
    // §4.1 / decision 8: the command is still the answer, but it is no longer
    // the *only* one — tmux is offered as its own opt-in.
    expect(screen.getByTestId('setup-row-note-tmux').textContent).toContain('sudo apt install tmux');
    expect(screen.queryByTestId('setup-install-for-me-tmux')).toBeTruthy();
    expect(screen.queryByTestId('setup-copy-tmux')).toBeTruthy();
    // A manual row is not part of the consent list, so it is not pre-ticked.
    expect(screen.getByTestId('setup-row-tmux')).toHaveProperty('disabled', true);
    // An older version reads as what it would become.
    // The sub-line says what the status chip deliberately does not repeat.
    expect(screen.getByTestId('setup-row-note-node').textContent).toContain('Older — 22.18.0');
    expect(screen.getByTestId('setup-row-note-node').textContent).toContain('~/.local/share/pi-node');
    expect(screen.getByTestId('setup-consent').textContent).toContain('Node');
  });

  it('unticking a row takes it out of what the core installs', async () => {
    setSetupTransport(transport({ run: async (_p, o) => { lastOnly = o; return { steps: [], cancelled: false }; } }));
    wizard();
    await ready();
    fireEvent.click(screen.getByTestId('setup-row-herdr'));
    expect(screen.getByTestId('setup-row-herdr').dataset.ticked).toBe('false');
    fireEvent.click(screen.getByTestId('setup-install'));
    await waitFor(() => expect(screen.queryByTestId('setup-done')).toBeTruthy());
    expect(lastOnly).toEqual(['node', 'pi', 'unipi', 'kanboard']);
  });

  it('says what needs the internet when the PC is offline', async () => {
    setSetupTransport(transport({ plan: async () => ({ ...PLAN, offline: true }) }));
    wizard();
    await ready();
    expect(screen.getByTestId('setup-offline').textContent).toContain('node');
    expect(screen.getByTestId('setup-offline').textContent).toContain('herdr');
  });

  it('"Not now" is remembered, so the takeover stops appearing', async () => {
    setSetupTransport(transport());
    wizard();
    await ready();
    fireEvent.click(screen.getByTestId('setup-not-now'));
    expect(localStorage.getItem('unipi.setup.notNow')).toBeTruthy();
    dismissSetup(0);
    forgetSetupDismissal();
    expect(localStorage.getItem('unipi.setup.notNow')).toBeNull();
  });

  it('a failed step opens the error sheet with the last log lines and a way out', async () => {
    setSetupTransport(
      transport({
        run: async () => ({
          steps: [
            { step: 'node', state: { state: 'failed', message: 'The download’s checksum does not match Node’s' }, logTail: ['line one', 'line two'], attempted: [], fallbackCommand: 'curl node' },
          ],
          cancelled: false,
        }),
      }),
    );
    wizard();
    await ready();
    fireEvent.click(screen.getByTestId('setup-install'));
    await waitFor(() => expect(screen.getByTestId('setup-error-message')).toBeTruthy());
    expect(screen.getByTestId('setup-error-message').textContent).toContain('checksum');
    expect(screen.getByTestId('setup-error-log').textContent).toContain('line two');
    expect(screen.getByTestId('setup-retry')).toBeTruthy();
    expect(screen.getByTestId('setup-copy-command')).toBeTruthy();
    expect(screen.getByTestId('setup-skip')).toBeTruthy();
  });

  it('a failed pi step offers "Try npm" — another way the engine really has', async () => {
    setSetupTransport(
      transport({
        run: async () => ({
          steps: [
            {
              step: 'pi',
              state: { state: 'failed', message: "pi.dev's installer exited 1" },
              logTail: [],
              attempted: ['pi.dev installer'],
              fallbackCommand: 'npm install --global --ignore-scripts @earendil-works/pi-coding-agent',
            },
          ],
          cancelled: false,
        }),
      }),
    );
    wizard();
    await ready();
    fireEvent.click(screen.getByTestId('setup-install'));
    await waitFor(() => expect(screen.getByTestId('setup-another-way')).toBeTruthy());
    // It names the method the engine would try next, not a rerun of the one that failed.
    expect(screen.getByTestId('setup-another-way').textContent).toContain('npm');
    expect(screen.getByTestId('setup-copy-command')).toBeTruthy();
  });

  it('a step whose chain is exhausted offers no "another way"', async () => {
    setSetupTransport(
      transport({
        run: async () => ({
          steps: [
            {
              step: 'pi',
              state: { state: 'failed', message: 'npm could not install pi' },
              logTail: [],
              attempted: ['pi.dev installer', 'npm'],
              fallbackCommand: 'npm install --global --ignore-scripts @earendil-works/pi-coding-agent',
            },
          ],
          cancelled: false,
        }),
      }),
    );
    wizard();
    await ready();
    fireEvent.click(screen.getByTestId('setup-install'));
    await waitFor(() => expect(screen.getByTestId('setup-error-message')).toBeTruthy());
    expect(screen.queryByTestId('setup-another-way')).toBeNull();
  });

  it('a cancelled install says what finished and offers to run it again', async () => {
    setSetupTransport(
      transport({
        run: async () => ({
          steps: [{ step: 'node', state: { state: 'done' }, logTail: [], attempted: [] }, { step: 'pi', state: { state: 'skipped', reason: 'Cancelled' }, logTail: [], attempted: [] }],
          cancelled: true,
        }),
      }),
    );
    wizard();
    await ready();
    fireEvent.click(screen.getByTestId('setup-install'));
    await waitFor(() => expect(screen.getByTestId('setup-done-steps')).toBeTruthy());
    expect(screen.getByTestId('setup-wizard').textContent).toContain('Stopped');
    expect(screen.getByTestId('setup-path-note').textContent).toContain('new terminal');
  });
});
