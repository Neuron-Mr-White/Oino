// @vitest-environment jsdom
/**
 * w5-ui D1: Settings › Memory crashed with "Stale read from <Show>" — the lazily loaded CodeMirror of a `json`
 * field read Show-accessor props after its `await import(...)`. Opening any page with a JsonField must render,
 * also when the user leaves before the import resolves or the data changes under it.
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, render, waitFor } from '@solidjs/testing-library';
import { createSignal, Show } from 'solid-js';
import { EditorView } from '@codemirror/view';
import { SettingsProvider } from '../../../settings/context';
import { createMemoryAdapter } from '../../../settings/adapter';
import { setHostClientFactory } from '../../../lib/host/factory';
import { createMockHost, type MockHostSpec, type MockScenario } from '../../../dev/mock/host';
import { memoryFormNamespace } from '../../../dev/mock/fixtures/settingsForm';
import { setDesktopOverride } from '../../../lib/layout';
import { setPlatformOverride } from '../../../lib/platform';
import { resetHubs } from '../../../lib/session/hub';
import { NamespaceForm } from '../form';
import { parseNamespace } from '../types';
import '../settings-form.css';

function host() {
  const memory = structuredClone(memoryFormNamespace) as unknown as Record<string, unknown>;
  const scenario: MockScenario = {
    name: 'json-field-test',
    hosts: [
      {
        id: 'local',
        name: 'workstation',
        transport: 'local',
        conn: { state: 'connected' },
        cards: [],
        settings: { pi: {}, namespaces: [memory], extensions: [{ id: 'unipi.memory', enabled: true, loaded: true }] },
      } as unknown as MockHostSpec,
    ],
  };
  setHostClientFactory(createMockHost(scenario).client);
  return parseNamespace(memory)!;
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 30));
const errors: unknown[] = [];
const onError = (e: PromiseRejectionEvent | ErrorEvent) => errors.push(e);

beforeEach(() => {
  errors.length = 0;
  setPlatformOverride('tauri-mobile');
  setDesktopOverride(true);
  window.addEventListener('unhandledrejection', onError);
  window.addEventListener('error', onError);
});
afterEach(() => {
  window.removeEventListener('unhandledrejection', onError);
  window.removeEventListener('error', onError);
  cleanup();
  resetHubs();
  setPlatformOverride(undefined);
  setDesktopOverride(undefined);
  setHostClientFactory(undefined);
});

describe('JsonField (lazy CodeMirror)', () => {
  it('renders the Memory form inside a <Show> accessor that goes away and comes back', async () => {
    const namespace = host();
    const [ns, setNs] = createSignal<typeof namespace | undefined>(namespace);
    const view = render(() => (
      <SettingsProvider adapter={createMemoryAdapter(undefined)}>
        <Show when={ns()}>{(current) => <NamespaceForm namespace={current()} />}</Show>
      </SettingsProvider>
    ));
    // Leave before the dynamic import resolves, return, and leave again mid-flight.
    setNs(undefined);
    await flush();
    setNs(namespace);
    setNs(undefined);
    setNs(namespace);
    await waitFor(() => expect(view.container.querySelector('#field-memory-json .cm-editor')).toBeTruthy());
    await flush();
    expect(errors).toEqual([]);
    expect(view.container.querySelector('.error-boundary, [data-testid=error-screen]')).toBeNull();
  });

  it('keeps editing against the live value after the editor loads (no stale prop read in the listener)', async () => {
    const namespace = host();
    const view = render(() => (
      <SettingsProvider adapter={createMemoryAdapter(undefined)}>
        <NamespaceForm namespace={namespace} />
      </SettingsProvider>
    ));
    await waitFor(() => expect(view.container.querySelector('#field-memory-json .cm-editor')).toBeTruthy());
    const editor = EditorView.findFromDOM(view.container.querySelector('#field-memory-json .cm-editor') as HTMLElement)!;
    editor.dispatch({ changes: { from: 0, to: editor.state.doc.length, insert: '{"a":1}' } });
    await flush();
    expect(errors).toEqual([]);
  });

  it('leaving before the import resolves builds nothing and throws nothing', async () => {
    const namespace = host();
    const [on, setOn] = createSignal(true);
    const view = render(() => (
      <SettingsProvider adapter={createMemoryAdapter(undefined)}>
        <Show when={on()}>
          <NamespaceForm namespace={namespace} />
        </Show>
      </SettingsProvider>
    ));
    setOn(false);
    await flush();
    expect(view.container.querySelector('.cm-editor')).toBeNull();
    expect(errors).toEqual([]);
  });
});
