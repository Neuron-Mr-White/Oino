import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@solidjs/testing-library';
import type { Components } from '../lib/about';

const ipc = vi.hoisted(() => ({ call: vi.fn() }));
vi.mock('../lib/ipc', () => ({ call: ipc.call }));

import { ComponentsCard } from './ComponentsCard';
import { brokenInstallMessage, resetComponents } from '../lib/about';
import { setPlatformOverride } from '../lib/platform';

const healthy: Components = {
  app: '1.0.0',
  host: { version: '1.0.0', path: '/usr/bin/unipi-oino-host' },
  oino: { version: '1.0.2', path: '/usr/bin/oino' },
  extensions: { count: 31, dir: '/usr/share/oino/extensions' },
  problems: [],
};

describe('ComponentsCard (About → Components)', () => {
  beforeEach(() => {
    resetComponents();
    setPlatformOverride('tauri-desktop');
  });
  afterEach(() => {
    cleanup();
    setPlatformOverride(undefined);
    ipc.call.mockReset();
  });

  it('shows what the install ships: app, host, engine, extension count', async () => {
    ipc.call.mockResolvedValue({ ok: true, value: healthy });
    render(() => <ComponentsCard />);
    await waitFor(() => expect(screen.getByTestId('components-extensions').textContent).toBe('31 bundled'));
    expect(ipc.call).toHaveBeenCalledWith('about_components', {}, expect.anything());
    expect(screen.getByTestId('about-version').textContent).toBe('1.0.0');
    expect(screen.getByTestId('components-host').textContent).toBe('1.0.0');
    expect(screen.getByTestId('components-oino').textContent).toBe('1.0.2');
    expect(screen.getByText('/usr/share/oino/extensions')).toBeInTheDocument();
    expect(screen.queryByRole('alert')).toBeNull();
    // Nothing to install, nothing legacy.
    expect(document.body.textContent).not.toMatch(/\bnpm\b|\bpi\b|Node|Install for me/);
  });

  it('a broken install names the problem and the fix, never an install prompt', async () => {
    ipc.call.mockResolvedValue({
      ok: true,
      value: {
        ...healthy,
        host: {},
        extensions: { count: 0 },
        problems: [{ component: 'host', message: 'The agent host (unipi-oino-host) is missing from this install.', fix: 'Reinstall UniPi Oino from the latest release.' }],
      },
    });
    render(() => <ComponentsCard />);
    await waitFor(() => expect(screen.getByTestId('components-problem-host')).toBeInTheDocument());
    expect(screen.getByText(/Reinstall UniPi Oino/)).toBeInTheDocument();
    expect(screen.getByTestId('components-host').textContent).toBe('Not found');
    expect(screen.getByTestId('components-download')).toBeInTheDocument();
  });

  it('only a problem makes a broken-install message', () => {
    expect(brokenInstallMessage(healthy)).toBeUndefined();
    expect(brokenInstallMessage({ ...healthy, problems: [{ component: 'extensions', message: 'No extensions.', fix: 'Reinstall.' }] })).toBe('No extensions. Reinstall.');
  });

  it('reads nothing off the desktop (a phone has no bundled host)', async () => {
    setPlatformOverride('tauri-mobile');
    render(() => <ComponentsCard />);
    await Promise.resolve();
    expect(ipc.call).not.toHaveBeenCalled();
  });
});
