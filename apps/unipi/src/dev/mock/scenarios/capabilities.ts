/**
 * WP-A25 harness-reachability scenarios (G-8..G-11) + Providers (G-15):
 *
 *   cap-mcp        Settings › MCP — the `mcp` namespace form above the
 *                  extension's Manage rows (status, servers, catalog, sync, reload)
 *   cap-memory     Settings › Memory — recall/write form + browse/consolidate
 *   cap-skills     Settings › Skills — exposure/pins/vault form + hub/editor
 *   cap-agents     Settings › Subagents — concurrency form + profile manager
 *   cap-panels     the memory **browse panel** as an overlay entry (a typed
 *                  `/memory` opens it; the tree mirrors memory/src/panel.rs)
 *   providers      Settings › Providers with signed-in, API-key, env and
 *                  unconfigured rows
 *
 * The capability pages are chatless-safe (R8): every scenario keeps one chat
 * open so the capture shows the enabled Manage rows, and `cap-mcp-nochat`
 * shows the "Open a chat to manage" state. `setup` lands on the page.
 */
import type { MockHostSpec, MockNamespace, MockScenario } from '../host';
import { genCards, genFsTree, genSession } from '../gen';

const HOST = 'local';
const CWD = '/home/you/projects/unipi';
const SESSION = 'cap001';

/** The commands the four capability extensions register (Manage… sends these). */
const CAP_COMMANDS = [
  { name: 'mcp-status', description: 'Show status of all configured MCP servers', source: 'extension' },
  { name: 'mcp-settings', description: 'Open the MCP server registry', source: 'extension' },
  { name: 'mcp-add', description: 'Add an MCP server from the catalog', source: 'extension' },
  { name: 'mcp-sync', description: 'Sync the MCP server catalog', source: 'extension' },
  { name: 'mcp-reload', description: 'Reconnect every MCP server', source: 'extension' },
  { name: 'memory', description: 'Browse the memory store', source: 'extension' },
  { name: 'memory-consolidate', description: 'Consolidate the session into memory', source: 'extension' },
  { name: 'global-memory-list', description: 'List all memories across all projects', source: 'extension' },
  { name: 'skills', description: 'Skill settings — proxy, exposure, per-skill pins', source: 'extension' },
  { name: 'skills-editor', description: 'Per-skill Enabled / Must show editor', source: 'extension' },
  { name: 'agents', description: 'Manage subagent profiles', source: 'extension' },
  { name: 'model', description: 'Pick the model', source: 'builtin' },
];

const MCP_NAMESPACE: MockNamespace = {
  namespace: 'mcp',
  title: 'MCP',
  description: 'Model Context Protocol servers, the catalog and tool filters',
  extensionId: 'unipi.mcp',
  icon: 'plug',
  order: 40,
  defaults: { enabled: true, catalogSource: 'github', reloadOnStart: true, toolFilter: '' },
  sections: [
    {
      id: 'servers',
      title: 'Servers',
      fields: [
        { key: 'enabled', type: 'boolean', label: 'Load MCP servers', default: true, description: 'Off = no server tools, the registry stays' },
        { key: 'reloadOnStart', type: 'boolean', label: 'Reconnect on session start', default: true },
        { key: 'toolFilter', type: 'string', label: 'Tool filter', description: 'Only register tools whose name matches', placeholder: 'e.g. playwright_*' },
      ],
    },
    {
      id: 'catalog',
      title: 'Catalog',
      fields: [
        {
          key: 'catalogSource',
          type: 'enum',
          label: 'Catalog source',
          default: 'github',
          options: [
            { value: 'github', label: 'GitHub (official)' },
            { value: 'bundled', label: 'Bundled copy' },
            { value: 'off', label: 'Off' },
          ],
        },
        { key: 'customUrl', type: 'string', label: 'Custom catalog URL', visibleIf: { key: 'catalogSource', equals: ['github'] }, placeholder: 'https://…' },
      ],
    },
  ],
  global: { toolFilter: 'playwright_*' },
};

