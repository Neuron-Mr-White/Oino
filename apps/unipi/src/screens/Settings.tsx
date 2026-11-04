/**
 * The one Settings screen (WP-A24, D-11/Z-113): two columns on the desktop —
 * a 264 px grouped nav (64 px icon rail when narrow) and the active section —
 * with one global search across app sections, host keys, pi agent keys and
 * extension namespaces. Phone: the same sections as one long page with group
 * labels. Secondary pages (an extension's settings namespace) use the
 * standard Back (Z-113).
 */
import { Show, createEffect, createResource, createSignal, onCleanup, onMount } from 'solid-js';
import { useNavigate, useParams } from '@solidjs/router';
import { FolderKanban, Keyboard as KeyboardIcon, Server, TerminalSquare, Unplug, MonitorSmartphone } from 'lucide-solid';
import { HostsSettings } from '../components/HostsSettings';
import { LanguageServers } from '../components/LanguageServers';
import { SwitchField } from '../components/Switch';
import { SharingSettings } from '../components/SharingSettings';
import { PushAccess } from '../components/PushAccess';
import { desktopShell } from '../lib/layout';
import { can } from '../lib/platform';
import { useSettings } from '../settings/context';
import { TOOLBAR_KEYS, type ToolbarKey } from '../settings/schema';
import { useHosts } from '../hosts/context';
import { LOCAL_HOST } from '../lib/host';
import { api as boardApi } from '../lib/board/api';
import { boardStore } from '../lib/board/store';
import { EnumSelect } from '../ui/settings/fields';
import { Requires } from '../components/Requires';
import { ChevronDown, Folder, ChevronUp, Minus, Plus } from 'lucide-solid';
import { SectionNav, Row, Section, Segmented, Stepper } from './settings/rows';
import { SettingsNav, visibleSections } from './settings/SettingsNav';
import { SettingsSearch, type SearchHit } from './settings/SettingsSearch';
import { AppearanceSection } from './settings/AppearanceSection';
import { ExtensionsSection } from './settings/ExtensionsSection';
import { ShortcutsSettings } from '../components/command/ShortcutsSettings';
import { AboutSection } from './settings/AboutSection';
import { LifetimeSection } from './settings/LifetimeSection';
import { HostSettingsSection } from './settings/HostSettingsSection';
import { AgentSection } from './settings/AgentSection';
import { McpPage } from './settings/McpPage';
import { MemoryPage } from './settings/MemoryPage';
import { UsagePage } from './settings/UsagePage';
import { SkillsPage } from './settings/SkillsPage';
import { SubagentsPage } from './settings/SubagentsPage';
import { ProvidersPage } from './settings/ProvidersPage';
import { NotificationsSection } from './settings/NotificationsSection';
import { UnicrabSection } from './settings/UnicrabSection';
import { sectionById } from './settings/catalog';
import { consumePendingSettingsNamespace } from '../lib/settings/open';
import { pushBack } from '../lib/back';
import { recalledPlace, recalledScroll, rememberPlace, rememberScroll, settingsScroller } from '../lib/settings/memory';
import { PageScaffold } from '../components/phone';
import { PhoneSettingsRoot, pcScoped, phoneSectionTitle } from './settings/PhoneSettings';
import { AgentProfilesPage } from './settings/AgentProfilesPage';
import { PortsSection } from './Preview';
import { projectScope, setProjectScope, settingsHostId } from '../settings/target';
import './settings/phoneSettings.css';

const TOOLBAR_LABELS: Record<ToolbarKey, string> = {
  ctrl: 'Ctrl', shift: 'Shift', opt: 'Opt', history: 'History', esc: 'Esc', tab: 'Tab',
  arrows: 'Arrows', ctrl_c: 'Ctrl+C', paste: 'Paste', shortcuts: 'Shortcuts', composer: 'Composer',
  keyboard: 'Keyboard', enter: '⏎', backspace: '⌫', home: 'Home', end: 'End', pgup: 'PgUp', pgdn: 'PgDn',
};

