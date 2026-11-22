// @vitest-environment jsdom
/**
 * WP-U11 acceptance at the screen: the OSS-2 form engine inside the one
 * Settings screen — an extension's namespace page with the per-extension
 * enable toggle as its first row (`_oino/set_extension_enabled` round trip,
 * DIRECTION §3.3.4), the core `oino.*` namespaces under Agent, and the global
 * search landing on a field row (§7 "search covers labels, descriptions and
 * keys of all namespaces").
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { HostsProvider } from '../../hosts/context';
import { SettingsProvider } from '../../settings/context';
import { createMemoryAdapter } from '../../settings/adapter';
import { setHostClientFactory } from '../../lib/host/factory';
import { setDesktopOverride } from '../../lib/layout';
import { setPlatformOverride } from '../../lib/platform';
import { resetSettingsMemory } from '../../lib/settings/memory';
import { resetHubs } from '../../lib/session/hub';
import { createMockHost, type MockHostSpec, type MockNamespace } from '../../dev/mock/host';
import { memoryFormNamespace, modelNamespace } from '../../dev/mock/fixtures/settingsForm';
import Settings from '../Settings';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), Channel: class {} }));

type Case = {
  memory: MockNamespace;
  model: MockNamespace;
  extensions: Array<{ id: string; enabled: boolean; loaded: boolean }>;
};

function setup(): Case {
  const memory = structuredClone(memoryFormNamespace) as MockNamespace;
  const model = structuredClone(modelNamespace) as MockNamespace;
  const extensions = [
    { id: 'unipi.memory', enabled: true, loaded: true },
    { id: 'unipi.notify', enabled: true, loaded: true },
  ];
  const spec = {
    id: 'local',
    name: 'workstation',
    transport: 'local',
    conn: { state: 'connected' },
    cards: [],
    settings: { pi: {}, namespaces: [memory, model], extensions },
  } as MockHostSpec;
  setHostClientFactory(createMockHost({ name: 'settings-form-screen', hosts: [spec] }).client);
  return { memory, model, extensions };
}

function renderSettings() {
  return render(() => (
    <SettingsProvider adapter={createMemoryAdapter(undefined)}>
      <HashRouter>
        <Route path="/settings/:section?" component={() => <HostsProvider><Settings /></HostsProvider>} />
      </HashRouter>
    </SettingsProvider>
  ));
}

beforeEach(() => {
  setPlatformOverride('tauri-desktop');
  setDesktopOverride(true);
  location.hash = '#/settings/extensions';
});
afterEach(() => {
  cleanup();
  resetHubs();
  resetSettingsMemory();
  setPlatformOverride(undefined);
  setDesktopOverride(undefined);
  setHostClientFactory(undefined);
});

describe('Settings › Extensions › a namespace page (WP-U11)', () => {
  it('opens with the extension enable toggle as the first row, then its form', async () => {
    setup();
    renderSettings();
    fireEvent.click(await screen.findByTestId('ns-memory'));
    await screen.findByTestId('namespace-page');
    // One place per extension: the toggle sits above the fields.
    const toggle = screen.getByTestId('ext-unipi.memory');
    expect(toggle.querySelector('input[role=switch]')).toBeTruthy();
    const form = screen.getByTestId('form-memory');
    expect(form.textContent).toContain('Auto-recall');
    // The toggle is the first interactive row of the namespace section.
    const rows = [...screen.getByTestId('namespace-page').querySelectorAll('[data-testid^=field-], [data-testid^=ext-]')];
    expect((rows[0] as HTMLElement).dataset.testid).toBe('ext-unipi.memory');
    // Back returns to the list (Z-113).
    fireEvent.click(screen.getByTestId('settings-back'));
    await waitFor(() => expect(screen.queryByTestId('namespace-page')).toBeNull());
  });

  it('the toggle round-trips _oino/set_extension_enabled; a disabled extension hides its namespace but keeps values (§7)', async () => {
    const c = setup();
    renderSettings();
    fireEvent.click(await screen.findByTestId('ns-memory'));
    await screen.findByTestId('namespace-page');
    fireEvent.click(screen.getByTestId('ext-unipi.memory').querySelector('input[role=switch]') as HTMLInputElement);
    await waitFor(() => expect(c.extensions[0]!.enabled).toBe(false));
    // Stored values survive the disable …
    expect((c.memory.global ?? {}).topK).toBe(20);
    // … and the namespace row is hidden in the list.
    fireEvent.click(screen.getByTestId('settings-back'));
    await waitFor(() => expect(screen.queryByTestId('ns-memory')).toBeNull());
    // The extension itself stays listed, with its toggle off.
    const row = await screen.findByTestId('ext-unipi.memory');
    expect((row.querySelector('input[role=switch]') as HTMLInputElement).checked).toBe(false);
  });

  it('writes a field through _oino/set_extension_setting', async () => {
    const c = setup();
    renderSettings();
    fireEvent.click(await screen.findByTestId('ns-memory'));
    const row = await waitFor(() => {
      expect(document.querySelector('.settings-form[data-loaded]')).toBeTruthy();
      const element = document.getElementById('field-memory-topK');
      expect(element).toBeTruthy();
      return element!;
    });
    fireEvent.click(row.querySelector('button[aria-label^="Increase"]')!);
    await waitFor(() => expect((c.memory.global ?? {}).topK).toBe(21));
  });
});

describe('Settings › Agent (WP-U11)', () => {
  it('renders the core oino.* namespaces with the Global | Project scope toggle', async () => {
    const c = setup();
    location.hash = '#/settings/agent';
    renderSettings();
    await waitFor(() => expect(document.getElementById('field-oino_model-defaultModel')).toBeTruthy());
    expect(screen.getByTestId('form-oino.model').textContent).toContain('Model & thinking');
    // Global view: the global override; project view: the project file's value wins.
    const input = () => document.querySelector('#field-oino_model-defaultModel input') as HTMLInputElement;
    await waitFor(() => expect(input().value).toBe('glm-4.7'));
    fireEvent.click(screen.getByTestId('scope-project'));
    await waitFor(() => expect(input().value).toBe('glm-4.7-flash'));
    // Writing in the project view targets the project layer only.
    fireEvent.change(input(), { target: { value: 'glm-4.7-air' } });
    await waitFor(() => expect(c.model.project?.defaultModel).toBe('glm-4.7-air'));
    expect(c.model.global?.defaultModel).toBe('glm-4.7');
  });
});

describe('Settings search covers namespace fields (WP-U11)', () => {
  it('finds a field label and lands on its row inside the namespace page', async () => {
    setup();
    renderSettings();
    const input = await screen.findByTestId('settings-search-input');
    await fireEvent.input(input, { target: { value: 'Results per search' } });
    const hit = await waitFor(() => {
      const hits = screen.getAllByTestId('settings-hit-extension');
      expect(hits.length).toBeGreaterThan(0);
      return hits[0]!;
    });
    expect(hit.textContent).toContain('memory.topK');
    fireEvent.click(hit);
    await waitFor(() => expect(screen.getByTestId('namespace-page')).toBeTruthy());
    const row = await waitFor(() => {
      const element = document.getElementById('field-memory-topK');
      expect(element).toBeTruthy();
      return element!;
    });
    expect(row.classList.contains('settings-flash-target')).toBe(true);
  });

  it('finds a core field and opens Agent', async () => {
    setup();
    renderSettings();
    const input = await screen.findByTestId('settings-search-input');
    await fireEvent.input(input, { target: { value: 'Default model' } });
    const hit = await waitFor(() => {
      const hits = screen.getAllByTestId('settings-hit-agent');
      expect(hits.length).toBeGreaterThan(0);
      return hits[0]!;
    });
    expect(hit.textContent).toContain('oino.model.defaultModel');
    fireEvent.click(hit);
    await waitFor(() => expect(document.getElementById('field-oino_model-defaultModel')).toBeTruthy());
    expect(document.getElementById('sec-agent')).toBeTruthy();
  });
});
