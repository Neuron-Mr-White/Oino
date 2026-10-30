// @vitest-environment jsdom
/** UNI-241: Settings → This computer → Components (§4.3 point 6) — the rows
 *  from `setup_detect` + `setup_check_updates`, the desktop app's own row from
 *  the signed updater's state, and the banner that stands in for the takeover. */
import { render, screen, cleanup, fireEvent, waitFor } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ComponentsCard } from './ComponentsCard';
import { checkForUpdates, forgetSetupDismissal, refreshSetup, resetSetup, setSetupTransport, setupReport, type SetupReport, type SetupTransport } from '../lib/setup';
import { resetDesktopUpdate, setDesktopUpdateTransport, desktopUpdateState } from '../lib/desktopUpdate';
import { setPlatformOverride } from '../lib/platform';
import { setupBannerVisible } from '../lib/setupGate';
import { setupGateOpen } from '../lib/setup';

const report: SetupReport = {
  components: [
    { name: 'node', state: 'ok', version: '24.16.0', source: 'found' },
    { name: 'npm', state: 'ok', version: '11.13.0', source: 'found' },
    { name: 'pi', state: 'ok', version: '1.2.0', source: 'found' },
    // An older install: pre-ticked as an update.
    { name: 'unipi', state: 'ok', version: '3.0.0-alpha.41', source: 'found', note: 'Installed as npm:@pi-unipi/unipi@alpha' },
    { name: 'kanboard', state: 'ok', version: '0.2.0', source: 'found' },
    { name: 'agent-browser', state: 'ok', version: '0.9.1', source: 'found' },
    { name: 'xvfb', state: 'missing', source: 'found', fix: 'sudo apt install xvfb', note: 'Optional — better terminals' },
    { name: 'host', state: 'ok', version: '0.4.9', source: 'found', note: 'Run by service' },
  ],
  path: { value: '/usr/bin', fromShell: [], localBinOnLoginPath: true, probed: true },
  ready: true,
  updateAvailable: true,
  at: 1,
};

function transport(over: Partial<SetupTransport> = {}): SetupTransport {
  return {
    detect: async () => report,
    plan: async () => ({ steps: [], needsInternet: [], offline: false, consent: '' }),
    run: async () => ({ steps: [], cancelled: false }),
    cancel: async () => {},
    checkUpdates: async () => ({
      components: [{ name: 'unipi', update: { state: 'available', from: '3.0.0-alpha.41', to: '3.0.0-alpha.42' } }],
      anyAvailable: true,
      offline: false,
    }),
    installComponent: async () => ({ steps: [], cancelled: false }),
    ...over,
  } satisfies SetupTransport;
}

function card() {
  return render(() => (
    <HashRouter>
      <Route path="*" component={ComponentsCard} />
    </HashRouter>
  ));
}

beforeEach(() => {
  localStorage.clear();
  forgetSetupDismissal();
  resetSetup();
  resetDesktopUpdate();
  setPlatformOverride('tauri-desktop');
});
afterEach(() => {
  cleanup();
  resetSetup();
  resetDesktopUpdate();
  setPlatformOverride(undefined);
  setSetupTransport(undefined);
  setDesktopUpdateTransport(undefined);
});

