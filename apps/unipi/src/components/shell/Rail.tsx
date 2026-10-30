/**
 * The sidebar rail (Z-1, WP-A3): the few rail pages — Board, Cron, the
 * extensions' `fullscreen` pages (`/x/:entryId`, registered by U10 in
 * ui-protocol §6.4 order) — and Settings at the bottom. Files / Git /
 * Terminal / Browser are **not** here: they are side-pane tabs (Z-5).
 * Board / Cron hide when their reserved gate is false (WP-U12: the owner's
 * enablement, live — no "disabled" placeholders, §8 rule 2).
 */
import { createSignal, For, Show, type Component } from 'solid-js';
import { A, useLocation } from '@solidjs/router';
import { Clock, Columns3, Settings as SettingsIcon } from 'lucide-solid';
import { can, type Capability } from '../../lib/platform';
import type { IconComponent } from '../../lib/panes/kinds';
import { reservedGate } from '../../ui/slots/reserved';

export type RailItem = {
  id: string;
  label: string;
  href: string;
  icon: IconComponent;
  cap?: Capability;
  /** Reserved id this page renders (ui-protocol §8): the owner's gate shows/hides it. */
  reserved?: string;
  order: number;
};

export const BUILTIN_RAIL: RailItem[] = [
  { id: 'board', label: 'Board', href: '/board', icon: Columns3, cap: 'board', reserved: 'unipi.kanboard/board', order: 10 },
  { id: 'cron', label: 'Cron', href: '/cron', icon: Clock, reserved: 'unipi.cron/jobs', order: 20 },
];

const [extPages, setExtPages] = createSignal<RailItem[]>([]);
const [gates, setGates] = createSignal<Record<string, boolean>>({});

/** An extension `fullscreen` page in the rail (U10). Returns the unregister. */
export function registerRailPage(page: { entryId: string; label: string; icon: IconComponent; order?: number }): () => void {
  const item: RailItem = { id: `x:${page.entryId}`, label: page.label, href: `/x/${encodeURIComponent(page.entryId)}`, icon: page.icon, order: 100 + (page.order ?? 0) };
  setExtPages((cur) => [...cur.filter((p) => p.id !== item.id), item].sort((a, b) => a.order - b.order));
  return () => setExtPages((cur) => cur.filter((p) => p !== item));
}

/** Hides / shows a built-in rail page (reserved gate, U12). */
export function setRailGate(id: string, visible: boolean): void {
  setGates((cur) => ({ ...cur, [id]: visible }));
}

export function railItems(): RailItem[] {
  // §8 rule 2: a reserved page appears only while its owner is enabled (U12).
  return [...BUILTIN_RAIL.filter((i) => gates()[i.id] !== false && (!i.reserved || reservedGate(i.reserved))), ...extPages()];
}

/** Extension page component for `/x/:entryId` (U10 registers the renderer). */
const [extPageView, setExtPageView] = createSignal<Component<{ entryId: string }> | undefined>();
export { extPageView, setExtPageView };

function RailLink(props: { item: RailItem }) {
  const location = useLocation();
  const active = () => location.pathname === props.item.href || location.pathname.startsWith(`${props.item.href}/`);
  const available = () => !props.item.cap || can(props.item.cap);
  const Icon = props.item.icon;
  return (
    <A href={props.item.href} class="rail-item" classList={{ unavailable: !available() }} aria-current={active() ? 'page' : undefined} data-testid={`rail-${props.item.id}`}>
      <Icon size={17} strokeWidth={1.7} />
      <span class="rail-label">{props.item.label}</span>
      <Show when={!available()}>
        <span class="rail-badge" title="Needs the UniPi app">app</span>
      </Show>
    </A>
  );
}

export function Rail() {
  const location = useLocation();
  const settingsActive = () => location.pathname === '/settings' || location.pathname.startsWith('/settings/');
  return (
    <nav class="rail" aria-label="Pages">
      <For each={railItems()}>{(item) => <RailLink item={item} />}</For>
      <A href="/settings" class="rail-item rail-settings" aria-current={settingsActive() ? 'page' : undefined} data-testid="rail-settings">
        <SettingsIcon size={17} strokeWidth={1.7} />
        <span class="rail-label">Settings</span>
      </A>
    </nav>
  );
}
