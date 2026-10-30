/**
 * WP-U11 capture/test fixtures: OSS-2 namespaces that exercise every field
 * type of §7 — boolean, enum (segmented + select), multiselect, string,
 * textarea, number (unit + step), secret, order, model, action, page,
 * list, dict, json — plus `visibleIf` / `disabledIf`, scope layering,
 * defaults, `advanced`, `restart` and a validation pattern.
 *
 * The mock host resolves and writes these through the app's own
 * `ui/settings/semantics` port, so a capture shows the same rows a real host
 * produces.
 */
import type { MockNamespace } from '../host';

/** The extension's namespace: one section per field family. */
export const memoryFormNamespace: MockNamespace = {
  namespace: 'memory',
  title: 'Memory',
  description: 'What the memory extension remembers and how it searches',
  extensionId: 'unipi.memory',
  icon: 'board',
  order: 10,
  defaults: { model: 'glm-4.7', topK: 12, autoRecall: true },
  sections: [
    {
      id: 'recall',
      title: 'Recall',
      fields: [
        { key: 'autoRecall', type: 'boolean', label: 'Auto-recall', description: 'Search memories before each turn', default: true },
        { key: 'model', type: 'model', label: 'Memory model', description: 'Writes and consolidates memories', default: 'glm-4.7', placeholder: 'provider/model' },
        {
          key: 'topK',
          type: 'number',
          label: 'Results per search',
          description: 'How many memories a search returns',
          default: 12,
          min: 1,
          max: 50,
          step: 1,
          unit: 'memories',
          integer: true,
        },
        {
          key: 'scope',
          type: 'enum',
          label: 'Recall scope',
          description: 'Which memories a turn sees',
          default: 'workspace',
          options: [
            { value: 'workspace', label: 'Workspace' },
            { value: 'global', label: 'Global' },
            { value: 'all', label: 'All' },
          ],
        },
        {
          key: 'strategy',
          type: 'enum',
          label: 'Ranking strategy',
          description: 'How candidates are ordered before the cut',
          default: 'hybrid',
          options: [
            { value: 'hybrid', label: 'Hybrid', description: 'Embeddings + keyword score' },
            { value: 'embedding', label: 'Embedding', description: 'Vector similarity only' },
            { value: 'keyword', label: 'Keyword', description: 'Full-text match' },
            { value: 'recency', label: 'Recency', description: 'Newest first' },
            { value: 'importance', label: 'Importance', description: 'Pinned and high-signal first' },
          ],
        },
      ],
    },
    {
      id: 'storage',
      title: 'Storage',
      fields: [
        {
          key: 'sources',
          type: 'multiselect',
          label: 'Sources',
          description: 'Where memories are collected from',
          default: ['turns', 'notes'],
          options: [
            { value: 'turns', label: 'Chat turns' },
            { value: 'notes', label: 'Session notes' },
            { value: 'files', label: 'Project files' },
            { value: 'web', label: 'Web research' },
          ],
          min: 1,
        },
        {
          key: 'vault',
          type: 'string',
          label: 'Vault path',
          description: 'Where the memory files live',
          default: '~/.oino/memory',
          placeholder: '~/.oino/memory',
          format: 'path',
        },
        {
          key: 'summary',
          type: 'string',
          label: 'Consolidation prompt',
          description: 'What the consolidator is told to keep',
          multiline: true,
          default: 'Keep decisions, names, paths and open threads. Drop pleasantries.',
        },
        {
          key: 'topicPattern',
          type: 'string',
          label: 'Topic pattern',
          description: 'Memories whose topic matches are grouped',
          placeholder: '^[a-z0-9-]+$',
          pattern: '^[a-z0-9-]+$',
          hint: 'Lowercase letters, digits and dashes only',
        },
        {
          key: 'token',
          type: 'secret',
          label: 'Embedding API key',
          description: 'Stored through the host credential store, never echoed',
        },
        {
          key: 'json',
          type: 'json',
          label: 'Extraction rules',
          description: 'Escape hatch: the raw rule object',
          default: { minWords: 4, drop: ['ok', 'thanks'] },
        },
      ],
    },
    {
      id: 'order',
      title: 'Ranking weights',
      fields: [
        {
          key: 'weights',
          type: 'order',
          label: 'Weight order',
          description: 'Drag to reorder; the first wins ties',
          default: ['recency', 'importance', 'similarity'],
          items: ['recency', 'importance', 'similarity', 'pins'],
        },
        {
          key: 'pins',
          type: 'list',
          label: 'Pinned memories',
          description: 'Always recalled, in order',
          addLabel: 'Pin a memory',
          item: { type: 'string', label: 'Memory id', placeholder: 'mem_…' },
        },
        {
          key: 'aliases',
          type: 'dict',
          label: 'Aliases',
          description: 'Name → canonical memory id',
          keyLabel: 'Alias',
          keyPattern: '^[a-z0-9-]+$',
          value: { type: 'string', label: 'Memory id' },
        },
      ],
    },
    {
      id: 'maintenance',
      title: 'Maintenance',
      advanced: true,
      fields: [
        {
          key: 'consolidate',
          type: 'action',
          label: 'Consolidate now',
          description: 'Runs the extension command in the open chat',
          command: 'memory-consolidate',
          confirm: 'Consolidate every memory now? This can take a minute.',
        },
        {
          key: 'retention',
          type: 'number',
          label: 'Retention',
          description: 'Days before an unused memory is dropped',
          default: 90,
          min: 0,
          step: 30,
          unit: 'days',
          integer: true,
          restart: 'session',
          advanced: true,
        },
        {
          key: 'decay',
          type: 'boolean',
          label: 'Decay unused memories',
          default: true,
          visibleIf: { key: 'retention', truthy: true },
          advanced: true,
        },
        {
          key: 'decayRate',
          type: 'number',
          label: 'Decay rate',
          default: 0.2,
          min: 0,
          max: 5,
          step: 0.1,
          unit: '/week',
          disabledIf: { key: 'decay', eq: false },
          advanced: true,
        },
        {
          key: 'advanced-page',
          type: 'page',
          label: 'Storage internals',
          description: 'SQLite, indexes and vacuum',
          sections: [
            {
              id: 'sqlite',
              title: 'SQLite',
              fields: [
                { key: 'pageSize', type: 'number', label: 'Page size', default: 4096, step: 1024, unit: 'bytes', integer: true },
                { key: 'vacuum', type: 'boolean', label: 'Vacuum on close', default: false },
              ],
            },
          ],
        },
      ],
    },
  ],
  global: {
    topK: 20,
    strategy: 'importance',
    sources: ['turns', 'notes', 'web'],
    token: '••••••',
    json: { minWords: 6, drop: ['ok'] },
    retention: 180,
  },
  project: {
    scope: 'workspace',
    aliases: { inbox: 'mem_01HQINBOX' },
  },
};

/** The core (pi) namespace: Agent › Model & thinking. */
export const modelNamespace: MockNamespace = {
  namespace: 'oino.model',
  title: 'Model & thinking',
  description: 'Startup model, scoped models and reasoning budgets',
  extensionId: 'oino',
  icon: 'bot',
  order: 10,
  defaults: { defaultThinkingLevel: 'medium' },
  sections: [
    {
      id: 'model',
      title: 'Model',
      fields: [
        { key: 'defaultModel', type: 'string', label: 'Default model', description: 'Used when a session starts without a pick', placeholder: 'provider/model' },
        {
          key: 'defaultThinkingLevel',
          type: 'enum',
          label: 'Thinking level',
          default: 'medium',
          options: [
            { value: 'off', label: 'Off' },
            { value: 'low', label: 'Low' },
            { value: 'medium', label: 'Medium' },
            { value: 'high', label: 'High' },
            { value: 'max', label: 'Max' },
          ],
        },
        { key: 'thinkingBudgets', type: 'json', label: 'Thinking budgets', description: 'Per-level token budgets' },
      ],
    },
  ],
  global: { defaultModel: 'glm-4.7', defaultThinkingLevel: 'high' },
  project: { defaultModel: 'glm-4.7-flash' },
};