const MEMORY_NAMESPACE: MockNamespace = {
  namespace: 'memory',
  title: 'Memory',
  description: 'What the memory extension remembers and how it searches',
  extensionId: 'unipi.memory',
  icon: 'brain',
  order: 30,
  defaults: { recallAtStart: true, write: true, saveMode: 'side', topK: 12 },
  sections: [
    {
      id: 'recall',
      title: 'Recall',
      fields: [
        { key: 'recallAtStart', type: 'boolean', label: 'Recall at the start of a session', default: true, description: 'Adds a memory reminder to the first turn' },
        { key: 'wakeUp', type: 'boolean', label: 'Include the wake-up text', default: true, visibleIf: { key: 'recallAtStart', equals: [true] } },
        { key: 'topK', type: 'number', label: 'Results per search', default: 12, min: 1, max: 50, step: 1, unit: 'memories', integer: true },
      ],
    },
    {
      id: 'write',
      title: 'Write',
      fields: [
        { key: 'write', type: 'boolean', label: 'Let the agent save memories', default: true },
        {
          key: 'saveMode',
          type: 'enum',
          label: 'End-of-task save',
          default: 'side',
          options: [
            { value: 'side', label: 'Side agent', description: 'A background pass writes what is worth keeping' },
            { value: 'reminder', label: 'Reminder', description: 'Ask the agent next turn' },
            { value: 'off', label: 'Off' },
          ],
        },
      ],
    },
  ],
  global: { topK: 8 },
};

const SKILLS_NAMESPACE: MockNamespace = {
  namespace: 'skills',
  title: 'Skills',
  description: 'Exposure, pins and the vault',
  extensionId: 'unipi.skill-registry',
  icon: 'sparkles',
  order: 50,
  defaults: { exposure: 'judged', maxVisible: 12, vault: [] },
  sections: [
    {
      id: 'exposure',
      title: 'Exposure',
      fields: [
        {
          key: 'exposure',
          type: 'enum',
          label: 'Which skills the agent sees',
          default: 'judged',
          options: [
            { value: 'judged', label: 'Judged', description: 'A judge picks the skills for each turn' },
            { value: 'all', label: 'All', description: 'Every skill is listed' },
            { value: 'off', label: 'Off', description: 'No skills are listed (none are removed)' },
          ],
        },
        { key: 'maxVisible', type: 'number', label: 'Skills per turn', default: 12, min: 1, max: 40, step: 1, integer: true },
        {
          key: 'vault',
          type: 'list',
          label: 'Vault',
          description: 'Skills kept out of the judge’s pool',
          items: { type: 'string' },
          default: [],
        },
      ],
    },
    {
      id: 'pins',
      title: 'Pins',
      fields: [
        {
          key: 'mustShow',
          type: 'multiselect',
          label: 'Must show',
          description: 'Always listed, whatever the judge decides',
          default: ['editor'],
          options: [
            { value: 'editor', label: 'editor' },
            { value: 'memory', label: 'memory' },
            { value: 'kanboard', label: 'kanboard' },
          ],
        },
      ],
    },
  ],
  global: { mustShow: ['editor', 'memory'] },
};

const SUBAGENTS_NAMESPACE: MockNamespace = {
  namespace: 'subagents',
  title: 'Subagents',
  description: 'Agent profiles, concurrency and defaults',
  extensionId: 'unipi.subagents',
  icon: 'bot',
  order: 60,
  defaults: { enabled: true, maxConcurrent: 8, defaultThinking: 'inherit', defaultModel: '' },
  sections: [
    {
      id: 'run',
      title: 'Running',
      fields: [
        { key: 'enabled', type: 'boolean', label: 'Allow subagents', default: true },
        { key: 'maxConcurrent', type: 'number', label: 'At most at once', default: 8, min: 1, max: 40, step: 1, integer: true, restart: true },
        { key: 'defaultModel', type: 'model', label: 'Default model', description: 'Empty = inherit the parent’s model', placeholder: 'inherit' },
        {
          key: 'defaultThinking',
          type: 'enum',
          label: 'Default thinking',
          default: 'inherit',
          options: [
            { value: 'inherit', label: 'Inherit' },
            { value: 'low', label: 'Low' },
            { value: 'medium', label: 'Medium' },
            { value: 'high', label: 'High' },
          ],
        },
      ],
    },
  ],
  global: { maxConcurrent: 6 },
};

const EXTENSIONS = [
  { id: 'unipi.mcp', enabled: true, loaded: true, path: '~/.oino/extensions/unipi.mcp' },
  { id: 'unipi.memory', enabled: true, loaded: true, path: '~/.oino/extensions/unipi.memory' },
  { id: 'unipi.skill-registry', enabled: true, loaded: true, path: '~/.oino/extensions/unipi.skill-registry' },
  { id: 'unipi.subagents', enabled: true, loaded: true, path: '~/.oino/extensions/unipi.subagents' },
];

/** The project tree plus a `.oino/sessions` folder, so the import dialog's
 *  "Recent session files" list has something to offer. */
