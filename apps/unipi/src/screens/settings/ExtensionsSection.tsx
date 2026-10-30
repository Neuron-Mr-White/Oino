/**
 * Extensions section (WP-A24, G-19): the host's extension list with enable
 * toggles (`_oino/set_extension_enabled` over `host_cmd`), crash/disabled
 * notices with their reason, and the safe-mode banner naming the suspects
 * (the hub's retained `HostNotice`, replayed on `events_open`).
 *
 * Each extension settings namespace opens a secondary page with the standard
 * Back (Z-113): its schema fields and current values (read-only here; the
 * schema-driven form itself is U11).
 */
import { For, Show, createResource, createSignal } from 'solid-js';
import type { AppError } from '@unipi/protocol';
import { AlertTriangle, ArrowLeft, Puzzle, RefreshCw } from 'lucide-solid';
import { LOCAL_HOST } from '../../lib/host/client';
import { hubs } from '../../lib/session/hub';
import { extensionSettings, listExtensions, setExtensionEnabled, settingsSchema, type ExtensionInfo, type Namespace } from '../../settings/hostCmd';
import { Row, Section } from './rows';
import { SwitchField } from '../../components/Switch';
import { ErrorBanner } from '../../components/ErrorBanner';

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

  const safeMode = () => safeModeNotices(hub().state.notices);
  const crashes = () => crashNotices(hub().state.notices);
  const openNamespace = () => (props.namespace ? namespaces.latest?.find((ns) => ns.namespace === props.namespace) : undefined);

  return (
    <Show when={!openNamespace()} fallback={<NamespacePage namespace={openNamespace()!} onBack={() => props.onNamespace(undefined)} />}>
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
        <Show when={(namespaces.latest ?? []).length > 0}>
          <div class="settings-in-card-label">Settings namespaces</div>
          <For each={namespaces.latest ?? []}>
            {(ns) => (
              <button type="button" class="settings-namespace-row" id={`ns-${ns.namespace}`} data-testid={`ns-${ns.namespace}`} onClick={() => props.onNamespace(ns.namespace)}>
                <span class="min-w-0">
                  <span class="block text-[15px] text-text truncate">{ns.title ?? ns.namespace}</span>
                  <Show when={ns.description}>
                    <span class="block text-xs text-text-dim truncate">{ns.description}</span>
                  </Show>
                </span>
                <span class="text-xs text-text-dim font-mono shrink-0">{ns.namespace}</span>
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

/** One extension's settings namespace: schema fields + current values (U11 adds the form). */
function NamespacePage(props: { namespace: Namespace; onBack: () => void }) {
  const hostId = () => LOCAL_HOST;
  const [values] = createResource(async () => {
    const result = await extensionSettings(props.namespace.namespace, hostId());
    return result.ok && typeof result.value === 'object' && result.value !== null ? (result.value as Record<string, unknown>) : {};
  });
  return (
    <section class="mb-7" id="sec-extensions" data-testid="namespace-page">
      <div class="flex items-center gap-3 mb-3">
        <SettingsBackButton onBack={props.onBack} />
        <h2 class="section-label settings-section-label mb-0">{props.namespace.title ?? props.namespace.namespace}</h2>
      </div>
      <div class="settings-group rounded-card bg-surface-1">
        <Show when={props.namespace.description}>
          <p class="px-4 pt-4 text-xs text-text-dim">{props.namespace.description}</p>
        </Show>
        <For each={props.namespace.fields ?? []}>
          {(field) => (
            <Row label={field.title ?? field.key} description={field.description ?? field.type}>
              <span class="text-xs text-text-dim font-mono">{String(values.latest?.[field.key] ?? '—')}</span>
            </Row>
          )}
        </For>
        <Show when={(props.namespace.fields ?? []).length === 0}>
          <Row label="No fields" description="This namespace takes free-form values" />
        </Show>
        <p class="px-4 py-3 text-xs text-text-dim">Values resolve defaults → global → project. Editing arrives with the settings form.</p>
      </div>
    </section>
  );
}
