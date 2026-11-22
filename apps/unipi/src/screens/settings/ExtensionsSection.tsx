/**
 * Extensions section (WP-A24 list + WP-U11 form, G-19/G-5): the host's
 * extension list with enable toggles (`_oino/set_extension_enabled` over
 * `host_cmd`), crash/disabled notices with their reason, the safe-mode banner
 * naming the suspects, and one row per settings namespace.
 *
 * A namespace opens its own section page (standard Back, Z-113) rendered by
 * the OSS-2 form engine; the per-extension enable toggle (DIRECTION §3.3.4) is
 * that section's first row, so an extension's settings and its on/off live in
 * one place. A disabled extension's namespace is hidden here but its stored
 * values are kept (§7).
 *
 * "Install extension…" (ext-install, `ExtensionInstall.tsx`) installs a package into the
 * host's `<agent dir>/extensions`; user-installed rows offer Remove. Both need an agent
 * host restart (one shared restart affordance), after which a new extension's own
 * settings page opens.
 */
import { For, Show, createResource, createSignal } from 'solid-js';
import type { AppError } from '@unipi/protocol';
import { AlertTriangle, ArrowLeft, ChevronRight, Puzzle, RefreshCw, Trash2 } from 'lucide-solid';
import { hubs } from '../../lib/session/hub';
import { listExtensions, setExtensionEnabled, type ExtensionInfo } from '../../settings/hostCmd';
import { Row, Section } from './rows';
import { SwitchField } from '../../components/Switch';
import { ErrorBanner } from '../../components/ErrorBanner';
import { IconView } from '../../ui/nodes/icons';
import { NamespaceForm } from '../../ui/settings/form';
import { sortNamespaces, type Namespace } from '../../ui/settings/types';
import { settingsSchema } from '../../ui/settings/data';
import { extensionNamespaces } from '../../ui/slots/SettingsSections';
import { NAMESPACE_HOME, openNamespaceHome } from '../../lib/settings/open';
import { recalledScroll, rememberScroll, settingsScroller } from '../../lib/settings/memory';
import { layout } from '../../lib/layout';
import { ExtensionInstallCard, removeInstalled, type InstallPhase } from './ExtensionInstall';
import { settingsHostId } from '../../settings/target';

const LIST_SCROLL_KEY = 'extensions:list';

/** The hub notices that name suspects (the safe-mode banner names them). */
export function safeModeNotices(notices: Array<{ level: string; text: string }>): Array<{ level: string; text: string }> {
  return notices.filter((notice) => notice.level === 'warning' && /safe mode/i.test(notice.text));
}

/** Other host warnings shown beside the extensions: an extension that crashed or was
 *  disabled for this session, with the reason the host gave (`HostNotice`). */
export function crashNotices(notices: Array<{ level: string; text: string }>): Array<{ level: string; text: string }> {
  return notices.filter((notice) => notice.level === 'warning' && !/safe mode/i.test(notice.text));
}

/** D-36: the package's display name; the raw id only when the host sent none. */
export function extensionTitle(extension: ExtensionInfo): string {
  return extension.name?.trim() || extension.id;
}

/** What it does, its version, and — only when something is off — its state. */
export function extensionBlurb(extension: ExtensionInfo): string {
  const state = extension.loaded
    ? undefined
    : extension.enabled
      ? 'Not loaded yet — starts with the next session'
      : 'Off — starts with the next session';
  const parts = [extension.description?.trim(), extension.version ? `v${extension.version}` : undefined, state ?? (extension.description ? undefined : 'Loaded')];
  return parts.filter(Boolean).join(' · ');
}

/** Z-113: the standard secondary-page back button (ghost, 32 px, 16 px arrow). */
export function SettingsBackButton(props: { onBack: () => void; label?: string }) {
  return (
    <button type="button" class="settings-back" onClick={props.onBack} data-testid="settings-back">
      <ArrowLeft size={16} /> {props.label ?? 'Back'}
    </button>
  );
}

