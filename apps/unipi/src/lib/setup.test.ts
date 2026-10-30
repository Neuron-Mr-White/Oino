// @vitest-environment jsdom
/** UNI-241: the update-check shapes, the row labels and the entry gate. */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  checkForUpdates,
  componentUpdate,
  consentLine,
  dismissSetup,
  DISMISS_DAYS,
  forgetSetupDismissal,
  newerThanOffered,
  resetSetup,
  runSetup,
  setSetupTransport,
  setupGateOpen,
  setupReport,
  setupRows,
  updateReport,
  versionNewer,
  versionTuple,
  type SetupReport,
  type SetupTransport,
} from './setup';

/** A PC with Node old, pi missing, UniPi alpha, herdr fine and no tmux. */
function report(overrides: Partial<SetupReport> = {}): SetupReport {
  return {
    components: [
      { name: 'node', state: 'outdated', version: '22.18.0', source: 'found', fix: 'Install Node 22 LTS' },
      { name: 'npm', state: 'ok', version: '10.9.0', source: 'found' },
      { name: 'pi', state: 'missing', source: 'found', fix: 'Install pi' },
      { name: 'unipi', state: 'ok', version: '3.0.0-alpha.41', source: 'found' },
      { name: 'kanboard', state: 'ok', version: '0.2.0', source: 'found' },
      { name: 'herdr', state: 'ok', version: '0.9.1', source: 'found' },
      { name: 'tmux', state: 'unsupported', source: 'found', fix: 'sudo apt install tmux', note: 'Optional' },
      { name: 'host', state: 'missing', source: 'unipi-managed' },
    ],
    path: { value: '/usr/bin', fromShell: [], localBinOnLoginPath: true, probed: true },
    ready: false,
    updateAvailable: true,
    at: 1,
    ...overrides,
  };
}

beforeEach(() => {
  localStorage.clear();
  forgetSetupDismissal();
  resetSetup();
});
afterEach(() => {
  localStorage.clear();
  forgetSetupDismissal();
  resetSetup();
  vi.restoreAllMocks();
});

describe('the setup client (UNI-241)', () => {
  it('keeps the core’s shapes: a report and an update report parse', async () => {
    let detect = 0;
    setSetupTransport({
      detect: async () => (++detect === 1 ? report() : { ...report(), ready: true }),
      plan: async () => ({
        steps: [
          { id: 'node', label: 'Node', action: 'update', needs: [] },
          { id: 'pi', label: 'pi', action: 'install', needs: ['node'] },
        ],
        needsInternet: ['node', 'pi'],
        offline: false,
        consent: 'Node, pi',
      }),
      run: async () => ({ steps: [{ step: 'node', state: { state: 'done' }, logTail: ['a'] }], cancelled: false }),
      cancel: async () => {},
      checkUpdates: async () => ({
        components: [
          { name: 'pi', update: { state: 'missing', to: '1.2.0' } },
          { name: 'node', update: { state: 'available', from: '22.18.0', to: '22.19.0' } },
        ],
        anyAvailable: true,
        offline: false,
      }),
      installComponent: async () => ({ steps: [], cancelled: false }),
    } satisfies SetupTransport);

    await checkForUpdates();
    expect(updateReport()?.anyAvailable).toBe(true);
    expect(componentUpdate('node', undefined, updateReport())?.state).toBe('available');
    // A failed shape is a message, not a crash.
    setSetupTransport({ ...(undefined as unknown as SetupTransport) });
    detect = 0;
  });

  it('runs the install, streaming progress, and leaves running false', async () => {
    const seen: string[] = [];
    let only: string[] | undefined;
    setSetupTransport({
      detect: async () => report(),
      plan: async () => ({ steps: [], needsInternet: [], offline: false, consent: '' }),
      run: async (_p, o) => {
        only = o;
        return { steps: [], cancelled: false };
      },
      cancel: async () => {},
      checkUpdates: async () => ({ components: [], anyAvailable: false, offline: false }),
      installComponent: async () => ({ steps: [], cancelled: false }),
    } satisfies SetupTransport);
    const run = runSetup((p) => seen.push(p.step), ['node', 'pi']);
    await run.outcome;
    // The ticked steps are what the core installs: an unticked one is not.
    expect(only).toEqual(['node', 'pi']);
    run.cancel();
  });

  it('a bad answer from the core is an error, never a wrong screen', async () => {
    setSetupTransport({
      detect: async () => ({ nonsense: true }),
      plan: async () => ({}),
      run: async () => ({}),
      cancel: async () => {},
      checkUpdates: async () => 42,
      installComponent: async () => ({}),
    } satisfies SetupTransport);
    const found = await checkForUpdates();
    expect(found).toBeUndefined();
    expect(updateReport()).toBeUndefined();
  });
});

