// @vitest-environment jsdom
/**
 * WP-A24 acceptance: the one Settings screen (D-11/Z-113 layout, Z-114
 * appearance, G-14 one Update card, G-19 extensions + safe mode, D-20
 * lifetime) and the global search across app / host / pi / extension keys.
 */
import { HostsProvider } from '../../hosts/context';
import { afterEach, beforeEach, describe, it, expect, vi } from 'vitest';
import { setPlatformOverride } from '../../lib/platform';
import { setDesktopOverride } from '../../lib/layout';
import { render, screen, waitFor, fireEvent, cleanup } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import '../../test-setup';
import Settings from '../Settings';
import { SettingsProvider } from '../../settings/context';
import { createMemoryAdapter } from '../../settings/adapter';
import { setHostClientFactory } from '../../lib/host/factory';
import { createMockHost } from '../../dev/mock/host';
import { settingsSearch as settingsSearchScenario, settingsExtensions as settingsExtensionsScenario, settingsSafeMode } from '../../dev/mock/scenarios/settings';
import { resetHubs } from '../../lib/session/hub';
import { resetSettingsMemory } from '../../lib/settings/memory';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), Channel: class {} }));

function renderSettings(adapter = createMemoryAdapter(undefined), hash = '#/settings') {
  location.hash = hash;
  return render(() => (
    <SettingsProvider adapter={adapter}>
      <HashRouter>
        <Route path="/settings/:section?/:sub?" component={() => <HostsProvider><Settings /></HostsProvider>} />
      </HashRouter>
    </SettingsProvider>
  ));
}

describe('Settings remembers where you were (owner5 #16)', () => {
  beforeEach(() => {
    setPlatformOverride('tauri-desktop');
    setDesktopOverride(true);
    resetSettingsMemory();
  });
  afterEach(() => {
    cleanup();
    resetHubs();
    resetSettingsMemory();
    setPlatformOverride(undefined);
    setDesktopOverride(undefined);
    setHostClientFactory(undefined);
  });

  it('leaving and coming back to /settings reopens the last section and extension page; a deep link wins', async () => {
    setHostClientFactory(createMockHost(settingsSearchScenario).client);
    renderSettings();
    location.hash = '#/settings/extensions';
    fireEvent.click(await screen.findByTestId('ns-memory'));
    await screen.findByTestId('namespace-page');
    cleanup();
    // Back to Settings from the rail (`/settings`, no section): same place.
    renderSettings();
    await waitFor(() => expect(location.hash).toBe('#/settings/extensions'));
    await screen.findByTestId('namespace-page');
    cleanup();
    // A deep link to another section overrides the memory.
    location.hash = '#/settings/about';
    render(() => (
      <SettingsProvider adapter={createMemoryAdapter(undefined)}>
        <HashRouter>
          <Route path="/settings/:section?" component={() => <HostsProvider><Settings /></HostsProvider>} />
        </HashRouter>
      </SettingsProvider>
    ));
    await waitFor(() => expect(document.getElementById('sec-about')).toBeTruthy());
    expect(screen.queryByTestId('namespace-page')).toBeNull();
  });
});

