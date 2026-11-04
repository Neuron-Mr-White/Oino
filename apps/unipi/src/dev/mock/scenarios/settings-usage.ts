/**
 * `settings-usage` (owner fix 2026-11-04): Settings › Usage — the host's
 * `usage_stats` answered from a deterministic generator (a free model with no
 * price, two priced ones, six projects), plus its loading, empty and old-host states.
 */
import type { MockHostSpec, MockScenario } from '../host';

const HOST = 'local';
const MODELS: Array<{ name: string; share: number; per: number; priced: boolean }> = [
  { name: 'claude-sonnet-5.5', share: 0.46, per: 38_000, priced: true },
  { name: 'glm-5.3', share: 0.31, per: 52_000, priced: false },
  { name: 'gpt-5-mini', share: 0.15, per: 21_000, priced: true },
  { name: 'glm-5.3-flash', share: 0.08, per: 17_000, priced: false },
];
const PROJECTS = ['/home/you/Projects/llm/oino', '/home/you/Projects/Personal/tally', '/home/you/Projects/Skywalker/openmontage', '/home/you/Projects/web/landing', '/home/you/.config/cliamp', '/home/you'];

/** A busy-then-quiet history ending today (a stable pseudo-random walk). */
export function buildUsage(days: number): unknown {
  const today = new Date();
  const daily = Array.from({ length: days }, (_, i) => {
    const d = new Date(today);
    d.setDate(today.getDate() - (days - 1 - i));
    const wave = Math.abs(Math.sin(i * 1.7) * 0.7 + Math.cos(i * 0.45) * 0.5);
    const weekend = [0, 6].includes(d.getDay());
    const turns = i % 9 === 4 || (weekend && i % 2) ? 0 : Math.round((8 + wave * 70) * (i === days - 3 ? 2.4 : 1));
    const tokens = turns * 31_000;
    const priced = tokens * 0.54;
    const day = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
    return { day, tokens, cost: priced * 0.0000046, unpriced_tokens: tokens - priced, turns };
  });
  const tokens = daily.reduce((n, d) => n + d.tokens, 0);
  const turns = daily.reduce((n, d) => n + d.turns, 0);
  const cost = daily.reduce((n, d) => n + d.cost, 0);
  const by_model = MODELS.map((m) => {
    const t = Math.round(tokens * m.share);
    return { name: m.name, tokens: t, cost: m.priced ? (cost * m.share) / 0.61 : 0, unpriced_tokens: m.priced ? 0 : t, turns: Math.round(t / m.per) };
  });
  const weights = [0.42, 0.23, 0.14, 0.1, 0.07, 0.04];
  const by_project = PROJECTS.map((name, i) => {
    const t = Math.round(tokens * weights[i]!);
    return { name, tokens: t, cost: cost * weights[i]!, unpriced_tokens: Math.round(t * 0.39), sessions: Math.max(1, Math.round(days * weights[i]! * 1.6)) };
  });
  return {
    days,
    totals: {
      input: Math.round(tokens * 0.08), output: Math.round(tokens * 0.03), cache_read: Math.round(tokens * 0.86), cache_write: Math.round(tokens * 0.03),
      tokens, cost, unpriced_tokens: Math.round(tokens * 0.39), turns, sessions: Math.round(days * 1.9), active_days: daily.filter((d) => d.tokens > 0).length,
    },
    daily,
    by_model,
    by_project,
  };
}

const host = (extra: Partial<MockHostSpec> = {}): MockHostSpec => ({
  id: HOST,
  name: 'workstation',
  transport: 'local',
  conn: { state: 'connected' },
  cards: [],
  settings: { pi: {}, namespaces: [], extensions: [], host: {} },
  usage: buildUsage,
  ...extra,
});

const go = () => {
  if (!location.hash.startsWith('#/settings')) location.hash = '#/settings/usage';
};

export const settingsUsage: MockScenario = { name: 'settings-usage', hosts: [host()], setup: go };
export const settingsUsageLoading: MockScenario = { name: 'settings-usage-loading', hosts: [host({ usageDelayMs: 60_000 })], setup: go };
export const settingsUsageEmpty: MockScenario = {
  name: 'settings-usage-empty',
  hosts: [host({ usage: (days) => ({ ...(buildUsage(days) as object), totals: { input: 0, output: 0, cache_read: 0, cache_write: 0, tokens: 0, cost: 0, unpriced_tokens: 0, turns: 0, sessions: 0, active_days: 0 }, daily: [], by_model: [], by_project: [] }) })],
  setup: go,
};
export const settingsUsageOld: MockScenario = { name: 'settings-usage-old', hosts: [host({ usage: null })], setup: go };