/** Editable toolbar order. `ctrl` is pinned first and cannot move or be removed. */
function ToolbarEditor() {
  const [settings, update] = useSettings();
  const toolbar = () => settings.input.toolbar;

  function move(index: number, dir: -1 | 1) {
    if (index === 0) return; // ctrl pinned
    const target = index + dir;
    if (target <= 0 || target >= toolbar().length) return;
    const next = [...toolbar()];
    [next[index], next[target]] = [next[target], next[index]];
    update('input.toolbar', next);
  }

  function toggle(key: ToolbarKey) {
    if (key === 'ctrl') return; // always present
    const current = toolbar();
    const next = current.includes(key) ? current.filter((k) => k !== key) : [...current, key];
    update('input.toolbar', next);
  }

  return (
    <div class="px-4 py-3">
      <p class="text-sm text-text mb-3">Toolbar</p>
      <ul class="space-y-2 mb-4">
        {toolbar().map((key, index) => (
          <li class="toolbar-key flex items-center gap-2 bg-surface-2 rounded-input px-3 py-2 min-h-[60px]">
            <span class="text-sm flex-1">{TOOLBAR_LABELS[key]}</span>
            <Show when={key !== 'ctrl'}>
              <button type="button" aria-label={`Move ${TOOLBAR_LABELS[key]} up`} class="toolbar-key-btn text-text-muted" disabled={index <= 1} onClick={() => move(index, -1)}>
                <ChevronUp size={16} />
              </button>
              <button type="button" aria-label={`Move ${TOOLBAR_LABELS[key]} down`} class="toolbar-key-btn text-text-muted" disabled={index >= toolbar().length - 1} onClick={() => move(index, 1)}>
                <ChevronDown size={16} />
              </button>
              <button type="button" aria-label={`Remove ${TOOLBAR_LABELS[key]}`} class="toolbar-key-btn text-text-dim" onClick={() => toggle(key)}>
                <Minus size={16} />
              </button>
            </Show>
          </li>
        ))}
      </ul>
      <p class="text-xs text-text-dim mb-2">Available keys</p>
      <div class="flex flex-wrap gap-2">
        {TOOLBAR_KEYS.filter((k) => k !== 'ctrl' && !toolbar().includes(k)).map((key) => (
          <button type="button" class="toolbar-add-chip rounded-pill bg-surface-2 px-3 py-1 text-xs text-text-muted flex items-center gap-1" onClick={() => toggle(key)}>
            <Plus size={12} />
            {TOOLBAR_LABELS[key]}
          </button>
        ))}
      </div>
    </div>
  );
}