describe('Settings screen', () => {
  beforeEach(() => {
    setPlatformOverride('tauri-mobile');
    setDesktopOverride(false);
  });
  afterEach(() => {
    cleanup();
    resetHubs();
    resetSettingsMemory();
    setPlatformOverride(undefined);
    setDesktopOverride(undefined);
    setHostClientFactory(undefined);
  });

  it('phone: the root is a grouped list of section pages (§3.9, PH-09/PH-20)', async () => {
    const r = renderSettings();
    await waitFor(() => expect(r.getByTestId('settings-phone-root')).toBeTruthy());
    expect(r.getByText('This phone')).toBeTruthy();
    expect(r.getByText('PCs & connection')).toBeTruthy();
    expect(r.getByTestId('settings-row-appearance')).toBeTruthy();
    expect(r.getByTestId('settings-row-extensions')).toBeTruthy();
    expect(r.getByTestId('settings-row-profiles')).toBeTruthy();
    expect(r.getByTestId('settings-row-ports')).toBeTruthy();
    expect(r.getByTestId('settings-row-about')).toBeTruthy();
    // Rows only — no section bodies inline.
    expect(r.queryByTestId('terminal-cursor-blink')).toBeNull();
    // Desktop-only pages stay off the phone (no tray, no local host config, no shortcuts).
    expect(r.queryByTestId('settings-row-shortcuts')).toBeNull();
    expect(r.queryByTestId('settings-row-general')).toBeNull();
    expect(r.queryByTestId('settings-row-host')).toBeNull();
  });

  it('phone: a section is its own page', async () => {
    const r = renderSettings(undefined, '#/settings/terminal');
    await waitFor(() => expect(r.getByTestId('terminal-cursor-blink')).toBeTruthy());
    expect(r.queryByTestId('settings-phone-root')).toBeNull();
    expect(r.queryByText('Scrollback')).toBeTruthy();
    expect(r.queryByText('Vim mode')).toBeNull();
  });

  it('web: only what works there — no Terminal/Input/Editor/Connection', async () => {
    setPlatformOverride('web');
    const r = renderSettings();
    await waitFor(() => expect(r.getByTestId('settings-row-about')).toBeTruthy());
    for (const gone of ['terminal', 'input', 'editor', 'connection']) expect(r.queryByTestId(`settings-row-${gone}`)).toBeNull();
  });

  it('toggling a Terminal row persists through the adapter', async () => {
    const adapter = createMemoryAdapter(undefined);
    const r = renderSettings(adapter, '#/settings/terminal');
    await waitFor(() => expect(r.getByTestId('terminal-cursor-blink')).toBeTruthy());
    const blinkSwitch = r.getByTestId('terminal-cursor-blink').querySelector('input[role=switch]') as HTMLInputElement;
    expect(blinkSwitch.checked).toBe(true);
    blinkSwitch.click();
    await waitFor(async () => {
      const persisted = (await adapter.read()) as { terminal: { cursorBlink: boolean } } | undefined;
      expect(persisted?.terminal.cursorBlink).toBe(false);
    });
  });

  it('appearance: the UI font size applies the var app-wide (Z-114)', async () => {
    const adapter = createMemoryAdapter(undefined);
    const r = renderSettings(adapter, '#/settings/appearance');
    await waitFor(() => expect(r.getByLabelText('Increase UI font size')).toBeTruthy());
    expect(document.documentElement.style.getPropertyValue('--ui-font-size')).toBe('15px');
    fireEvent.click(r.getByLabelText('Increase UI font size'));
    await waitFor(() => expect(document.documentElement.style.getPropertyValue('--ui-font-size')).toBe('16px'));
    const persisted = (await adapter.read()) as { appearance?: { uiFontSize: number } } | undefined;
    expect(persisted?.appearance?.uiFontSize).toBe(16);
    // The root html font-size is never touched (ZCode rule).
    expect(document.documentElement.style.fontSize).toBe('');
  });

  it('search finds app, host, pi and extension keys (fixture schemas)', async () => {
    setHostClientFactory(createMockHost(settingsSearchScenario).client);
    setPlatformOverride('tauri-desktop');
    setDesktopOverride(true);
    const r = renderSettings();
    const input = await screen.findByTestId('settings-search-input');
    await fireEvent.input(input, { target: { value: 'memory' } });
    // app row (the memory extension's own card), extension namespace + field, pi key.
    await waitFor(() => expect(r.getAllByTestId('settings-hit-extension').length).toBeGreaterThan(0));
    fireEvent.input(input, { target: { value: 'relay' } });
    await waitFor(() => expect(r.getAllByTestId('settings-hit-host').length).toBeGreaterThan(0));
    fireEvent.input(input, { target: { value: 'defaultModel' } });
    await waitFor(() => expect(r.getAllByTestId('settings-hit-pi').length).toBeGreaterThan(0));
    fireEvent.input(input, { target: { value: 'font' } });
    await waitFor(() => expect(r.getAllByTestId('settings-hit-app').length).toBeGreaterThan(0));
  });

  it('a search result opens its section (Z-113)', async () => {
    setHostClientFactory(createMockHost(settingsSearchScenario).client);
    setPlatformOverride('tauri-desktop');
    setDesktopOverride(true);
    const r = renderSettings();
    const input = await screen.findByTestId('settings-search-input');
    await fireEvent.input(input, { target: { value: 'appearance' } });
    const hit = await waitFor(() => {
      const first = r.getAllByTestId('settings-hit-app')[0];
      expect(first).toBeTruthy();
      return first;
    });
    fireEvent.click(hit);
    await waitFor(() => expect(r.getByLabelText('Increase UI font size')).toBeTruthy());
    expect((r.getByTestId('settings-search-input') as HTMLInputElement).value).toBe('');
  });

  it('extensions: toggle round-trips through the mock host (G-19)', async () => {
    setHostClientFactory(createMockHost(settingsSearchScenario).client);
    const r = renderSettings(undefined, '#/settings/extensions');
    const toggle = (await screen.findByTestId('ext-unipi.cron'))!.querySelector('input[role=switch]') as HTMLInputElement;
    expect(toggle.checked).toBe(false);
    toggle.click();
    await waitFor(() => expect(((screen.getByTestId('ext-unipi.cron') as HTMLElement).querySelector('input[role=switch]') as HTMLInputElement).checked).toBe(true));
  });

  it('a namespace opens a secondary page with the standard Back (Z-113)', async () => {
    setHostClientFactory(createMockHost(settingsSearchScenario).client);
    const r = renderSettings(undefined, '#/settings/extensions');
    fireEvent.click(await screen.findByTestId('ns-memory'));
    const page = await screen.findByTestId('namespace-page');
    expect(page.textContent).toContain('Memory');
    // Phone: the namespace is a route of its own (PH-09).
    expect(location.hash).toBe('#/settings/extensions/memory');
    fireEvent.click(screen.getByTestId('settings-back'));
    await waitFor(() => expect(r.queryByTestId('namespace-page')).toBeNull());
    expect(r.getByTestId('ns-memory')).toBeTruthy();
  });

  it('a crashed extension shows a notice with the reason (G-19)', async () => {
    setHostClientFactory(createMockHost(settingsExtensionsScenario).client);
    const r = renderSettings(undefined, '#/settings/extensions');
    const notice = await screen.findByTestId('extension-crash-notice');
    expect(notice.textContent).toContain('unipi.cron');
    expect(notice.textContent).toContain('wasm trap in schedule_tick');
  });

  it('the safe-mode banner names its suspects (G-19)', async () => {
    setHostClientFactory(createMockHost(settingsSafeMode).client);
    const r = renderSettings(undefined, '#/settings/extensions');
    const banner = await screen.findByTestId('safe-mode-banner');
    expect(banner.textContent).toContain('Safe mode');
    expect(banner.textContent).toContain('unipi.cron');
    expect(banner.textContent).toContain('unipi.dream');
  });

  it('about: exactly one Update card (G-14)', async () => {
    const r = renderSettings(undefined, '#/settings/about');
    await waitFor(() => expect(r.getByTestId('update-card')).toBeTruthy());
    expect(r.getAllByTestId('update-card').length).toBe(1);
    expect(r.queryByTestId('desktop-update-card')).toBeNull();
    expect(r.queryByTestId('ios-update-card')).toBeNull();
  });
});