export function ExtensionsSection(props: { namespace?: string; onNamespace: (ns: string | undefined) => void }) {
  const hostId = () => settingsHostId();
  const hub = () => hubs().ensure(hostId());
  const [error, setError] = createSignal<AppError | undefined>();
  const [toggling, setToggling] = createSignal<string | undefined>();
  const [list, { refetch }] = createResource(async (): Promise<ExtensionInfo[]> => {
    const result = await listExtensions(hostId());
    if (!result.ok) {
      setError(result.error);
      return [];
    }
    setError(undefined);
    return result.value;
  });
  const [namespaces, { refetch: refetchNamespaces }] = createResource(async (): Promise<Namespace[]> => {
    const result = await settingsSchema(hostId());
    return result.ok ? result.value : [];
  });

  const toggle = async (id: string, enabled: boolean) => {
    setToggling(id);
    const result = await setExtensionEnabled(id, enabled, hostId());
    setToggling(undefined);
    if (!result.ok) {
      setError(result.error);
      return;
    }
    await refetch();
  };

  const [phase, setPhase] = createSignal<InstallPhase>({ kind: 'idle' });
  const [removing, setRemoving] = createSignal<string | undefined>();
  const [confirmRemove, setConfirmRemove] = createSignal<string | undefined>();
  const remove = async (extension: ExtensionInfo) => {
    setConfirmRemove(undefined);
    setRemoving(extension.id);
    const result = await removeInstalled(hostId(), extension.id, extensionTitle(extension));
    setRemoving(undefined);
    if (!result.ok) {
      setError(result.error);
      return;
    }
    setPhase(result.phase);
    await refetch();
  };
  /** After an install is live: refetch and open the extension's own settings page. */
  const onLive = async (id: string): Promise<boolean> => {
    await refetch();
    const spaces = (await refetchNamespaces()) ?? [];
    const ns = sortNamespaces(extensionNamespaces(spaces)).find((space) => space.extensionId === id);
    if (!ns) return false;
    open(ns.namespace);
    return true;
  };

  const owner = (space: Namespace) => list.latest?.find((extension) => extension.id === space.extensionId);
  /** §7: a disabled extension's namespace is hidden, its stored values are kept. */
  const shownNamespaces = () =>
    sortNamespaces(extensionNamespaces(namespaces.latest ?? []).filter((space) => owner(space)?.enabled !== false));

  const namespaceOf = (id: string) => shownNamespaces().find((space) => space.extensionId === id);

  const safeMode = () => safeModeNotices(hub().state.notices);
  const crashes = () => crashNotices(hub().state.notices);
  const openNamespace = () => (props.namespace ? (namespaces.latest ?? []).find((ns) => ns.namespace === props.namespace) : undefined);

  // owner5 #11: the page opens at its top; Back restores the list's scroll position.
  let anchor: HTMLDivElement | undefined;
  const open = (namespace: string) => {
    if (openNamespaceHome(namespace)) return;
    const scroller = settingsScroller(anchor);
    if (scroller) rememberScroll(LIST_SCROLL_KEY, scroller.scrollTop);
    props.onNamespace(namespace);
    requestAnimationFrame(() => {
      const page = document.querySelector<HTMLElement>('[data-testid=namespace-page]');
      const sc = settingsScroller(page ?? anchor);
      if (!sc || !page) return;
      if (layout().desktop()) sc.scrollTop = 0;
      else page.scrollIntoView?.({ block: 'start' });
    });
  };
  const back = () => {
    props.onNamespace(undefined);
    const top = recalledScroll(LIST_SCROLL_KEY);
    if (top === undefined) return;
    let tries = 0;
    const land = () => {
      const sc = settingsScroller(anchor);
      if (sc && sc.scrollHeight - sc.clientHeight >= top - 1) {
        sc.scrollTop = top;
        return;
      }
      if (++tries < 20) requestAnimationFrame(land);
      else if (sc) sc.scrollTop = top;
    };
    requestAnimationFrame(land);
  };

  return (
    <Show
      when={!openNamespace()}
      fallback={
        <NamespacePage
          namespace={openNamespace()!}
          extension={owner(openNamespace()!)}
          toggling={toggling() === owner(openNamespace()!)?.id}
          onToggle={(enabled) => void toggle(openNamespace()!.extensionId, enabled)}
          onBack={back}
        />
      }
    >
      <div ref={anchor} class="contents" />
      <Section title="Extensions" id="sec-extensions" icon={<Puzzle size={14} />}>
        <Show when={safeMode().length > 0}>
          <div class="safe-mode-banner" role="alert" data-testid="safe-mode-banner">
            <AlertTriangle size={16} class="shrink-0" />
            <div class="min-w-0">
              <For each={safeMode()}>
                {(notice) => (
                  <p class="text-[13px] leading-snug">{notice.text}</p>
                )}
              </For>
              <p class="text-xs text-text-dim mt-1">Extensions stay off until you turn them back on.</p>
            </div>
          </div>
        </Show>
        <Show when={crashes().length > 0}>
          <For each={crashes()}>
            {(notice) => (
              <div class="settings-notice-row" role="status" data-testid="extension-crash-notice">
                <AlertTriangle size={15} class="shrink-0" />
                <span class="min-w-0">{notice.text}</span>
              </div>
            )}
          </For>
        </Show>
        <Show when={error()} keyed>
          {(err) => (
            <div class="p-3">
              <ErrorBanner error={err} dismiss={() => setError(undefined)} />
            </div>
          )}
        </Show>
        <Show when={!list.loading && list.latest?.length === 0}>
          <Row label="No extensions" description="Install one below — packages go into the agent's extensions folder on the PC" />
        </Show>
        <For each={list.latest ?? []}>
          {(extension) => (
            <Row
              icon={<Puzzle size={18} />}
              label={extensionTitle(extension)}
              description={extensionBlurb(extension)}
            >
              <Show when={namespaceOf(extension.id)}>
                {(ns) => (
                  <button
                    type="button"
                    class="rounded-pill bg-surface-2 text-text-muted px-3 py-1.5 text-xs mr-2"
                    data-testid={`ext-settings-${extension.id}`}
                    onClick={() => open(ns().namespace)}
                  >
                    Settings
                  </button>
                )}
              </Show>
              <Show when={extension.removable}>
                <Show
                  when={confirmRemove() === extension.id}
                  fallback={
                    <button
                      type="button"
                      class="rounded-pill bg-surface-2 text-text-muted px-2 py-1.5 text-xs mr-2 inline-flex items-center"
                      data-testid={`ext-remove-${extension.id}`}
                      aria-label={`Remove ${extensionTitle(extension)}`}
                      title="Remove (installed by you)"
                      disabled={removing() === extension.id}
                      onClick={() => setConfirmRemove(extension.id)}
                    >
                      <Trash2 size={13} />
                    </button>
                  }
                >
                  <span class="inline-flex items-center gap-1.5 mr-2" data-testid={`ext-remove-confirm-${extension.id}`}>
                    <button
                      type="button"
                      class="rounded-pill bg-surface-2 text-wm-red px-3 py-1.5 text-xs"
                      data-testid={`ext-remove-yes-${extension.id}`}
                      onClick={() => void remove(extension)}
                    >
                      Remove
                    </button>
                    <button type="button" class="rounded-pill bg-surface-2 text-text-muted px-3 py-1.5 text-xs" onClick={() => setConfirmRemove(undefined)}>
                      Keep
                    </button>
                  </span>
                </Show>
              </Show>
              <SwitchField
                label={`Enable ${extension.id}`}
                checked={extension.enabled}
                disabled={toggling() === extension.id}
                onChange={(v) => void toggle(extension.id, v)}
                testId={`ext-${extension.id}`}
              />
            </Row>
          )}
        </For>
        <Show when={shownNamespaces().length > 0}>
          <div class="settings-in-card-label">Settings namespaces</div>
          <For each={shownNamespaces()}>
            {(ns) => (
              <button
                type="button"
                class="settings-namespace-row"
                id={`ns-${ns.namespace}`}
                data-testid={`ns-${ns.namespace}`}
                // P2-03: a namespace with its own place opens there (one place per setting).
                onClick={() => open(ns.namespace)}
                title={NAMESPACE_HOME[ns.namespace] ? `Opens ${NAMESPACE_HOME[ns.namespace]!.label}` : undefined}
              >
                <span class="settings-namespace-icon"><IconView icon={ns.icon ?? 'gear'} size={16} /></span>
                <span class="min-w-0 flex-1">
                  <span class="block text-[15px] text-text truncate">{ns.title}</span>
                  <Show when={ns.description}>
                    <span class="block text-xs text-text-dim truncate">{ns.description}</span>
                  </Show>
                </span>
                <Show when={NAMESPACE_HOME[ns.namespace]} fallback={<span class="settings-ns-badge">{ns.namespace}</span>}>
                  {(home) => <span class="text-xs text-text-dim shrink-0">in {home().label}</span>}
                </Show>
                <ChevronRight size={15} class="text-text-dim shrink-0" />
              </button>
            )}
          </For>
        </Show>
        <ExtensionInstallCard hostId={hostId()} onChanged={async () => { await refetch(); }} onLive={onLive} phase={phase} setPhase={setPhase} />
        <div class="px-4 py-3 flex justify-end">
          <button type="button" class="rounded-pill bg-surface-2 text-text-muted px-3 py-2 text-xs flex items-center gap-1.5" onClick={() => void refetch()}>
            <RefreshCw size={13} /> Refresh
          </button>
        </div>
      </Section>
    </Show>
  );
}

