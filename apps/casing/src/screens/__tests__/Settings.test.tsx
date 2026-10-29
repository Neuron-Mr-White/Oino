import { HostsProvider } from '../../hosts/context';
// @vitest-environment jsdom
import { afterEach, beforeEach, describe, it, expect, vi } from 'vitest';
import { setPlatformOverride } from '../../lib/platform';
import { render, waitFor } from '@solidjs/testing-library';
import '../../test-setup';
import Settings from '../Settings';
import { SettingsProvider } from '../../settings/context';
import { createMemoryAdapter } from '../../settings/adapter';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')) }));

describe('Settings screen', () => {
  beforeEach(() => setPlatformOverride('tauri-mobile'));
  afterEach(() => setPlatformOverride(undefined));
  it('renders the Terminal section and toggling cursor blink persists through the adapter', async () => {
    const adapter = createMemoryAdapter(undefined);
    const r = render(() => (
      <SettingsProvider adapter={adapter}>
        <HostsProvider><Settings /></HostsProvider>
      </SettingsProvider>
    ));

    await waitFor(() => expect(r.getByText('Terminal')).toBeTruthy());

    const blinkSwitch = r.getByLabelText('Cursor blink').querySelector('input[role=switch]') as HTMLInputElement;
    expect(blinkSwitch.checked).toBe(true);

    blinkSwitch.click();

    await waitFor(async () => {
      const persisted = (await adapter.read()) as { terminal: { cursorBlink: boolean } } | undefined;
      expect(persisted?.terminal.cursorBlink).toBe(false);
    });
  });

  it('renders Hosts, Terminal, Input, Connection, Board and About sections', async () => {
    const adapter = createMemoryAdapter(undefined);
    const r = render(() => (
      <SettingsProvider adapter={adapter}>
        <HostsProvider><Settings /></HostsProvider>
      </SettingsProvider>
    ));
    await waitFor(() => expect(r.getByText('Terminal')).toBeTruthy());
    expect(r.getByText('Hosts')).toBeTruthy();
    expect(r.getByText('Input')).toBeTruthy();
    expect(r.getByText('Connection')).toBeTruthy();
    expect(r.getByText('Board')).toBeTruthy();
    expect(r.getByText('About')).toBeTruthy();
  });

  it('web: only what works there — Hosts notice, Board, About; no Terminal/Input/Editor/Connection', async () => {
    setPlatformOverride('web');
    const adapter = createMemoryAdapter(undefined);
    const r = render(() => (
      <SettingsProvider adapter={adapter}>
        <HostsProvider><Settings /></HostsProvider>
      </SettingsProvider>
    ));
    await waitFor(() => expect(r.getByText('Board')).toBeTruthy());
    expect(r.getByText('Hosts')).toBeTruthy();
    expect(r.getByText('Not available in the web version')).toBeTruthy();
    expect(r.getByText('About')).toBeTruthy();
    for (const gone of ['Terminal', 'Input', 'Editor', 'Connection']) expect(r.queryByText(gone)).toBeNull();
  });
});
