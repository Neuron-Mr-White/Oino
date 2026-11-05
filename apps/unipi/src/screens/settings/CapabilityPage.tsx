/**
 * One "Agent capabilities" page (WP-A25): MCP, Memory, Skills and Subagents
 * each show **their extension's settings form** (U11's OSS-2 engine, so the
 * schema is the extension's own) and the extension's actions as Manage rows.
 *
 * owner5 #5/#12: a Manage row never leaves Settings by surprise — commands
 * that just do something run in the background with the reply inline on the
 * row; commands that need UI open a Settings sheet that renders their dialog
 * steps; commands about one chat say "Opens in chat ↗" (`ui/settings/command`).
 * owner5 #7: an extension that is off greys its rows with the reason and a
 * "Turn on" button, inline.
 */
import { For, Show, createEffect, createMemo, createResource, createSignal, type Component, type JSX } from 'solid-js';
import { BoardSheet } from '../../components/board/BoardSheet';
import { ArrowUpRight, ChevronRight, CircleAlert } from 'lucide-solid';
import { NamespaceForm } from '../../ui/settings/form';
import { settingsSchema } from '../../ui/settings/data';
import type { Namespace } from '../../ui/settings/types';
import { manageActionsFor, type ManageAction } from '../../lib/settings/manage';
import { commandOwner } from '../../lib/settings/run';
import { CommandResult, CommandSheet, createExtensionStates, createSettingsCommand } from '../../ui/settings/command';
import { setExtensionEnabled } from '../../settings/hostCmd';
import { useHostsOptional } from '../../hosts/context';
import { Row, Section } from './rows';
import { settingsHostId } from '../../settings/target';

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
  /** Extra Manage rows a page renders itself. */
  extraManage?: JSX.Element;
  /** A native block shown between the form and the Manage rows (MCP's server table). */
  beforeManage?: JSX.Element;
  /** Manage rows Settings answers natively (action id → opener), e.g. Agent profiles. */
  openers?: Record<string, () => void>;
  /** Form fields the page already offers natively (one entry per destination): dropped from the form. */
  hideFields?: string[];
};

export function CapabilityPage(props: CapabilityPageProps) {
  const hosts = useHostsOptional();
  const [namespaces] = createResource(async (): Promise<Namespace[]> => {
    const result = await settingsSchema(settingsHostId());
    return result.ok ? result.value : [];
  });
  const namespace = createMemo(() => (namespaces.latest ?? []).find((ns) => ns.namespace === props.namespace));
  const actions = () => manageActionsFor(props.page);
  const localHost = () => hosts?.hosts().find((host) => host.transport === 'local')?.id ?? settingsHostId();
  const extensions = createExtensionStates(localHost);
  // owner5 #10: a row that is "this extension's settings" (Skills hub) opens its form in a sheet.
  const [formSheet, setFormSheet] = createSignal<ManageAction | undefined>();
  /** The extensions behind this page's rows that are off (in row order, once each). */
  const offOwners = () => {
    const out: string[] = [];
    for (const action of actions()) {
      const id = action.extension ?? commandOwner(action.command);
      if (id && extensions.enabled(id) === false && !out.includes(id)) out.push(id);
    }
    return out;
  };
  const turnOn = async (id: string) => {
    const result = await setExtensionEnabled(id, true, localHost());
    if (result.ok) await extensions.refresh();
  };

  return (
    <Section title={props.title} id={props.sectionId} icon={<props.icon size={14} />} intro={props.blurb}>
      <Show when={namespace()} fallback={<Row label="Settings" description="This host does not report this extension’s settings yet." />}>
        {(ns) => (
          <Show when={withoutFields(ns(), props.hideFields).sections.length > 0}>
            <NamespaceForm namespace={withoutFields(ns(), props.hideFields)} bare flat testId={`cap-${props.page}-form`} />
          </Show>
        )}
      </Show>
      {props.beforeManage}
      <Show when={actions().length > 0 || props.extraManage}>
        <div class="settings-in-card-label">Manage</div>
      </Show>
      <For each={offOwners()}>
        {(id) => (
          <div class="settings-off-row" role="status" data-testid={`cap-${props.page}-off`}>
            <CircleAlert size={15} class="shrink-0" />
            <span class="min-w-0 flex-1">{offName(id)} is off — these actions need it.</span>
            <button type="button" class="btn-secondary settings-btn" onClick={() => void turnOn(id)} data-testid={`cap-${props.page}-turn-on`}>
              Turn on
            </button>
          </div>
        )}
      </For>
      <For each={actions()}>
        {(action) => (
          <ManageRow
            action={action}
            hostId={localHost()}
            off={extensions.enabled(action.extension ?? commandOwner(action.command)) === false}
            onTurnedOn={() => void extensions.refresh()}
            onOpen={props.openers?.[action.id] ?? (action.form ? () => setFormSheet(action) : undefined)}
          />
        )}
      </For>
      <Show when={formSheet()}>
        {(action) => (
          <BoardSheet open onClose={() => setFormSheet(undefined)} label={action().label} class="settings-cmd-sheet" testId="settings-form-sheet">
            <Show when={(namespaces.latest ?? []).find((ns) => ns.namespace === action().form)} fallback={<p class="settings-cmd-wait">This host does not report these settings yet.</p>}>
              {(ns) => <NamespaceForm namespace={ns()} bare flat testId={`sheet-${ns().namespace}-form`} />}
            </Show>
          </BoardSheet>
        )}
      </Show>
      {props.extraManage}
    </Section>
  );
}

