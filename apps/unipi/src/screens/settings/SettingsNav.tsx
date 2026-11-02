/**
 * Settings nav (WP-A24, Z-113): the left column of the two-column layout —
 * 264 px with group titles (ZCode settings-sidebar-layout), collapsing to a
 * 64 px icon rail below the `lg` width so a narrow window never squeezes the
 * settings column. Group titles are structure, not links; entries keep
 * tooltips, `aria-current` and keyboard arrows.
 */
import { For, Show, createEffect, createSignal, onCleanup, onMount } from 'solid-js';
import { SETTINGS_GROUPS, SETTINGS_SECTIONS, type SettingsSection } from './catalog';
import { can, isTauriDesktop } from '../../lib/platform';
import { desktopShell } from '../../lib/layout';

/** Below this window width the nav collapses to the 64 px icon rail (ZCode's
 *  `lg` is 1024 px of content; plus the 264 px sidebar and the gutters that is
 *  labels return at 1100 px; shell.css uses the same breakpoint). */
const NAV_RAIL_WINDOW_MAX = 1100;

/** Sections that have at least one visible entry, in catalog group order
 *  (the phone page renders these with their group titles). */
export function sectionsByGroup(sections: SettingsSection[]): Array<{ id: string; title: string; sections: SettingsSection[] }> {
  return SETTINGS_GROUPS.map((group) => ({ ...group, sections: sections.filter((section) => section.group === group.id) })).filter((group) => group.sections.length > 0);
}

export function SettingsNav(props: { sections: SettingsSection[]; active: () => string; onPick: (id: string) => void }) {
  const [narrow, setNarrow] = createSignal(false);
  onMount(() => {
    const measure = () => setNarrow(window.innerWidth < NAV_RAIL_WINDOW_MAX);
    measure();
    window.addEventListener('resize', measure);
    onCleanup(() => window.removeEventListener('resize', measure));
  });
  const groups = () => sectionsByGroup(props.sections);

  const onNavKey = (e: KeyboardEvent) => {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    e.preventDefault();
    const list: string[] = props.sections.map((section) => section.id);
    const at = list.indexOf(props.active());
    const next = list[Math.max(0, Math.min(list.length - 1, at + (e.key === 'ArrowDown' ? 1 : -1)))]!;
    props.onPick(next);
    (e.currentTarget as HTMLElement).querySelector<HTMLElement>(`[data-nav="${next}"]`)?.focus();
  };

  let navEl: HTMLElement | undefined;
  // D-22: opening a deep section (About, at the foot) must leave its nav item in view.
  createEffect(() => {
    const id = props.active();
    queueMicrotask(() => navEl?.querySelector<HTMLElement>(`[data-nav="${id}"]`)?.scrollIntoView?.({ block: 'nearest' }));
  });

  return (
    <nav ref={(el) => (navEl = el)} class={`settings-nav ${narrow() ? 'settings-nav-rail' : ''}`} aria-label="Settings sections" data-testid="settings-nav" onKeyDown={onNavKey}>
      <For each={groups()}>
        {(group) => (
          <div class="settings-nav-group" role="group" aria-label={group.title}>
            <Show when={!narrow()}>
              <p class="settings-nav-group-title">{group.title}</p>
            </Show>
            <For each={group.sections}>
              {(section) => (
                <button
                  type="button"
                  class="settings-nav-item"
                  data-nav={section.id}
                  data-testid={`settings-nav-${section.id}`}
                  aria-current={props.active() === section.id ? 'page' : undefined}
                  title={section.title}
                  aria-label={section.title}
                  onClick={() => props.onPick(section.id)}
                >
                  <section.icon size={17} />
                  <Show when={!narrow()}>
                    <span class="truncate">{section.title}</span>
                  </Show>
                </button>
              )}
            </For>
          </div>
        )}
      </For>
    </nav>
  );
}

/** The sections this build shows: caps + shell (web/phone hide desktop-only). */
export function visibleSections(): SettingsSection[] {
  // D-45: "desktop-only" means the desktop *app* (tray, local host) — a narrow window keeps General.
  const desktop = desktopShell() || isTauriDesktop();
  return SETTINGS_SECTIONS.filter((section) => (!section.cap || can(section.cap)) && (!section.desktopOnly || desktop) && !(section.phoneOnly && desktop));
}