export default function Settings() {
  const params = useParams<{ section?: string; sub?: string }>();
  const navigate = useNavigate();
  const [settings, update] = useSettings();
  const hostsCtx = useHosts();
  const hostsList = () => hostsCtx.hosts().map((h) => ({ id: h.id, name: h.name }));
  const [relayUrl, setRelayUrl] = createSignal(settings.connection.relay.mode === 'custom' ? settings.connection.relay.url : '');

  const desktop = () => desktopShell();
  const sections = () => visibleSections();
  const active = () => {
    const fromRoute = sectionById(params.section);
    return fromRoute && sections().some((s) => s.id === fromRoute.id) ? fromRoute.id : sections()[0]?.id;
  };
  const activeTitle = () => sections().find((s) => s.id === active())?.title;

  const [namespace, setNamespaceRaw] = createSignal<string | undefined>();
  // Phone (§3.9, PH-09): an extension namespace is its own page, `/settings/extensions/<ns>`.
  const setNamespace = (ns: string | undefined) => {
    if (desktop() || params.section !== 'extensions') return setNamespaceRaw(ns);
    navigate(ns ? `/settings/extensions/${encodeURIComponent(ns)}` : '/settings/extensions');
  };
  createEffect(() => {
    if (desktop()) return;
    setNamespaceRaw(params.section === 'extensions' && params.sub ? decodeURIComponent(params.sub) : undefined);
  });
  createEffect(() => {
    const ns = consumePendingSettingsNamespace();
    if (!ns) return;
    if (!desktop() && params.section === 'extensions' && params.sub !== ns) navigate(`/settings/extensions/${encodeURIComponent(ns)}`, { replace: true });
    else setNamespaceRaw(ns);
  });

  // owner5 #16: Settings remembers its section, sub-page and scroll for the
  // app session. `/settings` (the rail, ⌘,) returns there; a deep link
  // (`/settings/<section>`, a search hit, an extension's openSettings) wins.
  let pageEl: HTMLDivElement | undefined;
  const scrollKey = () => (!desktop() ? 'phone' : namespace() ? `ns:${namespace()}` : `section:${active()}`);
  const saveScroll = () => {
    const sc = settingsScroller(pageEl);
    if (sc) rememberScroll(scrollKey(), sc.scrollTop);
  };
  const restoreScroll = (key: string) => {
    const top = recalledScroll(key) ?? 0;
    let tries = 0;
    const land = () => {
      const sc = settingsScroller(pageEl);
      if (!sc) return;
      // Sections fill in after their host reads: wait until the page is tall enough.
      if (sc.scrollHeight - sc.clientHeight >= top - 1 || ++tries >= 40) {
        sc.scrollTop = top;
        return;
      }
      window.setTimeout(land, 50);
    };
    requestAnimationFrame(land);
  };
  const remembered = recalledPlace();
  // The phone restores its place through the tab's own memory (A's tab bar), not here.
  const restoring = desktop() && !params.section && !!remembered.section;
  if (restoring && remembered.namespace) setNamespaceRaw(remembered.namespace);
  onMount(() => {
    if (!restoring) return;
    if (desktop() && sections().some((s) => s.id === remembered.section)) navigate(`/settings/${remembered.section}`, { replace: true });
    restoreScroll(scrollKey());
  });
  createEffect(() => rememberPlace({ section: active(), namespace: namespace() }));
  // Track the scroll as it happens (at unmount the page may already be detached).
  // A section switch resets the scroll before the page settles: ignore scroll
  // events for a moment after the key changes, so the old offset is never written under the new key.
  let settledAt = 0;
  createEffect(() => {
    scrollKey();
    settledAt = performance.now() + 250;
  });
  onMount(() => {
    const onScroll = (event: Event) => {
      const sc = settingsScroller(pageEl);
      if (!sc || (event.target !== sc && !(event.target === document && sc === document.scrollingElement))) return;
      if (performance.now() < settledAt) return;
      rememberScroll(scrollKey(), sc.scrollTop);
    };
    window.addEventListener('scroll', onScroll, { capture: true, passive: true });
    onCleanup(() => window.removeEventListener('scroll', onScroll, { capture: true }));
  });

  const pickSection = (id: string) => {
    if (id === active() && !namespace()) return;
    saveScroll();
    setNamespace(undefined);
    navigate(`/settings/${id}`, { replace: true });
    if (desktop()) restoreScroll(`section:${id}`);
  };

  /** Flash one element once (search feedback, Z-113). */
  const flash = (el: HTMLElement | null) => {
    if (!el) return;
    el.classList.remove('settings-flash-target');
    void el.offsetWidth; // restart the animation
    el.classList.add('settings-flash-target');
    window.setTimeout(() => el.classList.remove('settings-flash-target'), 1400);
  };

  /**
   * A search hit: open its section — the extension namespace page first when
   * the hit belongs to one — then scroll to (and flash) the field row. A form
   * row appears after its host read, so the landing retries for a moment.
   */
  const onSearchPick = (hit: SearchHit) => {
    if (!desktop()) {
      // Phone: a hit opens its section page (pushed), then lands on the row.
      navigate(hit.namespace ? `/settings/${hit.section}/${encodeURIComponent(hit.namespace)}` : `/settings/${hit.section}`);
    } else {
      if (hit.namespace) setNamespace(hit.namespace);
      else setNamespace(undefined);
      navigate(`/settings/${hit.section}`, { replace: true });
    }
    let tries = 0;
    const land = () => {
      const anchor = document.getElementById(hit.anchor);
      const el = anchor ?? document.getElementById(`sec-${hit.section}`);
      if (el) {
        el.scrollIntoView?.({ block: 'center' });
        // A row anchor (a form field) flashes itself (neutral); a section never flashes (owner5 #10).
        if (anchor && !hit.anchor.startsWith('sec-')) flash(anchor);
        return;
      }
      tries += 1;
      if (tries < 30) window.setTimeout(land, 60);
    };
    queueMicrotask(land);
  };

  // Desktop: Escape leaves a namespace page before leaving Settings. The phone's
  // Back walks the route hierarchy instead (`/settings/extensions/<ns>` → its section).
  onMount(() => {
    if (!desktop()) return;
    const off = pushBack(() => setNamespace(undefined));
    onCleanup(off);
  });

  // Phone: "What this PC sends" lands on the PC's notify form inside Notifications.
  createEffect(() => {
    if (desktop() || params.section !== 'notifications' || params.sub !== 'pc') return;
    let tries = 0;
    const land = () => {
      const el = document.getElementById('sec-notify-pc');
      if (el) return el.scrollIntoView?.({ block: 'start' });
      if (++tries < 40) window.setTimeout(land, 60);
    };
    requestAnimationFrame(land);
  });

  const onRelayUrlChange = (v: string) => {
    setRelayUrl(v);
    update('connection.relay', { mode: 'custom', url: v });
  };

  /** One section's body by id (the desktop column and the phone page share it). */
  const body = (id: string) => {
    switch (id) {
      case 'general': return <LifetimeSection />;
      case 'appearance': return <AppearanceSection />;
      case 'input': return <InputSection />;
      case 'shortcuts': return <ShortcutsSection />;
      case 'terminal': return <TerminalSection />;
      case 'editor': return <EditorSection />;
      case 'board': return <BoardSection />;
      case 'extensions': return <ExtensionsSection namespace={namespace()} onNamespace={setNamespace} />;
      case 'agent': return <AgentSection />;
      // WP-A25 (G-8..G-11, G-15): the harness capability pages.
      case 'mcp': return <McpPage />;
      case 'memory': return <MemoryPage />;
      case 'skills': return <SkillsPage />;
      case 'subagents': return <SubagentsPage />;
      case 'providers': return <ProvidersPage />;
      case 'hosts': return <HostsSection hosts={hostsList()} />;
      case 'connection': return <ConnectionSection relayUrl={relayUrl()} onRelayUrlChange={onRelayUrlChange} />;
      case 'sharing': return <SharingSection />;
      case 'notifications': return <NotificationsSection />;
      case 'unicrab': return <UnicrabSection />;
      case 'usage': return <UsagePage />;
      case 'host': return <HostSettingsSection />;
      case 'ports': return <PortsSection />;
      case 'about': return <AboutSection />;
      default: return undefined;
    }
  };

  const hosts = useHosts();
  const pc = () => hosts.hosts().find((h) => h.id === hosts.selectedId()) ?? hosts.hosts()[0];
  /** The phone's section page (pushed, §3.9): the section from the route, its sub-page if any. */
  const phoneSection = () => sectionById(params.section);
  const phoneBody = () => {
    const section = phoneSection();
    if (!section) return undefined;
    if (section.id === 'subagents' && params.sub === 'profiles') return <AgentProfilesPage hostId={settingsHostId()} onBack={() => navigate('/settings/subagents')} />;
    return body(section.id);
  };
  /** [sidebar-ui] The project the forms are scoped to ("Project settings" in the sidebar). */
  const scopeBar = () => (
    <Show when={projectScope()}>
      {(scope) => (
        <div class="settings-project-bar" data-testid="settings-project-bar" role="status">
          <Folder size={13} aria-hidden="true" />
          <span class="settings-project-name" title={scope().cwd}>
            Project settings · <strong>{scope().cwd.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || scope().cwd}</strong>
          </span>
          <button type="button" class="settings-project-back" onClick={() => setProjectScope(undefined)}>
            Back to global
          </button>
        </div>
      )}
    </Show>
  );

  return (
    <Show when={desktop()} fallback={
      <Show
        when={phoneSection()}
        fallback={
          <PageScaffold title="Settings" testId="settings-page">
            <div class="settings-page settings-phone" ref={pageEl}>
              <PhoneSettingsRoot pcName={pc()?.name} pcId={pc()?.id} search={() => <SettingsSearch onPick={onSearchPick} />} />
            </div>
          </PageScaffold>
        }
        keyed
      >
        {(section) => (
          <PageScaffold
            title={phoneSectionTitle(section, params.sub)}
            subtitle={pcScoped(section) && pc() ? <>{pc()!.name}</> : undefined}
            back={params.sub ? `/settings/${section.id}` : '/settings'}
            testId="settings-page"
          >
            <SectionNav.Provider value={{ active: () => section.title }}>
              <div class="settings-page settings-phone settings-phone-section" data-section-id={section.id} ref={pageEl}>
                {/* PC-scoped pages reload when the selected PC changes (PH-04). */}
                {scopeBar()}
                <Show when={`${settingsHostId()}|${projectScope()?.cwd ?? ''}`} keyed>
                  {(_host: string) => <div class="settings-body">{phoneBody()}</div>}
                </Show>
              </div>
            </SectionNav.Provider>
          </PageScaffold>
        )}
      </Show>
    }>
    <SectionNav.Provider value={{ active: activeTitle }}>
      <div class="settings-page" data-testid="settings-page" ref={pageEl}>
        <div class="settings-top">
          <SettingsSearch onPick={onSearchPick} />
          {scopeBar()}
        </div>
        <SettingsNav sections={sections()} active={active} onPick={pickSection} />
        <div class="settings-body">
          <Show when={`scope:${projectScope()?.cwd ?? ''}`} keyed>{(_scope: string) => <>{body(active() ?? 'about')}</>}</Show>
        </div>
      </div>
    </SectionNav.Provider>
    </Show>
  );
}

