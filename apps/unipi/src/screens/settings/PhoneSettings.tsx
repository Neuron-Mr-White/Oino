/**
 * Settings on the phone (phone spec §3.9, PH-09/PH-20/PH-35): the root is a
 * grouped list of sub-pages — This phone / <selected PC> / PCs & connection /
 * Pages / About — and every section opens as its own pushed page
 * (`/settings/:section`, sub-pages `/settings/:section/:sub`). Desktop-only
 * sections (keyboard shortcuts, lifetime, host config, editor) are not listed
 * on a phone; a narrow desktop window keeps them in a "This computer" group.
 */
import { For, Show, type Component } from 'solid-js';
import { Download, Puzzle, Send, Server, Users } from 'lucide-solid';
import { ListGroup, ListRow, RowIcon } from '../../components/phone';
import { SETTINGS_SECTIONS, type PhoneGroupId, type SettingsSection, type SettingsSectionId } from './catalog';
import { can, isTauriDesktop } from '../../lib/platform';
import { useHostsOptional } from '../../hosts/context';
import { hostConnState } from '../../components/ConnectionState';
import { updateState } from '../../lib/update';
import { railItems } from '../../components/shell/Rail';

type Icon = Component<{ size?: number | string }>;

/** One row of the phone Settings root. */
export type PhoneEntry = { id: string; label: string; href: string; icon: Icon; section: SettingsSectionId };

/** The phone root's order per group (§3.9). `profiles` and `pcsends` are links into a section. */
export const PHONE_ORDER: Record<PhoneGroupId, string[]> = {
  phone: ['appearance', 'input', 'terminal', 'notifications', 'unicrab'],
  pc: ['providers', 'agent', 'profiles', 'skills', 'subagents', 'extensions', 'mcp', 'memory', 'board', 'ports'],
  pcs: ['hosts', 'connection', 'sharing', 'pcsends'],
  about: ['about'],
};

/** Link rows that open a sub-page of a section rather than the section itself. */
const LINKS: Record<string, { section: SettingsSectionId; label: string; href: string; icon: Icon }> = {
  profiles: { section: 'subagents', label: 'Agent profiles', href: '/settings/subagents/profiles', icon: Users },
  pcsends: { section: 'notifications', label: 'What this PC sends', href: '/settings/notifications/pc', icon: Send },
};

/**
 * Pure (tested): the phone root's groups from the sections this build may show.
 * `sections` is already filtered by capability; `opts.desktopApp` keeps the
 * desktop-only sections in an extra group (narrow desktop window).
 */
export function phoneSettingsGroups(
  sections: SettingsSection[],
  opts: { desktopApp: boolean; sharing: boolean; hosts: boolean },
): Array<{ id: PhoneGroupId | 'desktop'; entries: PhoneEntry[] }> {
  const byId = new Map(sections.map((s) => [s.id as string, s]));
  const groups: Array<{ id: PhoneGroupId | 'desktop'; entries: PhoneEntry[] }> = [];
  for (const group of Object.keys(PHONE_ORDER) as PhoneGroupId[]) {
    const entries: PhoneEntry[] = [];
    for (const id of PHONE_ORDER[group]) {
      const link = LINKS[id];
      if (link) {
        if (!byId.has(link.section)) continue;
        if (id === 'pcsends' && !opts.hosts) continue;
        entries.push({ id, label: link.label, href: link.href, icon: link.icon, section: link.section });
        continue;
      }
      const section = byId.get(id);
      if (!section || section.phone === false) continue;
      // Sharing lists what this computer shares; a phone has nothing to share.
      if (id === 'sharing' && !opts.sharing) continue;
      entries.push({ id, label: (section.phone && section.phone.label) || section.title, href: `/settings/${id}`, icon: section.icon, section: section.id });
    }
    if (entries.length) groups.push({ id: group, entries });
  }
  if (opts.desktopApp) {
    const desktop = sections
      .filter((s) => s.phone === false)
      .map((s) => ({ id: s.id, label: s.title, href: `/settings/${s.id}`, icon: s.icon, section: s.id }));
    if (desktop.length) groups.splice(groups.length - 1, 0, { id: 'desktop', entries: desktop });
  }
  return groups;
}

