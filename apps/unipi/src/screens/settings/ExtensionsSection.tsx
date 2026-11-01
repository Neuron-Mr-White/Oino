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
 */
import { For, Show, createResource, createSignal } from 'solid-js';
import type { AppError } from '@unipi/protocol';
import { AlertTriangle, ArrowLeft, ChevronRight, Puzzle, RefreshCw } from 'lucide-solid';
import { LOCAL_HOST } from '../../lib/host/client';
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

/** The hub notices that name suspects (the safe-mode banner names them). */
export function safeModeNotices(notices: Array<{ level: string; text: string }>): Array<{ level: string; text: string }> {
  return notices.filter((notice) => notice.level === 'warning' && /safe mode/i.test(notice.text));
}

/** Other host warnings shown beside the extensions: an extension that crashed or was
 *  disabled for this session, with the reason the host gave (`HostNotice`). */
export function crashNotices(notices: Array<{ level: string; text: string }>): Array<{ level: string; text: string }> {
  return notices.filter((notice) => notice.level === 'warning' && !/safe mode/i.test(notice.text));
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
  const hostId = () => LOCAL_HOST;
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
  const [namespaces] = createResource(async (): Promise<Namespace[]> => {
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

  const owner = (space: Namespace) => list.latest?.find((extension) => extension.id === space.extensionId);
  /** §7: a disabled extension's namespace is hidden, its stored values are kept. */
  const shownNamespaces = () =>
    sortNamespaces(extensionNamespaces(namespaces.latest ?? []).filter((space) => owner(space)?.enabled !== false));

  const safeMode = () => safeModeNotices(hub().state.notices);
  const crashes = () => crashNotices(hub().state.notices);
  const openNamespace = () => (props.namespace ? (namespaces.latest ?? []).find((ns) => ns.namespace === props.namespace) : undefined);

  return (
    <Show
      when={!openNamespace()}
      fallback={
        <NamespacePage
          namespace={openNamespace()!}
          extension={owner(openNamespace()!)}
          toggling={toggling() === owner(openNamespace()!)?.id}
          onToggle={(enabled) => void toggle(openNamespace()!.extensionId, enabled)}
          onBack={() => props.onNamespace(undefined)}
        />
      }
    >
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
          <Row label="No extensions" description="Extensions install into ~/.oino/extensions on this PC" />
        </Show>
        <For each={list.latest ?? []}>
          {(extension) => (
            <Row
              label={extension.id}
              description={extension.loaded ? 'Loaded' : extension.enabled ? 'Not loaded yet — starts with the next session' : 'Disabled — starts with the next session'}
            >
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
                onClick={() => {
                  // P2-03: a namespace with its own place opens there (one place per setting).
                  if (!openNamespaceHome(ns.namespace)) props.onNamespace(ns.namespace);
                }}
                title={NAMESPACE_HOME[ns.namespace] ? `Opens ${NAMESPACE_HOME[ns.namespace]!.label}` : undefined}
              >
                <span class="settings-namespace-icon"><IconView icon={ns.icon ?? 'gear'} size={16} /></span>
                <span class="min-w-0">
                  <span class="block text-[15px] text-text truncate">{ns.title}</span>
                  <Show when={ns.description}>
                    <span class="block text-xs text-text-dim truncate">{ns.description}</span>
                  </Show>
                </span>
                <Show when={NAMESPACE_HOME[ns.namespace]} fallback={<span class="text-xs text-text-dim font-mono shrink-0">{ns.namespace}</span>}>
                  {(home) => <span class="text-xs text-text-dim shrink-0">in {home().label}</span>}
                </Show>
                <ChevronRight size={15} class="text-text-dim shrink-0" />
              </button>
            )}
          </For>
        </Show>
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
 * One extension's settings section (§7): the enable toggle first, then the
 * namespace's form (sections, fields, scope toggle, advanced).
 */
function NamespacePage(props: {
  namespace: Namespace;
  extension?: ExtensionInfo;
  toggling: boolean;
  onToggle: (enabled: boolean) => void;
  onBack: () => void;
}) {
  return (
    <section class="mb-7" id="sec-extensions" data-testid="namespace-page">
      <div class="flex items-center gap-3 mb-3">
        <SettingsBackButton onBack={props.onBack} />
        <h2 class="section-label settings-section-label mb-0">{props.namespace.title}</h2>
      </div>
      <NamespaceForm
        namespace={props.namespace}
        bare
        header={
          <Show when={props.extension}>
            {(extension) => (
              <Row
                label={`Enable ${extension().id}`}
                description={
                  extension().enabled
                    ? 'On — loaded with every session that uses it'
                    : 'Off — stored values are kept, nothing loads'
                }
              >
                <SwitchField
                  label={`Enable ${extension().id}`}
                  checked={extension().enabled}
                  disabled={props.toggling}
                  onChange={props.onToggle}
                  testId={`ext-${extension().id}`}
                />
              </Row>
            )}
          </Show>
        }
      />
    </section>
  );
}