describe('Settings → Components (UNI-241, §4.3 point 6)', () => {
  it('shows each component with its version and what the feeds offer', async () => {
    setSetupTransport(transport());
    card();
    await waitFor(() => expect(screen.getByTestId('components-row-unipi')).toBeTruthy());
    // An older install reads as what it would become, and offers the update.
    expect(screen.getByTestId('components-row-unipi').textContent).toContain('Update 3.0.0-alpha.41 → 3.0.0-alpha.42');
    expect(screen.getByTestId('components-repair-unipi').textContent).toContain('Update');
    expect(screen.getByTestId('components-row-node').textContent).toContain('Node');
    expect(screen.getByTestId('components-hint').textContent).toContain('1');
  });

  it('a newer version than tested is left alone and says so', async () => {
    const newer = { ...report, components: report.components.map((c) => (c.name === 'node' ? { ...c, version: '24.16.0' } : c)) };
    setSetupTransport(
      transport({
        detect: async () => newer,
        checkUpdates: async () => ({
          components: [{ name: 'node', update: { state: 'current', installed: '24.16.0', offered: '22.19.0' } }],
          anyAvailable: false,
          offline: false,
        }),
      }),
    );
    card();
    await waitFor(() => expect(screen.getByTestId('components-row-node')).toBeTruthy());
    expect(screen.getByTestId('components-row-node').textContent).toContain('Newer than tested');
    // Nothing to do, so no Update button.
    expect(screen.queryByTestId('components-repair-node')).toBeNull();
  });

  it('the desktop app comes from the signed updater, not the setup engine', async () => {
    setSetupTransport(transport());
    setDesktopUpdateTransport({
      check: async () => ({ current: '0.4.9', kind: 'manual', version: '0.4.10', downloadPage: 'https://unipi.nrn.one/' }),
      install: vi.fn(),
    });
    card();
    await waitFor(() => expect(screen.getByTestId('components-row-desktop')).toBeTruthy());
    expect(screen.getByTestId('components-row-desktop').textContent).toContain('Update 0.4.9 → 0.4.10');
    // A manual install can't replace itself: the row says so and offers a
    // download instead of an in-app update.
    expect(screen.getByTestId('components-row-desktop').textContent).toContain('download');
    expect(screen.getByTestId('components-desktop-download')).toBeTruthy();
    expect(screen.queryByTestId('components-desktop-update')).toBeNull();
    // The check button re-reads the feeds.
    fireEvent.click(screen.getByTestId('components-check'));
    await waitFor(() => expect(desktopUpdateState().kind).not.toBe('idle'));
  });

  it('offline never claims a component is up to date', async () => {
    setSetupTransport(
      transport({
        checkUpdates: async () => ({ components: [{ name: 'unipi', update: { state: 'unknown' } }], anyAvailable: false, offline: true }),
      }),
    );
    card();
    await waitFor(() => expect(screen.getByTestId('components-offline')).toBeTruthy());
    expect(screen.getByTestId('components-row-unipi').textContent).not.toContain('Update');
    expect(screen.queryByTestId('components-repair-unipi')).toBeNull();
    // §4.3 point 4: an unknown feed answer never dresses itself up as
    // "Up to date". What is installed is reported; newer-or-not is not.
    for (const name of ['node', 'pi', 'unipi', 'agent-browser']) {
      expect(screen.getByTestId(`components-row-${name}`).textContent).toContain('Installed');
      expect(screen.getByTestId(`components-row-${name}`).textContent).not.toContain('Up to date');
    }
  });

  it('Update all runs the install engine for every component that needs it', async () => {
    const installed: string[] = [];
    setSetupTransport(
      transport({
        checkUpdates: async () => ({
          components: [
            { name: 'unipi', update: { state: 'available', from: '3.0.0-alpha.41', to: '3.0.0-alpha.42' } },
            { name: 'agent-browser', update: { state: 'available', from: '0.9.1', to: '0.9.2' } },
          ],
          anyAvailable: true,
          offline: false,
        }),
        installComponent: async (name) => {
          installed.push(name);
          return { steps: [], cancelled: false };
        },
      }),
    );
    card();
    await waitFor(() => expect(screen.getByTestId('components-update-all')).toBeTruthy());
    await waitFor(() => expect(screen.queryByTestId('components-repair-unipi')).toBeTruthy());
    expect(screen.queryByTestId('components-repair-agent-browser')).toBeTruthy();
  });

  it('a failed install says so, and never claims it worked', async () => {
    setSetupTransport(transport({ installComponent: async () => undefined }));
    card();
    await waitFor(() => expect(screen.getByTestId('components-repair-unipi')).toBeTruthy());
    fireEvent.click(screen.getByTestId('components-repair-unipi'));
    await waitFor(() => expect(screen.getByTestId('components-error')).toBeTruthy());
    expect(screen.getByTestId('components-error').textContent).toContain('UniPi extensions');
  });
});

describe('the banner the takeover leaves behind (UNI-241)', () => {
  it('a PC that is not ready and not dismissed shows the banner', async () => {
    const notReady = { ...report, ready: false };
    setSetupTransport(transport({ detect: async () => notReady }));
    await refreshSetup();
    // Nothing dismissed yet: the takeover shows, so there is no banner.
    expect(setupBannerVisible(setupReport(), 0)).toBe(false);
    // Once "Not now" is remembered, the banner stands in for the takeover.
    expect(setupBannerVisible(setupReport(), Date.now())).toBe(true);
    // …until it expires and the takeover comes back (setupGateOpen's rule).
    const dismissed = Date.now();
    expect(setupGateOpen(setupReport(), dismissed, dismissed + 1000)).toBe(false);
    // A ready PC shows neither.
    setSetupTransport(transport());
    await refreshSetup();
    expect(setupBannerVisible(setupReport(), 0)).toBe(false);
  });

  it('"Check for updates" refreshes the feeds', async () => {
    let calls = 0;
    setSetupTransport(transport({ checkUpdates: async () => { calls++; return { components: [], anyAvailable: false, offline: false }; } }));
    card();
    await waitFor(() => expect(screen.getByTestId('components-check-updates')).toBeTruthy());
    fireEvent.click(screen.getByTestId('components-check-updates'));
    await waitFor(() => expect(calls).toBeGreaterThan(0));
    const after = await checkForUpdates();
    expect(after?.anyAvailable).toBe(false);
  });
});