/** Z-94: the shortcut registry's rebinding list (extension chords included). */
function ShortcutsSection() {
  return (
    <Section title="Keyboard shortcuts" id="sec-shortcuts" icon={<KeyboardIcon size={14} />}>
      <ShortcutsSettings />
    </Section>
  );
}

function HostsSection(props: { hosts: Array<{ id: string; name: string }> }) {
  return (
    <Section title="Hosts" id="sec-hosts" icon={<Server size={14} />}>
      <Requires cap="hosts" variant="inline">
        <HostsSettings />
      </Requires>
      <Show when={props.hosts.length > 0}>
        <PushAccess hosts={props.hosts} Section={Section} />
      </Show>
    </Section>
  );
}

function ConnectionSection(props: { relayUrl: string; onRelayUrlChange: (v: string) => void }) {
  const [settings, update] = useSettings();
  return (
    <Section title="Connection" id="sec-connection" icon={<Unplug size={14} />}>
      <Row label="Relay">
        <Segmented
          label="Relay"
          value={settings.connection.relay.mode}
          options={['n0', 'custom'] as const}
          labels={{ n0: 'n0 (default)', custom: 'Custom' }}
          onChange={(v) => update('connection.relay', v === 'n0' ? { mode: 'n0' } : { mode: 'custom', url: props.relayUrl || 'https://' })}
        />
      </Row>
      <Show when={settings.connection.relay.mode === 'custom'}>
        <div class="px-4 py-3">
          <label class="text-xs text-text-dim block mb-1" for="relay-url">
            Relay URL
          </label>
          <input
            id="relay-url"
            type="url"
            class="w-full rounded-input bg-surface-2 border border-line px-3 py-2 text-sm text-text"
            value={props.relayUrl}
            onInput={(e) => props.onRelayUrlChange(e.currentTarget.value)}
            placeholder="https://relay.example.com"
          />
        </div>
      </Show>
      <Row label="Path preference">
        <Segmented
          label="Path preference"
          value={settings.connection.pathPreference}
          options={['auto', 'prefer_ssh'] as const}
          labels={{ auto: 'Auto', prefer_ssh: 'Prefer SSH' }}
          onChange={(v) => update('connection.pathPreference', v)}
        />
      </Row>
    </Section>
  );
}