/** The sections a phone root may list (capability + runtime gates; desktop-only ones only in the desktop app). */
export function phoneSections(): SettingsSection[] {
  const desktopApp = isTauriDesktop();
  return SETTINGS_SECTIONS.filter(
    (s) => (!s.cap || can(s.cap)) && (!s.show || s.show()) && (!s.desktopOnly || desktopApp) && !(s.phoneOnly && desktopApp),
  );
}

/** The phone Settings root body (inside the tab page's PageScaffold). */
export function PhoneSettingsRoot(props: { pcName?: string; pcId?: string; search: Component }) {
  const hosts = useHostsOptional();
  const groups = () => phoneSettingsGroups(phoneSections(), { desktopApp: isTauriDesktop(), sharing: can('localSharing'), hosts: can('hosts') });
  const pcOffline = () => !!props.pcId && hostConnState(props.pcId)?.state === 'offline';
  const pcStatus = () => (!props.pcId ? 'No PC paired' : hostConnState(props.pcId)?.state === 'connected' ? 'Connected' : pcOffline() ? 'Offline' : 'Connecting…');
  const update = () => {
    const s = updateState();
    return s.kind === 'available' || s.kind === 'downloading' || s.kind === 'ready' ? s.release : undefined;
  };
  const pages = () => railItems().filter((item) => item.id.startsWith('x:'));
  const groupTitle = (id: PhoneGroupId | 'desktop') =>
    id === 'phone' ? 'This phone' : id === 'pc' ? props.pcName ?? 'Your PC' : id === 'pcs' ? 'PCs & connection' : id === 'desktop' ? 'This computer' : 'About';
  return (
    <div class="settings-phone-root" data-testid="settings-phone-root">
      <props.search />
      <Show when={update()}>
        {(release) => (
          <ListGroup inset>
            <ListRow
              title="Update available"
              subtitle={`Unipi ${release().version} is ready`}
              leading={<RowIcon icon={Download} tint="accent" />}
              href="/settings/about"
              chevron
              testId="settings-update-row"
            />
          </ListGroup>
        )}
      </Show>
      <Show when={hosts && hosts.hosts().length > 0 && props.pcId}>
        <ListGroup inset>
          <ListRow
            title={props.pcName ?? 'Your PC'}
            subtitle={pcStatus()}
            leading={<RowIcon icon={Server} tint={pcOffline() ? 'red' : 'green'} />}
            href="/settings/hosts"
            chevron
            testId="settings-selected-pc"
          />
        </ListGroup>
      </Show>
      <For each={groups()}>
        {(group) => (
          <>
            <Show when={group.id === 'about' && pages().length > 0}>
              <ListGroup inset title="Pages">
                <For each={pages()}>
                  {(page) => <ListRow title={page.label} leading={<RowIcon icon={page.icon ?? Puzzle} />} href={page.href} chevron testId={`settings-page-${page.id}`} />}
                </For>
              </ListGroup>
            </Show>
            <ListGroup
              inset
              title={groupTitle(group.id)}
              footer={group.id === 'pc' ? <span>These settings live on {props.pcName ?? 'your PC'}.</span> : undefined}
            >
              <For each={group.entries}>
                {(entry) => (
                  <ListRow
                    title={entry.label}
                    leading={<RowIcon icon={entry.icon} />}
                    caption={group.id === 'pc' && pcOffline() ? 'Offline' : undefined}
                    dim={group.id === 'pc' && pcOffline()}
                    href={entry.href}
                    chevron
                    testId={`settings-row-${entry.id}`}
                  />
                )}
              </For>
            </ListGroup>
          </>
        )}
      </For>
    </div>
  );
}

/** The page title of a phone section / sub-page. */
export function phoneSectionTitle(section: SettingsSection | undefined, sub?: string): string {
  if (!section) return 'Settings';
  if (section.id === 'subagents' && sub === 'profiles') return 'Agent profiles';
  if (section.id === 'notifications' && sub === 'pc') return 'What this PC sends';
  return (section.phone && section.phone.label) || section.title;
}

/** True when a section is about the selected PC (its page subtitle names the PC). */
export function pcScoped(section: SettingsSection | undefined): boolean {
  return !!section && !!section.phone && section.phone.group === 'pc';
}