function sessionFs() {
  const tree = genFsTree(CWD);
  const root = tree[CWD];
  if (root?.kind === 'dir') {
    root.children['.oino'] = {
      kind: 'dir',
      children: {
        sessions: {
          kind: 'dir',
          children: {
            '2026-10-31T09-00-00.jsonl': { kind: 'file', text: '{"type":"session"}\n', mtime: 0 },
            '2026-10-30T18-42-00.jsonl': { kind: 'file', text: '{"type":"session"}\n', mtime: 0 },
          },
        },
      },
    };
  }
  return tree;
}

/** The memory browse panel's tree — the shape `extensions/unipi/memory/src/panel.rs` builds. */
export const MEMORY_PANEL_ROOT = {
  t: 'Layout',
  direction: 'Vertical',
  spacing: 1,
  children: [
    // Mirrors `panel::header`: one muted line with the scope and the count.
    { t: 'Paragraph', text: { lines: [{ spans: [{ content: 'Memory · unipi · 12 memories', style: { fg: 'muted' } }] }] } },
    { t: 'Input', value: '', placeholder: 'Search memories…', label: ['Search'], onSubmit: 'search' },
    {
      t: 'List',
      highlightSymbol: '› ',
      onActivate: 'open',
      items: [
        { key: 'decisions_use_oino_host', content: 'Decisions use the oino host', detail: ['decision · ', { content: '2 h ago', style: { fg: 'dim' } }], icon: 'list' },
        { key: 'pattern_graph_retry', content: 'Graph nodes retry from their own session', detail: 'pattern · yesterday', icon: 'list' },
        { key: 'preference_short_reports', content: 'Prefers short reports with evidence', detail: 'preference · 3 d ago', icon: 'list' },
        { key: 'summary_oct_harness', content: 'Harness reachability session', detail: 'summary · 4 d ago', icon: 'list' },
      ],
      block: { title: 'Project memories', borderType: 'Rounded' },
    },
    {
      t: 'Layout',
      direction: 'Horizontal',
      spacing: 2,
      children: [
        { t: 'Button', label: 'Forget…', action: 'forget', variant: 'Danger' },
        { t: 'Button', label: 'Consolidate', action: 'consolidate' },
        { t: 'Button', label: 'All projects', action: 'global' },
        { t: 'Button', label: 'Close', action: 'close' },
      ],
    },
  ],
};

/** The skills editor overlay (`/skills-editor`) — mirror of skill-registry's `editor.rs` spec. */
export const SKILLS_PANEL_ROOT = {
  t: 'Layout',
  direction: 'Vertical',
  spacing: 1,
  children: [
    { t: 'Paragraph', text: { lines: [{ spans: ['Skill settings · ', { content: 'enabled / must show', style: { fg: 'muted' } }] }] } },
    {
      t: 'Table',
      header: { cells: [{ content: 'Skill', style: { addModifier: ['BOLD'] } }, { content: 'Enabled' }, { content: 'Must show' }] },
      rows: [
        { cells: ['editor', { content: 'yes', style: { fg: 'success' } }, { content: 'yes', style: { fg: 'success' } }] },
        { cells: ['memory', { content: 'yes', style: { fg: 'success' } }, { content: 'yes', style: { fg: 'success' } }] },
        { cells: ['kanboard', { content: 'no', style: { fg: 'dim' } }, { content: 'no', style: { fg: 'dim' } }] },
        { cells: ['web-api', { content: 'yes', style: { fg: 'success' } }, { content: 'no', style: { fg: 'dim' } }] },
      ],
    },
    {
      t: 'KeyHints',
      hints: [
        { keys: 'Enter', label: 'save' },
        { keys: 'Esc', label: 'cancel' },
      ],
    },
  ],
};

/** The subagent profile manager overlay (`/agents`) — mirror of `agents_manager`. */
export const AGENTS_PANEL_ROOT = {
  t: 'Layout',
  direction: 'Vertical',
  spacing: 1,
  children: [
    { t: 'Paragraph', text: { lines: [{ spans: ['Agent profiles · ', { content: '4 built-in · 2 custom', style: { fg: 'muted' } }] }] } },
    {
      t: 'List',
      highlightSymbol: '› ',
      items: [
        { key: 'explore', content: 'subagent_explore', detail: 'Read-only exploration · inherit model', icon: 'bot' },
        { key: 'general', content: 'subagent_general', detail: 'General-purpose changes · inherit model', icon: 'bot' },
        { key: 'review', content: 'reviewer', detail: 'Claude Opus 5.5 · high · project', icon: 'bot' },
      ],
      block: { title: 'Profiles', borderType: 'Rounded' },
    },
    {
      t: 'Layout',
      direction: 'Horizontal',
      spacing: 2,
      children: [
        { t: 'Button', label: 'New…', action: 'new' },
        { t: 'Button', label: 'Copy', action: 'copy' },
        { t: 'Button', label: 'Delete', action: 'delete', variant: 'Danger' },
        { t: 'Button', label: 'Close', action: 'close' },
      ],
    },
  ],
};