/**
 * One extension's own page (owner5 #11), one level below the Extensions list:
 * "← Extensions" aligned with the title, then the extension's name,
 * description and enable toggle, then its settings form (§7).
 */
function NamespacePage(props: {
  namespace: Namespace;
  extension?: ExtensionInfo;
  toggling: boolean;
  onToggle: (enabled: boolean) => void;
  onBack: () => void;
}) {
  const title = () => props.extension?.name?.trim() || props.namespace.title || props.extension?.id || props.namespace.namespace;
  const blurb = () => props.extension?.description?.trim() || props.namespace.description;
  return (
    <section class="mb-7 settings-subpage" id="sec-extensions" data-testid="namespace-page">
      <div class="settings-subpage-head">
        <SettingsBackButton onBack={props.onBack} label="Extensions" />
      </div>
      <div class="settings-subpage-hero">
        <span class="settings-namespace-icon"><IconView icon={props.namespace.icon ?? 'gear'} size={16} /></span>
        <div class="min-w-0 flex-1">
          <h2 class="settings-subpage-title" data-testid="namespace-page-title">{title()}</h2>
          <Show when={blurb()}>
            <p class="settings-subpage-desc">{blurb()}</p>
          </Show>
          <Show when={props.extension}>
            {(extension) => (
              <p class="settings-subpage-state">
                {extension().enabled ? 'On — loaded with every session that uses it' : 'Off — stored values are kept, nothing loads'}
                <Show when={extension().version}> · v{extension().version}</Show>
              </p>
            )}
          </Show>
        </div>
        <Show when={props.extension}>
          {(extension) => (
            <SwitchField
              label={`Enable ${extensionTitle(extension())}`}
              checked={extension().enabled}
              disabled={props.toggling}
              onChange={props.onToggle}
              testId={`ext-${extension().id}`}
            />
          )}
        </Show>
      </div>
      <NamespaceForm namespace={props.namespace} bare />
    </section>
  );
}