function SharingSection() {
  // Its cards ("Sharing with phones", "This computer") belong to the Sharing nav section.
  return <SharingSettings Section={(p) => <Section {...p} navTitle="Sharing" />} />;
}

function TerminalSection() {
  const [settings, update] = useSettings();
  return (
    <Section title="Terminal" id="sec-terminal" icon={<TerminalSquare size={14} />}>
      <Row label="Font size">
        <Stepper label="Font size" value={settings.terminal.fontSize} min={8} max={32} step={1} onChange={(v) => update('terminal.fontSize', v)} />
      </Row>
      <Row label="Line height">
        <Stepper label="Line height" value={settings.terminal.lineHeight} min={1} max={2} step={0.1} format={(v) => v.toFixed(1)} onChange={(v) => update('terminal.lineHeight', v)} />
      </Row>
      <Row label="Cursor style">
        <Segmented label="Cursor style" value={settings.terminal.cursorStyle} options={['block', 'underline', 'bar'] as const} onChange={(v) => update('terminal.cursorStyle', v)} />
      </Row>
      <Row label="Cursor blink">
        <SwitchField label="Cursor blink" checked={settings.terminal.cursorBlink} onChange={(v) => update('terminal.cursorBlink', v)} testId="terminal-cursor-blink" />
      </Row>
      <Row label="Scrollback">
        <Stepper label="Scrollback" value={settings.terminal.scrollback} min={0} max={100000} step={1000} onChange={(v) => update('terminal.scrollback', v)} />
      </Row>
      <Row label="Phone-size policy" description="Fit phone reflows the session; Keep PC size leaves it alone">
        <Segmented label="Phone-size policy" value={settings.terminal.sizePolicy} options={['fit_phone', 'keep_pc'] as const} labels={{ fit_phone: 'Fit phone', keep_pc: 'Keep PC size' }} onChange={(v) => update('terminal.sizePolicy', v)} />
      </Row>
      <Row label="Keep screen on">
        <SwitchField label="Keep screen on" checked={settings.terminal.keepScreenOn} onChange={(v) => update('terminal.keepScreenOn', v)} />
      </Row>
      <Row label="Reopen last session">
        <SwitchField label="Reopen last session" checked={settings.terminal.reopenLast} onChange={(v) => update('terminal.reopenLast', v)} />
      </Row>
    </Section>
  );
}