/** The MCP server panel (`/mcp-settings`) — mirror of mcp's `servers_spec`. */
export const MCP_PANEL_ROOT = {
  t: 'Layout',
  direction: 'Vertical',
  spacing: 1,
  children: [
    { t: 'Paragraph', text: { lines: [{ spans: ['MCP servers · ', { content: '3 configured · 2 connected', style: { fg: 'muted' } }] }] } },
    {
      t: 'List',
      highlightSymbol: '› ',
      items: [
        { key: 'playwright', content: 'playwright', detail: [{ content: 'connected', style: { fg: 'success' } }, ' · 24 tools'], icon: 'globe' },
        { key: 'github', content: 'github', detail: [{ content: 'connected', style: { fg: 'success' } }, ' · 18 tools'], icon: 'globe' },
        { key: 'postgres', content: 'postgres', detail: [{ content: 'failed', style: { fg: 'error' } }, ' · connect ECONNREFUSED'], icon: 'globe' },
      ],
      block: { title: 'Servers', borderType: 'Rounded' },
    },
    { t: 'Button', label: 'Edit JSON', action: 'edit' },
    {
      t: 'Layout',
      direction: 'Horizontal',
      spacing: 2,
      children: [
        { t: 'Button', label: 'Custom server…', action: 'custom' },
        { t: 'Button', label: 'Sync catalog', action: 'sync' },
        { t: 'Button', label: 'Close', action: 'close' },
      ],
    },
  ],
};

function capSession(session = SESSION) {
  const base = genSession(session, { seed: 21, turns: 2, toolsPerTurn: 1, cwd: CWD });
  base.snapshot.commands = CAP_COMMANDS as never;
  return base;
}

function host(extra: Partial<MockHostSpec> = {}): MockHostSpec {
  return {
    id: HOST,
    name: 'workstation',
    transport: 'local',
    conn: { state: 'connected' as const },
    cards: genCards(3, { seed: 4, root: CWD }),
    sessions: { [SESSION]: capSession() },
    fs: sessionFs(),
    settings: {
      pi: { defaultModel: 'glm-4.7', enabledModels: ['anthropic/claude-opus-5-5', 'openai/gpt-5'] },
      namespaces: [MCP_NAMESPACE, MEMORY_NAMESPACE, SKILLS_NAMESPACE, SUBAGENTS_NAMESPACE],
      extensions: EXTENSIONS,
    },
    panels: {
      'mcp-status': { id: 'unipi.mcp/servers', title: 'MCP servers', extensionId: 'unipi.mcp', root: MCP_PANEL_ROOT },
      'mcp-settings': { id: 'unipi.mcp/servers', title: 'MCP servers', extensionId: 'unipi.mcp', root: MCP_PANEL_ROOT },
      memory: { id: 'unipi.memory/browse', title: 'Memory', extensionId: 'unipi.memory', root: MEMORY_PANEL_ROOT },
      'skills-editor': { id: 'unipi.skill-registry/editor', title: 'Skill settings', extensionId: 'unipi.skill-registry', root: SKILLS_PANEL_ROOT },
      agents: { id: 'unipi.subagents/agents', title: 'Agent profiles', extensionId: 'unipi.subagents', root: AGENTS_PANEL_ROOT },
    },
    ...extra,
  };
}

/**
 * Gives the app an open chat without visiting it, then lands on the page:
 * R8's Manage rows need a session in the registry, and unmounting a mounted
 * conversation currently trips a pre-existing Solid `cleanNode` crash (A6/A9/
 * A12), so a capture must not go chat → Settings. Opening the handle is what
 * the app does for a chat shown in another pane.
 */
function openChat(): void {
  void import('../../../lib/session/store').then(({ sessions }) => sessions().ensure(HOST, SESSION));
}

/** Clicks the Manage row of one action once the page is up. */
function clickManage(id: string): void {
  let tries = 0;
  const go = () => {
    tries += 1;
    const el = document.querySelector(`[data-testid="manage-${id}"]`) as HTMLElement | null;
    if (el) {
      el.click();
      return;
    }
    if (tries < 60) setTimeout(go, 80);
  };
  setTimeout(go, 200);
}

