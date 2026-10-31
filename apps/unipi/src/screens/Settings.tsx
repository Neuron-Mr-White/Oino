/**
 * The one Settings screen (WP-A24, D-11/Z-113): two columns on the desktop —
 * a 264 px grouped nav (64 px icon rail when narrow) and the active section —
 * with one global search across app sections, host keys, pi agent keys and
 * extension namespaces. Phone: the same sections as one long page with group
 * labels. Secondary pages (an extension's settings namespace) use the
 * standard Back (Z-113).
 */
import { For, Show, createEffect, createSignal, onCleanup, onMount } from 'solid-js';
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
import { Requires } from '../components/Requires';
import { ChevronDown, ChevronUp, Minus, Plus } from 'lucide-solid';
import { SectionNav, Row, Section, Segmented, Stepper } from './settings/rows';
import { SettingsNav, sectionsByGroup, visibleSections } from './settings/SettingsNav';
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
import { SkillsPage } from './settings/SkillsPage';
import { SubagentsPage } from './settings/SubagentsPage';
import { ProvidersPage } from './settings/ProvidersPage';
import { NotificationsSection } from './settings/NotificationsSection';
import { UnicrabSection } from './settings/UnicrabSection';
import { sectionById } from './settings/catalog';
import { consumePendingSettingsNamespace } from '../lib/settings/open';
import { pushBack } from '../lib/back';

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
          <li class="flex items-center gap-2 bg-surface-2 rounded-input px-3 py-2">
            <span class="text-sm flex-1">{TOOLBAR_LABELS[key]}</span>
            <Show when={key !== 'ctrl'}>
              <button type="button" aria-label={`Move ${TOOLBAR_LABELS[key]} up`} class="text-text-muted" disabled={index <= 1} onClick={() => move(index, -1)}>
                <ChevronUp size={16} />
              </button>
              <button type="button" aria-label={`Move ${TOOLBAR_LABELS[key]} down`} class="text-text-muted" disabled={index >= toolbar().length - 1} onClick={() => move(index, 1)}>
                <ChevronDown size={16} />
              </button>
              <button type="button" aria-label={`Remove ${TOOLBAR_LABELS[key]}`} class="text-text-dim" onClick={() => toggle(key)}>
                <Minus size={16} />
              </button>
            </Show>
          </li>
        ))}
      </ul>
      <p class="text-xs text-text-dim mb-2">Available keys</p>
      <div class="flex flex-wrap gap-2">
        {TOOLBAR_KEYS.filter((k) => k !== 'ctrl' && !toolbar().includes(k)).map((key) => (
          <button type="button" class="rounded-pill bg-surface-2 px-3 py-1 text-xs text-text-muted flex items-center gap-1" onClick={() => toggle(key)}>
            <Plus size={12} />
            {TOOLBAR_LABELS[key]}
          </button>
        ))}
      </div>
    </div>
  );
}

export default function Settings() {
  const params = useParams<{ section?: string }>();
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

  const [namespace, setNamespace] = createSignal<string | undefined>();
  createEffect(() => {
    const ns = consumePendingSettingsNamespace();
    if (ns) setNamespace(ns);
  });

  const pickSection = (id: string) => {
    setNamespace(undefined);
    navigate(`/settings/${id}`, { replace: true });
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
    if (hit.namespace) setNamespace(hit.namespace);
    else setNamespace(undefined);
    navigate(`/settings/${hit.section}`, { replace: true });
    let tries = 0;
    const land = () => {
      const anchor = document.getElementById(hit.anchor);
      const el = anchor ?? document.getElementById(`sec-${hit.section}`);
      if (el) {
        el.scrollIntoView?.({ block: 'center' });
        // A row anchor (a form field) flashes itself; a section flashes its card.
        flash(anchor ?? el);
        return;
      }
      tries += 1;
      if (tries < 30) window.setTimeout(land, 60);
    };
    queueMicrotask(land);
  };

  // Phone: Android Back leaves a namespace page before leaving Settings.
  onMount(() => {
    const off = pushBack(() => setNamespace(undefined));
    onCleanup(off);
  });

  // Phone deep links (`#/settings/extensions`, a notification's settings target):
  // the one long page jumps to the section the route names.
  createEffect(() => {
    const id = params.section;
    if (!id || desktop()) return;
    requestAnimationFrame(() => document.getElementById(`sec-${id}`)?.scrollIntoView?.({ block: 'start' }));
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
      case 'host': return <HostSettingsSection />;
      case 'about': return <AboutSection />;
      default: return undefined;
    }
  };

  return (
    <SectionNav.Provider value={{ active: desktop() ? activeTitle : () => undefined }}>
      <div class="settings-page" data-testid="settings-page">
        <div class="settings-top">
          <SettingsSearch onPick={onSearchPick} />
        </div>
        <Show when={desktop()}>
          <SettingsNav sections={sections()} active={active} onPick={pickSection} />
        </Show>
        <div class="settings-body">
          <Show when={desktop()} fallback={<For each={sectionsByGroup(sections())}>{(group) => (
            <div class="settings-phone-group">
              <Show when={group.sections.length > 1 || group.sections[0]?.title !== group.title}>
                <p class="settings-phone-group-title">{group.title}</p>
              </Show>
              <For each={group.sections}>{(section) => body(section.id)}</For>
            </div>
          )}</For>}>
            {body(active() ?? 'about')}
          </Show>
        </div>
      </div>
    </SectionNav.Provider>
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
  return <SharingSettings Section={Section} />;
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
  return (
    <Section title="Board" id="sec-board" icon={<FolderKanban size={14} />}>
      <Row label="Default project">
        <input
          type="text"
          class="rounded-input bg-surface-2 border border-line px-3 py-1.5 text-sm text-text w-36"
          value={settings.board.defaultProject ?? ''}
          placeholder="none"
          onChange={(e) => update('board.defaultProject', e.currentTarget.value.trim() === '' ? null : e.currentTarget.value.trim())}
        />
      </Row>
      <Row label="Show backlog">
        <SwitchField label="Show backlog" checked={settings.board.showBacklog} onChange={(v) => update('board.showBacklog', v)} />
      </Row>
    </Section>
  );
}