function InputSection() {
  const [settings, update] = useSettings();
  return (
    <Section title="Input" id="sec-input" icon={<KeyboardIcon size={14} />}>
      <Show when={can('haptics')}>
        <Row label="Haptics">
          <SwitchField label="Haptics" checked={settings.input.haptics} onChange={(v) => update('input.haptics', v)} />
        </Row>
      </Show>
      <Row label="Composer autocorrect">
        <SwitchField label="Composer autocorrect" checked={settings.input.composerAutocorrect} onChange={(v) => update('input.composerAutocorrect', v)} />
      </Row>
      <Row label="Use icons for modifier keys" description="^ ⇧ ↺ ⇥ ⌥ instead of Ctrl Shift Esc Tab Opt">
        <SwitchField label="Use icons for modifier keys" checked={settings.input.modifierIcons} onChange={(v) => update('input.modifierIcons', v)} />
      </Row>
      <ToolbarEditor />
    </Section>
  );
}

function EditorSection() {
  const [settings, update] = useSettings();
  return (
    <Section title="Editor" id="sec-editor" icon={<MonitorSmartphone size={14} />}>
      <Row label="Vim mode" description="vim keybindings in the file editor, with a NORMAL/INSERT/VISUAL indicator and :w to save">
        <SwitchField label="Vim mode" checked={settings.editor.vim} onChange={(v) => update('editor.vim', v)} />
      </Row>
      <LanguageServers />
    </Section>
  );
}

function BoardSection() {
  const [settings, update] = useSettings();
  const hosts = useHosts();
  // The projects the Board screen lists (same board CLI), from the PC the Board last used.
  const hostId = () => boardStore().target()?.hostId ?? hosts.hosts()[0]?.id ?? LOCAL_HOST;
  const [projects] = createResource(hostId, (id) => boardApi.projects(id).catch(() => []));
  const NONE = '';
  const options = () => {
    const list = (projects.latest ?? []).filter((p) => !p.archived).map((p) => ({ value: p.slug, label: p.name || p.slug }));
    const current = settings.board.defaultProject;
    // A stored project the list doesn't know (another PC, renamed) stays visible, never silently dropped.
    const known = current ? (projects.latest ?? []).find((p) => p.slug.toLowerCase() === current.toLowerCase() || p.name.toLowerCase() === current.toLowerCase()) : undefined;
    return [{ value: NONE, label: 'None' }, ...list, ...(current && !known && !list.some((o) => o.value === current) ? [{ value: current, label: current }] : [])];
  };
  const selected = () => {
    const current = settings.board.defaultProject;
    if (!current) return NONE;
    const hit = (projects.latest ?? []).find((p) => p.slug.toLowerCase() === current.toLowerCase() || p.name.toLowerCase() === current.toLowerCase());
    return hit ? hit.slug : current;
  };
  return (
    <Section title="Board" id="sec-board" icon={<FolderKanban size={14} />}>
      <Row label="Default project" description="The project the Board opens first, when you haven’t opened one before.">
        <EnumSelect
          field={{ key: 'board.defaultProject', label: 'Default project', options: options() }}
          value={selected()}
          disabled={false}
          onChange={(next) => update('board.defaultProject', next === NONE ? null : next)}
        />
      </Row>
      <Row label="Always show backlog">
        <SwitchField label="Always show backlog" checked={settings.board.showBacklog} onChange={(v) => update('board.showBacklog', v)} />
      </Row>
    </Section>
  );
}