function offName(id: string): string {
  const tail = id.replace(/^unipi\./, '').replace(/[-_]/g, ' ');
  return tail === 'skill registry' ? 'Skills' : tail === 'mcp' ? 'MCP' : tail[0]!.toUpperCase() + tail.slice(1);
}

/** One Manage row: label + description, the inline result under the text, and its sheet. */
export function ManageRow(props: { action: ManageAction; hostId: string; off?: boolean; onTurnedOn?: () => void; onOpen?: () => void }) {
  const spec = () => ({ label: props.action.label, command: props.action.command, hidden: props.action.hidden, extension: props.action.extension });
  const cmd = createSettingsCommand(spec, () => props.hostId);
  const opensChat = () => !props.onOpen && cmd.mode() === 'chat';
  createEffect(() => {
    if (cmd.state().status === 'turned-on') props.onTurnedOn?.();
  });
  return (
    <div class="settings-manage-row" data-off={props.off ? '' : undefined}>
      <button
        type="button"
        class="settings-namespace-row"
        data-testid={`manage-${props.action.id}`}
        aria-label={`${props.action.label} — ${props.action.description}${opensChat() ? ' (opens in chat)' : ''}`}
        aria-busy={cmd.state().status === 'running' ? 'true' : undefined}
        aria-disabled={props.off ? 'true' : undefined}
        title={props.off ? 'The extension is off' : undefined}
        onClick={() => {
          if (props.off) return;
          if (props.onOpen) props.onOpen();
          else void cmd.run();
        }}
      >
        <span class="min-w-0 flex-1">
          <span class="block text-[15px] text-text truncate">{props.action.label}</span>
          <span class="block text-xs text-text-dim truncate">{props.action.description}</span>
        </span>
        <Show when={opensChat()} fallback={<ChevronRight size={15} class="text-text-dim shrink-0" />}>
          <span class="settings-opens-chat">Opens in chat <ArrowUpRight size={13} /></span>
        </Show>
      </button>
      <CommandResult cmd={cmd} testId={`manage-${props.action.id}-result`} />
      <Show when={cmd.sheet()}>
        <CommandSheet cmd={cmd} spec={spec()} hostId={props.hostId} />
      </Show>
    </div>
  );
}

/** The namespace minus `keys` (and sections left empty). */
function withoutFields(ns: Namespace, keys: string[] | undefined): Namespace {
  if (!keys?.length) return ns;
  return { ...ns, sections: ns.sections.map((sec) => ({ ...sec, fields: sec.fields.filter((f) => !keys.includes(f.key)) })).filter((sec) => sec.fields.length > 0) };
}