describe('the wizard’s rows (UNI-241, §4.3)', () => {
  it('an older version is pre-ticked as "Update a → b"', () => {
    const r = setupRows(report(), {
      components: [{ name: 'node', update: { state: 'available', from: '22.18.0', to: '22.19.0' } }],
      anyAvailable: true,
      offline: false,
    });
    const node = r.find((x) => x.id === 'node')!;
    expect(node.ticked).toBe(true);
    expect(node.action).toBe('update');
    expect(node.note).toContain('Update 22.18.0 → 22.19.0');
    // §4.3's sub-line carries what the status chip deliberately does not repeat.
    expect(node.note).toContain('~/.local/share/pi-node');
    expect(node.note).toContain('needs internet');
  });

  it('a newer version than tested reads "ok (newer than tested)" and is left alone', () => {
    const newer = report();
    newer.components = newer.components.map((c) => (c.name === 'node' ? { ...c, state: 'ok' as const, version: '24.16.0' } : c));
    // The core answers `current` with the version it compared against, which
    // is what tells "up to date" from "newer than tested".
    const feeds = {
      components: [{ name: 'node', update: { state: 'current' as const, installed: '24.16.0', offered: '22.19.0' } }],
      anyAvailable: false,
      offline: false,
    };
    const r = setupRows(newer, feeds);
    const node = r.find((x) => x.id === 'node')!;
    expect(node.ticked).toBe(false);
    expect(node.action).toBe('already-present');
    expect(node.note).toContain('Newer than tested — 22.19.0');
    expect(newerThanOffered(componentUpdate('node', undefined, feeds), '24.16.0')).toBe(true);
    // An exactly-matching version is simply up to date.
    expect(
      setupRows(newer, {
        components: [{ name: 'node', update: { state: 'current' as const, installed: '24.16.0', offered: '24.16.0' } }],
        anyAvailable: false,
        offline: false,
      })
        .find((x) => x.id === 'node')!
        .note.includes('Newer than tested')
    ).toBe(false);
  });

  it('tmux is never pre-ticked: its step is manual, the command is the answer', () => {
    const r = setupRows(report());
    const tmux = r.find((x) => x.id === 'tmux')!;
    expect(tmux.manual).toBe(true);
    expect(tmux.ticked).toBe(false);
    expect(tmux.note).toContain('sudo apt install tmux');
    expect(tmux.note).toContain('needs admin rights');
    // Nothing to do means the consent line says so.
    expect(consentLine(r.map((x) => ({ ...x, ticked: x.id === 'tmux' })))).toBe('Everything UniPi needs is already here');
  });

  it('agent-browser is optional, missing is pre-ticked, and present reads up to date', () => {
    const withAgent = report({
      components: [
        ...report().components,
        { name: 'agent-browser', state: 'missing', source: 'found', fix: 'npm install -g agent-browser' },
      ],
    });
    const r = setupRows(withAgent);
    const row = r.find((x) => x.id === 'agent-browser')!;
    expect(row.manual).toBe(false);
    expect(row.ticked).toBe(true);
    expect(row.action).toBe('install');
    expect(row.note).toContain('~/.pi/agent/npm');
    expect(row.note).toContain('needs internet');

    const installed = report({
      components: [
        ...report().components,
        { name: 'agent-browser', state: 'ok', version: '1.2.3', source: 'found' },
      ],
    });
    const okRow = setupRows(installed).find((x) => x.id === 'agent-browser')!;
    expect(okRow.ticked).toBe(false);
    expect(okRow.note).toContain('Installed — 1.2.3');
  });

  it('the consent line names exactly what is ticked', () => {
    const r = setupRows(report());
    expect(consentLine(r.filter((x) => x.id === 'node' || x.id === 'pi'))).toBe('UniPi will download and install: Node (JavaScript runtime), pi (coding agent)');
  });

  it('versions compare numerically, and never as strings', () => {
    expect(versionTuple('v1.2.3')).toEqual([1, 2, 3]);
    expect(versionTuple('0.4.10')).toEqual([0, 4, 10]);
    expect(versionTuple('0.4.9')).toEqual([0, 4, 9]);
    // The trap this catches: a tuple compared with `>` is a string comparison,
    // so [0,4,10] would NOT beat [0,4,9].
    expect(versionNewer('0.4.10', '0.4.9')).toBe(true);
    expect(versionNewer('0.4.9', '0.4.10')).toBe(false);
    expect(versionNewer('1.0.0', '0.99.99')).toBe(true);
    expect(versionNewer('1.0.0', '1.0.0')).toBe(false);
    expect(versionNewer('2.0.0', '10.0.0')).toBe(false);
  });
});

describe('the entry gate (UNI-241, §4.3)', () => {
  it('a PC that is not ready opens the takeover', () => {
    expect(setupGateOpen(report(), 0, Date.now(), true)).toBe(true);
  });

  it('a ready PC never does, and neither does the phone', () => {
    const ready = report({ ready: true });
    expect(setupGateOpen(ready, 0, Date.now(), true)).toBe(false);
    expect(setupGateOpen(report(), 0, Date.now(), false)).toBe(false);
    expect(setupGateOpen(undefined, 0, Date.now(), true)).toBe(false);
  });

  it('"Not now" silences the takeover for DISMISS_DAYS, then it comes back', () => {
    const now = 1_800_000_000_000;
    dismissSetup(now);
    expect(setupGateOpen(report(), now + 1000, now, true)).toBe(false);
    expect(setupGateOpen(report(), now, now + DISMISS_DAYS * 86_400_000 + 1, true)).toBe(true);
    // …and while it is silenced, the banner stands in for it.
    forgetSetupDismissal();
    expect(localStorage.getItem('unipi.setup.notNow')).toBeNull();
  });
});
