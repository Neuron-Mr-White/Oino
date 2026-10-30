/**
 * Settings scenarios (WP-A24):
 *
 *   settings-nav         the two-column layout — grouped 264 px nav, the
 *                        Extensions section open (toggles + namespaces)
 *   settings-search      the global search with results across all four
 *                        sources (app rows, host keys, pi keys, extension
 *                        namespaces) for the query "memory"
 *   settings-extensions  the extension list with enable toggles, one
 *                        disabled-with-reason row and one loaded provider
 *   settings-safe-mode   the safe-mode banner naming its suspects (G-19):
 *                        a retained warning notice the hub replays
 *
 * `setup` navigates to the Settings route (and drives the search input for
 * `settings-search`, like the files scenarios drive their panels).
 */
import type { MockHostSpec, MockScenario } from '../host';

const HOST = 'local';

const PI_SETTINGS = {
  defaultModel: 'glm-4.7',
  theme: 'unipi-dark',
  disabledExtensions: [],
  extensionSettings: {
    memory: { model: 'gpt-5-mini', topK: 12 },
    compactor: { keepLines: 400 },
  },
};

const NAMESPACES = [
  {
    namespace: 'memory',
    title: 'Memory',
    description: 'What the memory extension remembers and how it searches',
    extensionId: 'unipi.memory',
    fields: [
      { key: 'model', type: 'string', title: 'Model', description: 'The model that writes memories' },
      { key: 'topK', type: 'number', title: 'Results per search', description: 'How many memories a search returns' },
    ],
  },
  {
    namespace: 'compactor',
    title: 'Compaction',
    description: 'How old turns compact when the context fills',
    extensionId: 'unipi.compactor',
    fields: [{ key: 'keepLines', type: 'number', title: 'Lines to keep' }],
  },
  {
    namespace: 'notify',
    title: 'Notifications',
    description: 'Needs-you fan-out over ntfy',
    extensionId: 'unipi.notify',
    fields: [{ key: 'topic', type: 'secret', title: 'Topic' }],
  },
];

const EXTENSIONS = [
  { id: 'unipi.memory', enabled: true, loaded: true, path: '~/.oino/extensions/unipi.memory' },
  { id: 'unipi.notify', enabled: true, loaded: true, path: '~/.oino/extensions/unipi.notify' },
  { id: 'unipi.cron', enabled: false, loaded: false, note: 'Disabled — it trapped in two sessions (wasm trap in schedule_tick)' },
  { id: 'openai-compat', kind: 'provider', enabled: true, loaded: true, path: '~/.oino/extensions/openai-compat' },
];

const HOST_SETTINGS = {
  relay: 'n0',
  relay_only: false,
  forwarding: { enabled: true, ports: [5173, 3000], devices: [] },
  push: { enabled: true, devices: ['pixel-9'] },
  fs: { allow: ['/srv/oino'], deny: [] },
  term: { max: 16 },
  update: { mode: 'prompt' },
};

const host = (extra: Partial<MockHostSpec> = {}): MockHostSpec => ({
  id: HOST,
  name: 'workstation',
  transport: 'local' as const,
  conn: { state: 'connected' as const },
  cards: [],
  settings: { pi: PI_SETTINGS, namespaces: NAMESPACES, extensions: EXTENSIONS, host: HOST_SETTINGS },
  ...extra,
});

function goSettings(section?: string): void {
  location.hash = section ? `#/settings/${section}` : '#/settings';
}

/** Drives the real search input once mounted (the capture must show results). */
function searchFor(query: string): void {
  const drive = () => {
    const input = document.querySelector('[data-testid="settings-search-input"]') as HTMLInputElement | null;
    if (!input) return false;
    input.focus();
    input.value = query;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    return true;
  };
  const untilMounted = setInterval(() => {
    if (drive()) clearInterval(untilMounted);
  }, 50);
  setTimeout(() => clearInterval(untilMounted), 5000);
}

export const settingsNav: MockScenario = {
  name: 'settings-nav',
  hosts: [host()],
  // The capture URL picks the section (extensions by default, about for the
  // Update card shot); only navigate when the harness URL has no route.
  setup: () => {
    if (!location.hash.startsWith('#/settings')) goSettings('extensions');
  },
};

export const settingsSearch: MockScenario = {
  name: 'settings-search',
  hosts: [host()],
  setup: () => {
    goSettings();
    searchFor('memory');
  },
};

export const settingsExtensions: MockScenario = {
  name: 'settings-extensions',
  hosts: [
    host({
      notices: [
        {
          level: 'warning' as const,
          text: 'unipi.cron crashed in a session: wasm trap in schedule_tick. It is disabled for this session (DIRECTION §3.3).',
        },
      ],
    }),
  ],
  setup: () => goSettings('extensions'),
};

export const settingsSafeMode: MockScenario = {
  name: 'settings-safe-mode',
  hosts: [
    host({
      notices: [
        {
          level: 'warning' as const,
          text: 'Safe mode: extensions are off after 2 failed starts; suspects: unipi.cron (wasm trap in schedule_tick), unipi.dream (timeout in init)',
        },
      ],
    }),
  ],
  setup: () => goSettings('extensions'),
};
