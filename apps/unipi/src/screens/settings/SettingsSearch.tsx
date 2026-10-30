/**
 * Settings search (WP-A24, Z-113): one search across all sections and the
 * extension namespaces — app rows (the catalog), host settings keys, pi agent
 * settings keys and every extension namespace + field. Selecting a result
 * jumps to its section (the desktop two-column view switches column, the
 * phone scrolls to the card) with the row highlighted.
 */
import { For, Show, createEffect, createMemo, createResource, createSignal } from 'solid-js';
import { Search } from 'lucide-solid';
import { HOST_SETTINGS_KEYS } from '@unipi/protocol';
import { visibleSections } from './SettingsNav';
import type { SettingsSectionId } from './catalog';
import { piSettings, settingsSchema } from '../../settings/hostCmd';
import { can } from '../../lib/platform';

export type SearchHit = {
  section: SettingsSectionId;
  label: string;
  detail: string;
  kind: 'app' | 'host' | 'pi' | 'extension';
  /** `sec-<id>` anchor (sections) or `key-<dotted key>` row anchor. */
  anchor: string;
};

const HOST_KEY_LABELS: Record<string, string> = {
  relay: 'Host relay', relay_only: 'Relay only',
  'forwarding.enabled': 'Port forwarding', 'forwarding.ports': 'Forwarded ports', 'forwarding.devices': 'Forwarding devices',
  'push.enabled': 'Push to phones', 'push.devices': 'Push devices',
  'fs.allow': 'File access allow list', 'fs.deny': 'File access deny list',
  'term.max': 'Max terminals', 'update.mode': 'Host updates',
};

/** The dynamic keys (pi settings + extension namespaces), loaded once. */
async function dynamicHits(): Promise<SearchHit[]> {
  if (!can('hosts')) return [];
  const hits: SearchHit[] = [];
  const [pi, schema] = await Promise.all([piSettings(), settingsSchema()]);
  if (pi.ok) flatten(pi.value, '', 0, hits);
  if (schema.ok) {
    for (const ns of schema.value) {
      hits.push({
        section: 'extensions',
        label: ns.title ?? ns.namespace,
        detail: `${ns.namespace}${ns.extensionId ? ` · ${ns.extensionId}` : ''} · extension settings`,
        kind: 'extension',
        anchor: `ns-${ns.namespace}`,
      });
      for (const field of ns.fields ?? []) {
        hits.push({
          section: 'extensions',
          label: field.title ?? field.key,
          detail: `${ns.namespace}.${field.key}${field.type ? ` · ${field.type}` : ''} · extension setting`,
          kind: 'extension',
          anchor: `ns-${ns.namespace}`,
        });
      }
    }
  }
  return hits;
}

function flatten(value: Record<string, unknown>, prefix: string, depth: number, out: SearchHit[]): void {
  if (depth > 1) return;
  for (const [key, child] of Object.entries(value)) {
    const dotted = prefix ? `${prefix}.${key}` : key;
    if (dotted === 'extensionSettings') continue; // the namespaces cover these
    out.push({ section: 'agent', label: dotted, detail: `pi agent setting · ${describe(child)}`, kind: 'pi', anchor: `pi-${dotted}` });
    if (child && typeof child === 'object' && !Array.isArray(child)) flatten(child as Record<string, unknown>, dotted, depth + 1, out);
  }
}

function describe(value: unknown): string {
  if (value === null) return 'null';
  if (Array.isArray(value)) return `list (${value.length})`;
  if (typeof value === 'object') return 'group';
  return String(value);
}

export function SettingsSearch(props: { onPick: (hit: SearchHit) => void }) {
  const [query, setQuery] = createSignal('');
  const [open, setOpen] = createSignal(false);
  const [cursor, setCursor] = createSignal(0);
  const [dynamic] = createResource(dynamicHits);

  const staticHits = (): SearchHit[] => {
    const hits: SearchHit[] = [];
    for (const section of visibleSections()) {
      // The section itself (typing "appearance" finds the page, not only its rows).
      hits.push({ section: section.id, label: section.title, detail: `${section.group} · settings section`, kind: 'app', anchor: `sec-${section.id}` });
      for (const row of section.rows) {
        hits.push({ section: section.id, label: row.label, detail: row.description ?? `${section.title} · app setting`, kind: 'app', anchor: `key-${row.key}` });
      }
      if (section.id === 'host') {
        for (const key of HOST_SETTINGS_KEYS) {
          if (key === 'forwarding' || key === 'push' || key === 'fs' || key === 'term' || key === 'update' || key === 'browser' || key === 'sharing') continue;
          hits.push({ section: 'host', label: HOST_KEY_LABELS[key] ?? key, detail: `${key} · host config`, kind: 'host', anchor: `host-${key}` });
        }
      }
    }
    return hits;
  };

  const results = createMemo<SearchHit[]>(() => {
    const q = query().trim().toLowerCase();
    if (!q) return [];
    const all = [...staticHits(), ...(dynamic.latest ?? [])];
    const scored = all
      .map((hit) => {
        const label = hit.label.toLowerCase();
        const detail = hit.detail.toLowerCase();
        let score = -1;
        if (label === q) score = 0;
        else if (label.startsWith(q)) score = 1;
        else if (label.includes(q)) score = 2;
        else if (detail.includes(q)) score = 3;
        return { hit, score };
      })
      .filter((entry) => entry.score >= 0)
      .sort((a, b) => a.score - b.score);
    return scored.slice(0, 12).map((entry) => entry.hit);
  });

  const pick = (hit: SearchHit) => {
    setOpen(false);
    setQuery('');
    props.onPick(hit);
  };

  const onInputKey = (e: KeyboardEvent) => {
    const list = results();
    if (!list.length) return;
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setCursor((at) => Math.min(list.length - 1, at + 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setCursor((at) => Math.max(0, at - 1));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      pick(list[Math.min(cursor(), list.length - 1)]!);
    } else if (e.key === 'Escape') {
      setOpen(false);
    }
  };

  createEffect(() => {
    results();
    setCursor(0);
  });

  return (
    <div class="settings-search" data-testid="settings-search">
      <Search size={16} class="shrink-0" />
      <input
        type="search"
        role="searchbox"
        aria-label="Search settings"
        placeholder="Search settings, agent keys and extensions…"
        value={query()}
        onInput={(e) => {
          setQuery(e.currentTarget.value);
          setOpen(true);
        }}
        onKeyDown={onInputKey}
        onFocus={() => setOpen(true)}
        onBlur={() => setTimeout(() => setOpen(false), 120)}
        data-testid="settings-search-input"
      />
      <Show when={open() && results().length > 0}>
        <ul class="settings-search-results" role="listbox" aria-label="Search results" data-testid="settings-search-results">
          <For each={results()}>
            {(hit, index) => (
              <li>
                <button
                  type="button"
                  role="option"
                  aria-selected={index() === cursor()}
                  class={`settings-search-hit ${index() === cursor() ? 'active' : ''}`}
                  data-kind={hit.kind}
                  data-testid={`settings-hit-${hit.kind}`}
                  onMouseEnter={() => setCursor(index())}
                  onClick={() => pick(hit)}
                >
                  <span class="truncate">{hit.label}</span>
                  <span class="settings-search-detail truncate">{hit.detail}</span>
                </button>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </div>
  );
}
