// @vitest-environment jsdom
import { describe, it, expect } from 'vitest';
import { render, waitFor } from '@solidjs/testing-library';
import '../../test-setup';
import { SettingsProvider, useSettings, useSettingsApi } from '../context';
import { createMemoryAdapter } from '../adapter';
import { DEFAULT_SETTINGS } from '../schema';

function Probe(props: { onReady: (api: ReturnType<typeof useSettingsApi>) => void }) {
  const [settings] = useSettings();
  const api = useSettingsApi();
  props.onReady(api);
  return <div data-testid="font-size">{settings.terminal.fontSize}</div>;
}

describe('SettingsProvider / useSettings', () => {
  it('loads defaults through an injected adapter', async () => {
    const adapter = createMemoryAdapter(undefined);
    let api: ReturnType<typeof useSettingsApi> | undefined;
    const r = render(() => (
      <SettingsProvider adapter={adapter}>
        <Probe onReady={(a) => (api = a)} />
      </SettingsProvider>
    ));
    await waitFor(() => expect(r.getByTestId('font-size').textContent).toBe('14'));
    expect(api!.settings).toEqual(DEFAULT_SETTINGS);
  });

  it('update() applies a valid single-key change and persists it', async () => {
    const adapter = createMemoryAdapter(undefined);
    let api: ReturnType<typeof useSettingsApi> | undefined;
    const r = render(() => (
      <SettingsProvider adapter={adapter}>
        <Probe onReady={(a) => (api = a)} />
      </SettingsProvider>
    ));
    await waitFor(() => expect(r.getByTestId('font-size').textContent).toBe('14'));

    const err = api!.update('terminal.cursorBlink', false);
    expect(err).toBeUndefined();
    expect(api!.settings.terminal.cursorBlink).toBe(false);

    await waitFor(async () => {
      const persisted = (await adapter.read()) as { terminal: { cursorBlink: boolean } };
      expect(persisted.terminal.cursorBlink).toBe(false);
    });
  });

  it('update() rejects an invalid value with E_SETTINGS and does not save', async () => {
    const adapter = createMemoryAdapter(undefined);
    let api: ReturnType<typeof useSettingsApi> | undefined;
    const r = render(() => (
      <SettingsProvider adapter={adapter}>
        <Probe onReady={(a) => (api = a)} />
      </SettingsProvider>
    ));
    await waitFor(() => expect(r.getByTestId('font-size').textContent).toBe('14'));

    const err = api!.update('terminal.fontSize', 999);
    expect(err).toBeDefined();
    expect(err?.code).toBe('E_SETTINGS');
    expect(api!.settings.terminal.fontSize).toBe(14);
    expect(api!.error()).toEqual(err);

    const persisted = await adapter.read();
    expect(persisted).toBeUndefined();
  });
});

describe('every settings group is updatable (UNI-173)', () => {
  // `notify` once existed in the schema but was missing from the update
  // registry, so its switches silently did nothing. This loops over every
  // group of the schema (via DEFAULT_SETTINGS) so a future group can't
  // regress the same way.
  it('update() accepts a no-op write for the first key of every group', async () => {
    const adapter = createMemoryAdapter(undefined);
    let api: ReturnType<typeof useSettingsApi> | undefined;
    const r = render(() => (
      <SettingsProvider adapter={adapter}>
        <Probe onReady={(a) => (api = a)} />
      </SettingsProvider>
    ));
    await waitFor(() => expect(r.getByTestId('font-size').textContent).toBe('14'));

    for (const [group, value] of Object.entries(DEFAULT_SETTINGS)) {
      if (group === 'version' || !value || typeof value !== 'object') continue;
      const firstKey = Object.keys(value as object)[0]!;
      const current = (api!.settings as unknown as Record<string, Record<string, unknown>>)[group]![firstKey];
      const err = api!.update(`${group}.${firstKey}`, current);
      expect(err, `${group}.${firstKey} must be updatable`).toBeUndefined();
    }
  });

  it("update('notify.enabled', true) applies and persists — the phone-notification switch", async () => {
    const adapter = createMemoryAdapter(undefined);
    let api: ReturnType<typeof useSettingsApi> | undefined;
    const r = render(() => (
      <SettingsProvider adapter={adapter}>
        <Probe onReady={(a) => (api = a)} />
      </SettingsProvider>
    ));
    await waitFor(() => expect(r.getByTestId('font-size').textContent).toBe('14'));

    expect(api!.update('notify.enabled', true)).toBeUndefined();
    expect(api!.settings.notify.enabled).toBe(true);
    expect(api!.update('notify.keepListening', true)).toBeUndefined();
    expect(api!.settings.notify.keepListening).toBe(true);

    await waitFor(async () => {
      const persisted = (await adapter.read()) as { notify: { enabled: boolean; keepListening: boolean } };
      expect(persisted.notify.enabled).toBe(true);
      expect(persisted.notify.keepListening).toBe(true);
    });
  });
});
