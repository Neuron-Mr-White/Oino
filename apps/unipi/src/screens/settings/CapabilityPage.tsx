/**
 * One "Agent capabilities" page (WP-A25): MCP, Memory, Skills and Subagents
 * each show **their extension's settings form** (U11's OSS-2 engine, so the
 * schema is the extension's own) and the extension's **panels** as Manage…
 * rows — the panel itself opens in a chat through the extension's command
 * (U10 renders the UI v2 entry there), never as a second list in Settings
 * (DIRECTION §4.2.2).
 *
 * R8 / P1-07: a panel opens inside a chat. With none open a row opens this
 * PC's latest chat (or starts one) and runs there — no dead button; the form
 * keeps working with no chat at all.
 */
import { For, Show, createMemo, createResource, createSignal, type Component } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { ChevronRight } from 'lucide-solid';
import { NamespaceForm } from '../../ui/settings/form';
import { settingsSchema } from '../../ui/settings/data';
import type { Namespace } from '../../ui/settings/types';
import { LOCAL_HOST } from '../../lib/host/client';
import {
  MISSING_EXTENSION,
  NEEDS_CHAT,
  manageActionsFor,
  manageChat,
  runManageActionAnywhere,
  type ManageAction,
} from '../../lib/settings/manage';
import { useHostsOptional } from '../../hosts/context';
import { Row, Section } from './rows';

export type CapabilityPageProps = {
  page: ManageAction['page'];
  title: string;
  icon: Component<{ size?: number | string }>;
  /** The extension settings namespace rendered first (`undefined` = none on this host). */
  namespace: string;
  /** What the settings form configures (one line under the title, optional). */
  blurb?: string;
  /** Section anchor id (`sec-<page>`); also what Settings search scrolls to. */
  sectionId: string;
};

export function CapabilityPage(props: CapabilityPageProps) {
  const navigate = useNavigate();
  const hosts = useHostsOptional();
  const [namespaces] = createResource(async (): Promise<Namespace[]> => {
    const result = await settingsSchema(LOCAL_HOST);
    return result.ok ? result.value : [];
  });
  const namespace = createMemo(() => (namespaces.latest ?? []).find((ns) => ns.namespace === props.namespace));
  const actions = () => manageActionsFor(props.page);
  // The Manage rows re-render with the session registry: `manageChat()` tracks it.
  const hasChat = () => !!manageChat();
  const [note, setNote] = createSignal<string | undefined>();
  const [busy, setBusy] = createSignal<string | undefined>();
  const localHost = () => hosts?.hosts().find((host) => host.transport === 'local')?.id ?? LOCAL_HOST;
  const run = async (action: ManageAction) => {
    if (busy()) return;
    setBusy(action.id);
    setNote(undefined);
    try {
      const outcome = await runManageActionAnywhere(action, { navigate, hostId: localHost() });
      if (outcome !== 'sent') setNote(outcome === 'no-chat' ? NEEDS_CHAT : MISSING_EXTENSION);
    } finally {
      setBusy(undefined);
    }
  };

  return (
    <Section title={props.title} id={props.sectionId} icon={<props.icon size={14} />}>
      <Show when={props.blurb}>
        <p class="settings-page-blurb">{props.blurb}</p>
      </Show>
      <Show when={namespace()} fallback={<Row label="Settings" description="This host does not report this extension’s settings yet." />}>
        {(ns) => <NamespaceForm namespace={ns()} bare flat testId={`cap-${props.page}-form`} />}
      </Show>
      <div class="settings-in-card-label">Manage</div>
      <For each={actions()}>
        {(action) => (
          <button
            type="button"
            class="settings-namespace-row"
            data-testid={`manage-${action.id}`}
            aria-label={`${action.label} — ${action.description}`}
            aria-describedby={hasChat() ? undefined : `cap-${props.page}-nochat`}
            aria-busy={busy() === action.id ? 'true' : undefined}
            disabled={!!busy() && busy() !== action.id}
            onClick={() => void run(action)}
          >
            <span class="min-w-0">
              <span class="block text-[15px] text-text truncate">{action.label}</span>
              <span class="block text-xs text-text-dim truncate">{action.description}</span>
            </span>
            <ChevronRight size={15} class="text-text-dim shrink-0" />
          </button>
        )}
      </For>
      <Show when={!hasChat()}>
        <p class="settings-field-hint" id={`cap-${props.page}-nochat`} data-testid={`cap-${props.page}-nochat`}>
          Panels open inside a chat: your latest chat on this PC opens (or a new one starts).
        </p>
      </Show>
      <Show when={note()}>
        <p class="settings-field-hint settings-field-hint-bad" role="status" data-testid={`cap-${props.page}-note`}>
          {note()}
        </p>
      </Show>
    </Section>
  );
}