export const capMcp: MockScenario = {
  name: 'cap-mcp',
  hosts: [host()],
  setup: () => openChat(),
};

export const capMemory: MockScenario = {
  name: 'cap-memory',
  hosts: [host()],
  setup: () => openChat(),
};

export const capSkills: MockScenario = {
  name: 'cap-skills',
  hosts: [host()],
  setup: () => openChat(),
};

export const capAgents: MockScenario = {
  name: 'cap-agents',
  hosts: [host()],
  setup: () => openChat(),
};

/** `/import`: the Import session… dialog over an open chat (G-15). */
export const capImport: MockScenario = {
  name: 'cap-import',
  hosts: [host()],
  setup: () => {
    void import('../../../lib/session/store').then(({ sessions }) => sessions().ensure(HOST, SESSION));
    location.hash = '#/settings/mcp';
    void import('../../../components/command/ImportSession').then((m) => m.openImportSession());
  },
};

/** The command center with the capability "Manage…" actions (one action, two entry points). */
export const capPalette: MockScenario = {
  name: 'cap-palette',
  hosts: [host()],
  setup: () => {
    void import('../../../lib/session/store').then(({ sessions }) => sessions().ensure(HOST, SESSION));
    location.hash = '#/s/' + HOST + '/' + SESSION;
    void import('../../../lib/shell').then((m) => m.setPaletteOpen(true));
    setTimeout(() => {
      const input = document.querySelector('.palette-input') as HTMLInputElement | null;
      if (!input) return;
      input.value = 'mcp';
      input.dispatchEvent(new Event('input', { bubbles: true }));
    }, 400);
  },
};

/** The chatless R8 state: the Manage rows say what they need. */
export const capMcpNoChat: MockScenario = {
  name: 'cap-mcp-nochat',
  hosts: [host()],
  setup: () => {
    location.hash = '#/settings/mcp';
  },
};

/** Clicking a Manage row opens the extension's panel over the chat (G-8..G-11). */
export const capMemoryPanel: MockScenario = {
  name: 'cap-memory-panel',
  hosts: [host()],
  setup: () => {
    openChat();
    setTimeout(() => clickManage('memory.browse'), 1200);
  },
};

export const capMcpPanel: MockScenario = {
  name: 'cap-mcp-panel',
  hosts: [host()],
  setup: () => {
    openChat();
    setTimeout(() => clickManage('mcp.servers'), 1200);
  },
};

export const capSkillsPanel: MockScenario = {
  name: 'cap-skills-panel',
  hosts: [host()],
  setup: () => {
    openChat();
    setTimeout(() => clickManage('skills.editor'), 1200);
  },
};

export const capAgentsPanel: MockScenario = {
  name: 'cap-agents-panel',
  hosts: [host()],
  setup: () => {
    openChat();
    setTimeout(() => clickManage('agents.profiles'), 1200);
  },
};

const PROVIDERS = [
  { id: 'anthropic', name: 'Anthropic', oauth: true, configured: true, stored: 'oauth' },
  { id: 'openai', name: 'OpenAI', oauth: false, configured: true, stored: 'api_key' },
  { id: 'deepseek', name: 'DeepSeek', oauth: false, configured: true, env: ['DEEPSEEK_API_KEY'] },
  { id: 'google', name: 'Google', oauth: true, configured: false },
  { id: 'openrouter', name: 'OpenRouter', oauth: false, configured: false, env: ['OPENROUTER_API_KEY'] },
];

export const providers: MockScenario = {
  name: 'providers',
  hosts: [
    host({
      settings: {
        pi: { defaultModel: 'glm-4.7' },
        namespaces: [MCP_NAMESPACE, MEMORY_NAMESPACE, SKILLS_NAMESPACE, SUBAGENTS_NAMESPACE],
        extensions: EXTENSIONS,
        providers: PROVIDERS,
      },
    }),
  ],
  setup: () => openChat(),
};

/** The chatless Providers page: Sign in disabled with the reason, keys still work. */
export const providersNoChat: MockScenario = {
  name: 'providers-nochat',
  hosts: [
    host({
      settings: {
        pi: { defaultModel: 'glm-4.7' },
        namespaces: [],
        extensions: EXTENSIONS,
        providers: PROVIDERS,
      },
    }),
  ],
  setup: () => {
    location.hash = '#/settings/providers';
  },
};
