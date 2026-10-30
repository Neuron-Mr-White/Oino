/**
 * One "Agent capabilities" page (WP-A25): MCP, Memory, Skills and Subagents
 * each show **their extension's settings form** (U11's OSS-2 engine, so the
 * schema is the extension's own) and the extension's **panels** as Manage…
 * rows — the panel itself opens in a chat through the extension's command
 * (U10 renders the UI v2 entry there), never as a second list in Settings
 * (DIRECTION §4.2.2).
 *
 * R8: a panel needs an open chat. Without one the rows say "Open a chat to
 * manage" and offer "New chat here" in one click; the form keeps working with
 * no chat at all.
 */
import { For, Show, createMemo, createResource, createSignal, type Component } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { ChevronRight } from 'lucide-solid';
import { NamespaceForm } from '../../ui/settings/form';
import { settingsSchema } from '../../ui/settings/data';
import type { Namespace } from '../../ui/settings/types';
import { LOCAL_HOST } from '../../lib/host/client';
import { appNavigate } from '../../lib/nav';
import {
  MISSING_EXTENSION,
  NEEDS_CHAT,
  manageActionsFor,
  manageChat,
  runManageAction,
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
  const run = (action: ManageAction) => {
    const outcome = runManageAction(action, { navigate });
    if (outcome === 'sent') {
      setNote(undefined);
      return;
    }
    setNote(outcome === 'no-chat' ? NEEDS_CHAT : MISSING_EXTENSION);
  };
  /** R8: one click from a disabled Manage row to a chat in the current project. */
  const newChatHere = () => {
    const ws = hosts?.hosts().find((host) => host.transport === 'local');
    appNavigate(`/new/${encodeURIComponent(ws?.id ?? LOCAL_HOST)}`);
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
            data-needs-chat={hasChat() ? undefined : ''}
            aria-label={`${action.label} — ${action.description}`}
            aria-describedby={hasChat() ? undefined : `cap-${props.page}-nochat`}
            onClick={() => run(action)}
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
        <div class="settings-manage-empty" id={`cap-${props.page}-nochat`} data-testid={`cap-${props.page}-nochat`}>
          <p class="text-xs text-text-dim">A panel opens inside a chat, so it can talk to the session’s extensions.</p>
          <button type="button" class="btn-secondary settings-btn" data-testid={`cap-${props.page}-newchat`} onClick={newChatHere}>
            New chat here
          </button>
        </div>
      </Show>
      <Show when={note()}>
        <p class="settings-field-hint settings-field-hint-bad" role="status" data-testid={`cap-${props.page}-note`}>
          {note()}
        </p>
      </Show>
    </Section>
  );
}
