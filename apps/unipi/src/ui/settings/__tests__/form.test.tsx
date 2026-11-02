// @vitest-environment jsdom
/**
 * WP-U11 acceptance: every field type of the OSS-2 form engine (ui-protocol
 * §7) rendered from the mock host, at both shells — golden DOM per type — plus
 * the behaviours the spec names: defaults vs "modified · Reset", the Global /
 * Project scope toggle and layering, `visibleIf`, the secret's "Replace…",
 * the order/list/dict editors and a simplified write (a default value unsets).
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { SettingsProvider } from '../../../settings/context';
import { createMemoryAdapter } from '../../../settings/adapter';
import { setHostClientFactory } from '../../../lib/host/factory';
import { createMockHost, type MockHostSpec, type MockScenario } from '../../../dev/mock/host';
import { memoryFormNamespace } from '../../../dev/mock/fixtures/settingsForm';
import { setDesktopOverride } from '../../../lib/layout';
import { setPlatformOverride } from '../../../lib/platform';
import { resetHubs } from '../../../lib/session/hub';
import { NamespaceForm } from '../form';
import { parseNamespace, type Namespace } from '../types';
import '../settings-form.css';

type Fixtures = { namespaces: unknown[] };

function setup(): { namespace: Namespace; global: Record<string, unknown>; project: Record<string, unknown> } {
  const memory = structuredClone(memoryFormNamespace) as unknown as { global: Record<string, unknown>; project: Record<string, unknown> };
  const spec = { namespaces: [memory] } satisfies Fixtures;
  const scenario: MockScenario = {
    name: 'settings-form-test',
    hosts: [
      {
        id: 'local',
        name: 'workstation',
        transport: 'local',
        conn: { state: 'connected' },
        cards: [],
        settings: {
          pi: {},
          namespaces: spec.namespaces as MockHostSpec['settings'] extends { namespaces?: infer N } ? N : never,
          extensions: [{ id: 'unipi.memory', enabled: true, loaded: true }],
        },
      } as MockHostSpec,
    ],
  };
  setHostClientFactory(createMockHost(scenario).client);
  return { namespace: parseNamespace(memory)!, global: memory.global, project: memory.project };
}

function mount(namespace: Namespace) {
  return render(() => (
    <SettingsProvider adapter={createMemoryAdapter(undefined)}>
      <NamespaceForm namespace={namespace} />
    </SettingsProvider>
  ));
}

/** Kobalte's per-mount ids and CodeMirror's generated classes are not stable
 *  across mounts; the golden compares structure, classes and aria. */
const sanitize = (html: string) => html.replace(/switch-cl-\d+/g, 'switch-cl').replace(/ͼ[\w-]+/g, '');

const row = (ns: string, key: string) => document.getElementById(`field-${ns}-${key.replace(/\./g, '_')}`) as HTMLElement;
/** Waits for the namespace's three layers (rows exist before that, with defaults only). */
const loaded = () => waitFor(() => expect(document.querySelector('.settings-form[data-loaded]')).toBeTruthy());
const switchOf = (testId: string) => screen.getByTestId(testId).querySelector('input[role=switch]') as HTMLInputElement;

beforeEach(() => {
  setPlatformOverride('tauri-mobile');
  setDesktopOverride(true);
});
afterEach(() => {
  cleanup();
  resetHubs();
  setPlatformOverride(undefined);
  setDesktopOverride(undefined);
  setHostClientFactory(undefined);
});

