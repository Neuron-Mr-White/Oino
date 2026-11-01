/**
 * Settings search (WP-A24 + WP-U11, Z-113): one search across all sections and
 * every OSS-2 namespace — app rows (the catalog), host settings keys, the core
 * `oino.*` namespaces (pi settings) and each extension namespace with its
 * fields (label, description and key, §7 "Search covers labels, descriptions
 * and keys of all namespaces"). Selecting a result opens its section — a
 * namespace page for extensions — and flashes the field row.
 */
import { For, Show, createEffect, createMemo, createResource, createSignal } from 'solid-js';
import { Search } from 'lucide-solid';
import { HOST_SETTINGS_KEYS } from '@unipi/protocol';
import { visibleSections } from './SettingsNav';
import type { SettingsSectionId } from './catalog';
import { piSettings } from '../../settings/hostCmd';
import { settingsSchema } from '../../ui/settings/data';
import { can } from '../../lib/platform';
import type { Namespace } from '../../ui/settings/types';
import { allFields, fieldAnchor, isCoreNamespace } from '../../ui/settings/types';

export type SearchHit = {
  section: SettingsSectionId;
  label: string;
  detail: string;
  kind: 'app' | 'host' | 'pi' | 'agent' | 'extension';
  /** `sec-<id>` / `key-<dotted key>` / `field-<ns>-<key>` anchor. */
  anchor: string;
  /** The extension namespace a hit belongs to (its page opens first). */
  namespace?: string;
};

const HOST_KEY_LABELS: Record<string, string> = {
  relay: 'Host relay', relay_only: 'Relay only',
  'forwarding.enabled': 'Port forwarding', 'forwarding.ports': 'Forwarded ports', 'forwarding.devices': 'Forwarding devices',
  'push.enabled': 'Push to phones', 'push.devices': 'Push devices',
  'fs.allow': 'File access allow list', 'fs.deny': 'File access deny list',
  'term.max': 'Max terminals', 'update.mode': 'Host updates',
};

/** Namespace hits (§7): the namespace itself plus one hit per field. */
function namespaceHits(namespaces: readonly Namespace[], out: SearchHit[]): void {
  for (const ns of namespaces) {
    const core = isCoreNamespace(ns);
    const section: SearchHit['section'] = core ? 'agent' : 'extensions';
    const kind: SearchHit['kind'] = core ? 'agent' : 'extension';
    out.push({
      section,
      label: ns.title,
      detail: `${ns.namespace}${ns.extensionId ? ` · ${ns.extensionId}` : ''} · ${core ? 'agent' : 'extension'} settings`,
      kind,
      anchor: core ? 'sec-agent' : `ns-${ns.namespace}`,
      ...(core ? {} : { namespace: ns.namespace }),
    });
    for (const field of allFields(ns.sections)) {
      out.push({
        section,
        label: field.label,
        detail: `${ns.namespace}.${field.key}${field.type ? ` · ${field.type}` : ''} · ${core ? 'agent' : 'extension'} setting`,
        kind,
        anchor: fieldAnchor(ns.namespace, field.key),
        ...(core ? {} : { namespace: ns.namespace }),
      });
    }
  }
}

/** The dynamic keys (pi settings + every namespace), loaded once. */
async function dynamicHits(): Promise<SearchHit[]> {
  if (!can('hosts')) return [];
  const hits: SearchHit[] = [];
  const [pi, schema] = await Promise.all([piSettings(), settingsSchema()]);
  const namespaces = schema.ok ? schema.value : [];
  const core = namespaces.some(isCoreNamespace);
  // A v1 host without core namespaces still indexes the raw pi keys.
  if (pi.ok && !core) flatten(pi.value, '', 0, hits);
  namespaceHits(namespaces, hits);
  return hits;
}

function flatten(value: Record<string, unknown>, prefix: string, depth: number, out: SearchHit[]): void {
  if (depth > 1) return;
  for (const [key, child] of Object.entries(value)) {
    const dotted = prefix ? `${prefix}.${key}` : key;
    if (dotted === 'extensionSettings') continue; // the namespaces cover these
    out.push({ section: 'agent', label: dotted, detail: `Agent setting · ${describe(child)}`, kind: 'pi', anchor: `pi-${dotted}` });
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