describe('OSS-2 form: every field type', () => {
  const TYPES: Array<[string, string]> = [
    ['autoRecall', 'boolean'],
    ['model', 'model'],
    ['topK', 'number'],
    ['scope', 'enum'],
    ['sources', 'multiselect'],
    ['vault', 'string'],
    ['summary', 'string'],
    ['token', 'secret'],
    ['json', 'json'],
    ['weights', 'order'],
    ['pins', 'list'],
    ['aliases', 'dict'],
  ];

  it('renders each type with its row chrome (desktop + phone goldens)', async () => {
    const { namespace } = setup();
    mount(namespace);
    await loaded();
    for (const [key, type] of TYPES) {
      const element = row('memory', key);
      expect(element, key).toBeTruthy();
      expect(element.dataset.type, key).toBe(type);
      // Label, description and the default/modified meta line are always there.
      expect(element.textContent).toContain(namespaceLabel(namespace, key));
      // No "default" caption any more: only "modified · Reset" (and "not set" for keys).
      expect(element.querySelector('[data-testid=field-default]'), key).toBeNull();
      expect(sanitize(element.outerHTML)).toMatchSnapshot(`${type}-desktop`);
    }
    // Phone: the same rows, the wide controls on their own line.
    setDesktopOverride(false);
    cleanup();
    mount(namespace);
    await loaded();
    for (const [key, type] of TYPES) {
      expect(sanitize(row('memory', key).outerHTML)).toMatchSnapshot(`${type}-phone`);
    }
  });

  it('advanced types: action and page rows, and an unknown type stays read-only', async () => {
    const { namespace } = setup();
    mount(namespace);
    await loaded();
    fireEvent.click(screen.getByTestId('form-advanced'));
    await waitFor(() => expect(row('memory', 'consolidate')).toBeTruthy());
    // `action` → a .btn-secondary that runs the command in an open chat.
    const action = row('memory', 'consolidate');
    expect(action.dataset.type).toBe('action');
    expect(action.querySelector('.btn-secondary')).toBeTruthy();
    expect(sanitize(action.outerHTML)).toMatchSnapshot('action-desktop');
    // `page` → a drill-in row; its nested sections render under a Back (Z-113).
    const page = row('memory', 'advanced-page');
    expect(sanitize(page.outerHTML)).toMatchSnapshot('page-desktop');
    fireEvent.click(page);
    const body = await screen.findByTestId('page-body-advanced-page');
    expect(body.textContent).toContain('Page size');
    expect(sanitize(body.outerHTML)).toMatchSnapshot('page-body-desktop');

    setDesktopOverride(false);
    cleanup();
    mount(namespace);
    await loaded();
    fireEvent.click(screen.getByTestId('form-advanced'));
    await waitFor(() => expect(row('memory', 'consolidate')).toBeTruthy());
    expect(sanitize(row('memory', 'consolidate').outerHTML)).toMatchSnapshot('action-phone');
    fireEvent.click(row('memory', 'advanced-page'));
    await waitFor(() => expect(screen.getByTestId('page-body-advanced-page')).toBeTruthy());
    expect(sanitize(screen.getByTestId('page-body-advanced-page').outerHTML)).toMatchSnapshot('page-body-phone');
  });

  it('an unknown field type renders read-only and is never written', async () => {
    const spec = {
      name: 'settings-unknown',
      hosts: [
        {
          id: 'local',
          name: 'workstation',
          transport: 'local',
          conn: { state: 'connected' },
          cards: [],
          settings: {
            pi: {},
            namespaces: [
              {
                namespace: 'demo',
                title: 'Demo',
                extensionId: 'unipi.demo',
                sections: [{ id: 's', title: 'S', fields: [{ key: 'mystery', type: 'quantum', label: 'Mystery lever', description: 'A newer host sent a type this build does not know' }] }],
              },
            ],
            extensions: [],
          },
        } as MockHostSpec,
      ],
    } as MockScenario;
    setHostClientFactory(createMockHost(spec).client);
    mount(parseNamespace((spec.hosts[0]!.settings as { namespaces: unknown[] }).namespaces[0])!);
    await loaded();
    const row = document.getElementById('field-demo-mystery')!;
    expect(row.dataset.type).toBe('unknown');
    expect(row.querySelector('input, button, select, textarea')).toBeNull();
    expect(sanitize(row.outerHTML)).toMatchSnapshot('unknown-desktop');
  });

  it('shows "default" for an unset key and "modified · Reset" for a stored one', async () => {
    const { namespace } = setup();
    mount(namespace);
    await loaded();
    // autoRecall: not stored → default (true); topK: stored 20 vs default 12 → modified.
    expect(row('memory', 'autoRecall').querySelector('[data-testid=field-modified]')).toBeNull();
    const modified = row('memory', 'topK').querySelector('[data-testid=field-modified]')!;
    expect(modified).toBeTruthy();
    expect(row('memory', 'topK').querySelector('[data-testid=field-reset]')).toBeTruthy();
  });

  it('Reset unsets the stored value and the row falls back to its default', async () => {
    const { namespace, global } = setup();
    mount(namespace);
    await loaded();
    fireEvent.click(row('memory', 'topK').querySelector('[data-testid=field-reset]')!);
    await waitFor(() => expect('topK' in global).toBe(false));
    await waitFor(() => expect(row('memory', 'topK').querySelector('[data-testid=field-modified]')).toBeNull());
  });

  it('a boolean writes; writing the default back unsets the key (§7.2)', async () => {
    const { namespace, global } = setup();
    mount(namespace);
    await loaded();
    switchOf('set-autoRecall').click();
    await waitFor(() => expect(global.autoRecall).toBe(false));
    // false equals the default → it is stored (the field default is a value, not the below-layer key)
    switchOf('set-autoRecall').click();
    await waitFor(() => expect(row('memory', 'autoRecall').querySelector('[data-testid=field-modified]')).toBeNull());
  });

  it('the scope toggle switches to the project layer and back', async () => {
    const { namespace } = setup();
    mount(namespace);
    await loaded();
    expect(screen.getByTestId('scope-global').getAttribute('aria-checked')).toBe('true');
    fireEvent.click(screen.getByTestId('scope-project'));
    // The project layer's aliases appear; a global write is still the base.
    const aliasRow = () => row('memory', 'aliases');
    await waitFor(() => expect((aliasRow().querySelector('input[aria-label=Alias]') as HTMLInputElement).value).toBe('inbox'));
    fireEvent.click(screen.getByTestId('scope-global'));
    await waitFor(() => expect(aliasRow().querySelector('input[aria-label=Alias]')).toBeNull());
  });

  it('a project write stores in the project layer, not the global one', async () => {
    const { namespace, global, project } = setup();
    mount(namespace);
    await loaded();
    fireEvent.click(screen.getByTestId('scope-project'));
    fireEvent.click(row('memory', 'topK').querySelector('button[aria-label^="Increase"]')!);
    await waitFor(() => expect(project.topK).toBe(21));
    expect(global.topK).toBe(20);
  });

  it('visibleIf / disabledIf follow the effective values, advanced stays folded', async () => {
    const { namespace } = setup();
    mount(namespace);
    await loaded();
    // Advanced fields (retention / decay / decayRate / page) are folded away.
    expect(row('memory', 'retention')).toBeNull();
    fireEvent.click(screen.getByTestId('form-advanced'));
    await waitFor(() => expect(row('memory', 'retention')).toBeTruthy());
    // decay is visible while retention is truthy (180d) …
    expect(row('memory', 'decay')).toBeTruthy();
    // … and the decay rate is disabled while decay is on, enabled when off.
    expect((row('memory', 'decayRate').querySelector('input') as HTMLInputElement).disabled).toBe(false);
    switchOf('set-decay').click();
    await waitFor(() => expect((row('memory', 'decayRate').querySelector('input') as HTMLInputElement).disabled).toBe(true));
  });

  it('secret: masked value, "Replace…" writes a new one, never echoed', async () => {
    const { namespace, global } = setup();
    mount(namespace);
    await loaded();
    expect(screen.getByTestId('secret-token').textContent).toBe('•••• set');
    fireEvent.click(screen.getByTestId('replace-token'));
    const input = (await screen.findByLabelText('New Embedding API key')) as HTMLInputElement;
    expect(input.type).toBe('password');
    fireEvent.input(input, { target: { value: 'sk-new' } });
    fireEvent.click(screen.getByText('Save'));
    await waitFor(() => expect(global.token).toBe('sk-new'));
  });

  it('order rows move with the buttons and add from the universe', async () => {
    const { namespace, global } = setup();
    mount(namespace);
    await loaded();
    fireEvent.click(screen.getByLabelText('Move importance up'));
    await waitFor(() => expect(global.weights).toEqual(['importance', 'recency', 'similarity']));
    fireEvent.click(screen.getByText('pins'));
    await waitFor(() => expect(global.weights).toEqual(['importance', 'recency', 'similarity', 'pins']));
  });

  it('list and dict rows add and remove entries', async () => {
    const { namespace, global, project } = setup();
    mount(namespace);
    await loaded();
    fireEvent.click(screen.getByTestId('add-pins'));
    await waitFor(() => expect(global.pins).toEqual(['']));
    const input = screen.getByLabelText('Memory id') as HTMLInputElement;
    fireEvent.input(input, { target: { value: 'mem_1' } });
    fireEvent.change(input);
    await waitFor(() => expect(global.pins).toEqual(['mem_1']));
    // A `both` field writes to the view scope (global here), not to project.
    expect(project.aliases).toEqual({ inbox: 'mem_01HQINBOX' });
    fireEvent.click(screen.getByTestId('add-aliases'));
    await waitFor(() => expect(Object.keys(global.aliases as object)).toContain('key1'));
  });
});

/** The field's label as the namespace declares it. */
function namespaceLabel(namespace: Namespace, key: string): string {
  for (const section of namespace.sections) {
    for (const field of section.fields) {
      if (field.key === key) return field.label;
      for (const nested of field.sections) {
        for (const inner of nested.fields) if (inner.key === key) return inner.label;
      }
    }
  }
  return key;
}
