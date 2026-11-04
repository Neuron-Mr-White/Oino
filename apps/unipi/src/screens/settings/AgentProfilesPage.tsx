/**
 * Settings › Subagents › Agent profiles (owner5 #17): one level below the
 * Subagents page — the built-in profiles (read-only) and the custom ones
 * (global and this project's) with a typed editor: name, description, model,
 * thinking level, tools, system prompt. Save writes the subagents
 * extension's own Markdown format through the host's fs ops
 * (`lib/settings/agentProfiles.ts`); nothing goes through a chat.
 *
 * agent-disable: every profile (built-ins too) has an Enabled switch — in its
 * row, before the lock / chevron, and in its detail sheet. A disabled profile
 * stays listed (dimmed, a "Disabled" chip) and is no longer offered to the
 * agent. The list is `subagents.disabled` (global; the project's own list
 * when it has one); the last enabled profile can't be switched off.
 */
import { For, Show, createMemo, createResource, createSignal } from 'solid-js';
import { Bot, ChevronRight, Copy, Lock, Plus, Trash2 } from 'lucide-solid';
import { hubs } from '../../lib/session/hub';
import { refreshThisComputer, thisComputer } from '../../lib/thisComputer';
import { isHomeCwd } from '../../lib/session/selectors';
import {
  KNOWN_TOOLS,
  THINKING_LEVELS,
  clearProjectDisabled,
  defaultProfileId,
  deleteProfile,
  effectiveDisabled,
  loadDisabled,
  loadProfiles,
  offeredIds,
  saveDisabled,
  toggleDisabled,
  DEFAULT_PROFILE,
  type DisabledState,
  profileDirs,
  promptTemplate,
  saveProfile,
  validateName,
  type AgentDraft,
  type AgentProfile,
  type ProfileSource,
} from '../../lib/settings/agentProfiles';
import { BoardSheet } from '../../components/board/BoardSheet';
import { ModelPickerField } from '../../ui/settings/ModelPickerField';
import { EnumSelect } from '../../ui/settings/fields';
import { SettingsBackButton } from './ExtensionsSection';
import { SwitchField } from '../../components/Switch';
import { settingsHostId } from '../../settings/target';
import { desktopShell } from '../../lib/layout';

/** The host's oino dir (its data dir is `<oino dir>/host`). */
async function oinoDir(): Promise<string | undefined> {
  if (thisComputer().kind !== 'ready') await refreshThisComputer().catch(() => undefined);
  const state = thisComputer();
  return state.kind === 'ready' ? state.info.data_dir.replace(/[\\/]+host[\\/]*$/, '') : undefined;
}

/** The project the list edits: the most recent chat's folder on this PC (not the bare home). */
function recentProject(hostId: string): string | undefined {
  const cards = Object.values(hubs().ensure(hostId).state.cards).filter((c) => c.source !== 'cron' && !isHomeCwd(c.cwd));
  cards.sort((a, b) => (b.last_activity ?? b.updated_at ?? 0) - (a.last_activity ?? a.updated_at ?? 0));
  return cards[0]?.cwd ?? undefined;
}

const SOURCE_LABEL: Record<ProfileSource, string> = { builtin: 'Built-in', global: 'Global', project: 'Project' };
const folderName = (p: string) => p.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || p;

function toolsSummary(p: AgentProfile): string {
  if (!p.tools) return 'all tools';
  if (p.tools.length <= 4) return p.tools.join(', ') || 'no tools';
  return `${p.tools.length} tools`;
}

type Editing = { profile?: AgentProfile; draft: AgentDraft; scope: Exclude<ProfileSource, 'builtin'>; readOnly: boolean };

export function AgentProfilesPage(props: { hostId?: string; onBack: () => void }) {
  const hostId = () => props.hostId ?? settingsHostId();
  const project = recentProject(hostId());
  const [dirs] = createResource(async () => profileDirs(await oinoDir(), project));
  const [disabledState, { mutate: setDisabledState }] = createResource(() => hostId(), (h) => loadDisabled(h, project));
  const [toggleError, setToggleError] = createSignal<string | undefined>();
  const [data, { refetch }] = createResource(
    () => dirs(),
    (d) => loadProfiles(hostId(), d),
  );
  const [editing, setEditing] = createSignal<Editing | undefined>();
  const profiles = () => data.latest?.profiles ?? [];
  const group = (source: ProfileSource) => profiles().filter((p) => p.source === source);
  const customIds = () => profiles().filter((p) => p.source !== 'builtin').map((p) => p.id);
  const ids = () => profiles().map((p) => p.id);
  const disabled = () => (disabledState.latest ? effectiveDisabled(disabledState.latest) : []);
  const isOn = (id: string) => offeredIds(ids(), disabled()).includes(id);
  /** The one profile still on: its switch can't go off. */
  const lastOn = (id: string) => isOn(id) && offeredIds(ids(), disabled()).length === 1;
  const defaultId = () => defaultProfileId(ids(), disabled());
  const setEnabled = async (id: string, enabled: boolean) => {
    const state = disabledState.latest;
    if (!state) return;
    const next = toggleDisabled(ids(), effectiveDisabled(state), id, enabled);
    if (!next.ok) {
      setToggleError(next.error);
      return;
    }
    setToggleError(undefined);
    const optimistic: DisabledState = state.project !== undefined && state.cwd ? { ...state, project: next.list } : { ...state, global: next.list };
    setDisabledState(optimistic);
    const saved = await saveDisabled(hostId(), state, next.list);
    if (!saved.ok) {
      setDisabledState(state);
      setToggleError(`Not saved — ${saved.error}`);
    }
  };
  const useGlobalList = async () => {
    const state = disabledState.latest;
    if (!state) return;
    const saved = await clearProjectDisabled(hostId(), state);
    if (saved.ok) setDisabledState({ global: state.global, ...(state.cwd ? { cwd: state.cwd } : {}) });
    else setToggleError(`Not saved — ${saved.error}`);
  };

  const open = (profile: AgentProfile) =>
    setEditing({
      profile,
      draft: { name: profile.id, description: profile.description, model: profile.model, thinking: profile.thinking, tools: profile.tools, maxNesting: profile.maxNesting, prompt: profile.systemPrompt },
      scope: profile.source === 'project' ? 'project' : 'global',
      readOnly: profile.source === 'builtin',
    });
  const create = () =>
    setEditing({ draft: { name: '', description: '', prompt: '' }, scope: 'global', readOnly: false });
  const duplicate = (profile: AgentProfile) => {
    let name = `${profile.id.replace(/^subagent_/, '')}-copy`.toLowerCase().replace(/[^a-z0-9_-]/g, '-');
    for (let n = 2; customIds().includes(name); n += 1) name = `${profile.id}-copy-${n}`;
    setEditing({
      draft: { name, description: profile.description, model: profile.model, thinking: profile.thinking, tools: profile.tools ? [...profile.tools] : undefined, maxNesting: profile.maxNesting, prompt: profile.systemPrompt },
      scope: profile.source === 'project' ? 'project' : 'global',
      readOnly: false,
    });
  };

  return (
    <section class="mb-7 settings-subpage" id="sec-subagents" data-testid="agent-profiles-page">
      <div class="settings-subpage-head">
        <SettingsBackButton onBack={props.onBack} label="Subagents" />
      </div>
      <div class="settings-subpage-hero">
        <span class="settings-namespace-icon"><Bot size={16} /></span>
        <div class="min-w-0 flex-1">
          <h2 class="settings-subpage-title">Agent profiles</h2>
          <p class="settings-subpage-desc">The agents the main chat can delegate to. Switch one off to hide it from the agent; built-ins are otherwise read-only — duplicate one to change it.</p>
        </div>
        <button type="button" class="btn-secondary settings-btn inline-flex items-center gap-1" onClick={create} data-testid="agent-new">
          <Plus size={14} /> New agent
        </button>
      </div>
      <Show when={disabledState.latest?.project !== undefined && disabledState.latest?.cwd}>
        <div class="settings-off-row" role="status" data-testid="agent-project-list">
          <span class="min-w-0">{folderName(disabledState.latest!.cwd!)} has its own list of enabled profiles.</span>
          <button type="button" class="settings-link" onClick={() => void useGlobalList()}>Use the global list</button>
        </div>
      </Show>
      <Show when={profiles().length > 0 && defaultId() && defaultId() !== DEFAULT_PROFILE}>
        <div class="settings-off-row" role="status" data-testid="agent-default-note">
          <span class="min-w-0">{DEFAULT_PROFILE} is disabled: work that names no profile runs <span class="font-mono">{defaultId()}</span>.</span>
        </div>
      </Show>
      <Show when={toggleError()}>
        <p class="settings-field-error px-1 mb-3" role="alert" data-testid="agent-toggle-error">{toggleError()}</p>
      </Show>
      <Show when={(data.latest?.warnings.length ?? 0) > 0}>
        <div class="settings-off-row" role="status">
          <span class="min-w-0">{data.latest!.warnings.join(' · ')}</span>
        </div>
      </Show>
      <For each={['builtin', 'global', 'project'] as const}>
        {(source) => (
          <Show when={source !== 'project' || dirs.latest?.project}>
            <div class="settings-group rounded-card bg-surface-1 mb-4" data-testid={`agent-group-${source}`}>
              <div class="settings-in-card-label settings-in-card-label-first">
                {SOURCE_LABEL[source]}
                <Show when={source === 'project' && dirs.latest?.project}> · {folderName(dirs.latest!.project!.replace(/[\\/]\.oino[\\/]agents$/, ''))}</Show>
              </div>
              <Show when={data.loading && !data.latest}>
                <p class="settings-cmd-wait px-4 py-3">Loading…</p>
              </Show>
              <Show when={!data.loading || data.latest}>
                <Show when={group(source).length > 0} fallback={<p class="settings-cmd-wait px-4 py-3">No {SOURCE_LABEL[source].toLowerCase()} agents yet.</p>}>
                  <For each={group(source)}>
                    {(profile) => (
                      <div class="settings-namespace-row agent-row" data-off={isOn(profile.id) ? undefined : ''} data-testid={`agent-row-${profile.id}`}>
                        <button type="button" class="agent-row-open" onClick={() => open(profile)} data-testid={`agent-${profile.id}`}>
                          <span class="min-w-0 flex-1 agent-row-text">
                            <span class="block text-[15px] text-text truncate font-mono">{profile.id}</span>
                            <span class="block text-xs text-text-dim truncate">{profile.description}</span>
                          </span>
                          <Show when={!isOn(profile.id)}>
                            <span class="agent-off-chip" data-testid={`agent-off-${profile.id}`}>Disabled</span>
                          </Show>
                          <span class="settings-ns-badge">{profile.model ? profile.model.split('/').pop() : 'inherit'} · {toolsSummary(profile)}</span>
                        </button>
                        <span class="agent-row-switch" title={lastOn(profile.id) ? 'At least one profile must stay enabled' : undefined}>
                          <SwitchField
                            label={`${profile.id} enabled`}
                            checked={isOn(profile.id)}
                            disabled={!disabledState.latest || lastOn(profile.id)}
                            onChange={(on) => void setEnabled(profile.id, on)}
                            testId={`agent-switch-${profile.id}`}
                          />
                        </span>
                        <button type="button" class="agent-row-end" onClick={() => open(profile)} aria-label={profile.source === 'builtin' ? `${profile.id} (read-only)` : `Edit ${profile.id}`} tabIndex={-1}>
                          <Show when={profile.source === 'builtin'} fallback={<ChevronRight size={15} class="text-text-dim shrink-0" />}>
                            <Lock size={13} class="text-text-dim shrink-0" aria-label="Read-only" />
                          </Show>
                        </button>
                      </div>
                    )}
                  </For>
                </Show>
              </Show>
            </div>
          </Show>
        )}
      </For>
      <Show when={editing()}>
        {(ed) => (
          <ProfileEditor
            hostId={hostId()}
            editing={ed()}
            dirs={dirs.latest ?? {}}
            existing={customIds().filter((id) => id !== ed().profile?.id)}
            onClose={() => setEditing(undefined)}
            onSaved={() => {
              setEditing(undefined);
              void refetch();
            }}
            onDuplicate={(p) => duplicate(p)}
            enabled={ed().profile ? isOn(ed().profile!.id) : undefined}
            lastOn={ed().profile ? lastOn(ed().profile!.id) : false}
            onEnabled={(on) => ed().profile && void setEnabled(ed().profile!.id, on)}
          />
        )}
      </Show>
    </section>
  );
}

function ProfileEditor(props: {
  hostId: string;
  editing: Editing;
  dirs: { global?: string; project?: string };
  existing: string[];
  onClose: () => void;
  onSaved: () => void;
  onDuplicate: (profile: AgentProfile) => void;
  /** The saved profile's switch (`undefined` for a new agent). */
  enabled?: boolean;
  lastOn: boolean;
  onEnabled: (on: boolean) => void;
}) {
  const ro = () => props.editing.readOnly;
  const isNew = () => !props.editing.profile;
  const [name, setName] = createSignal(props.editing.draft.name);
  const [description, setDescription] = createSignal(props.editing.draft.description);
  const [model, setModel] = createSignal(props.editing.draft.model ?? '');
  const [thinking, setThinking] = createSignal(props.editing.draft.thinking ?? '');
  const [tools, setTools] = createSignal<string[] | undefined>(props.editing.draft.tools);
  const [prompt, setPrompt] = createSignal(props.editing.draft.prompt);
  const [scope, setScope] = createSignal(props.editing.scope);
  const [error, setError] = createSignal<string | undefined>();
  const [busy, setBusy] = createSignal(false);
  const [confirmDelete, setConfirmDelete] = createSignal(false);
  const nameError = () => (ro() ? undefined : name() ? validateName(name(), props.existing) : undefined);
  const toolOptions = createMemo(() => [...new Set([...KNOWN_TOOLS, ...(props.editing.draft.tools ?? [])])]);
  const title = () => (ro() ? props.editing.profile!.id : isNew() ? 'New agent' : `Edit ${props.editing.profile!.id}`);

  const save = async () => {
    if (ro() || busy()) return;
    const problem = validateName(name(), props.existing) ?? (description().trim() ? undefined : 'Add a one-line description.');
    if (problem) {
      setError(problem);
      return;
    }
    const dir = scope() === 'project' ? props.dirs.project : props.dirs.global;
    if (!dir) {
      setError('This PC didn’t report its agent folder — try again in a moment.');
      return;
    }
    setBusy(true);
    setError(undefined);
    const draft: AgentDraft = {
      name: name(),
      description: description().trim(),
      model: model() || undefined,
      thinking: thinking() || undefined,
      tools: tools(),
      maxNesting: props.editing.draft.maxNesting,
      prompt: prompt().trim() || promptTemplate(name(), description().trim()),
    };
    // A scope change moves the file: the old one goes, the new one is written fresh.
    const previous = props.editing.profile && props.editing.profile.source === scope() ? props.editing.profile : undefined;
    const result = await saveProfile(props.hostId, dir, draft, previous);
    if (result.ok && props.editing.profile && !previous) await deleteProfile(props.hostId, props.editing.profile);
    setBusy(false);
    if (result.ok) props.onSaved();
    else setError(result.error);
  };
  const remove = async () => {
    const profile = props.editing.profile;
    if (!profile) return;
    setBusy(true);
    const result = await deleteProfile(props.hostId, profile);
    setBusy(false);
    if (result.ok) props.onSaved();
    else setError(result.error);
  };

  return (
    <BoardSheet
      open
      onClose={props.onClose}
      label={title()}
      class="settings-cmd-sheet agent-editor-sheet"
      testId="agent-editor"
      guarded={!ro()}
      footer={
        <div class="settings-cmd-foot">
          <Show when={props.editing.profile}>
            {(profile) => (
              <button type="button" class="settings-link inline-flex items-center gap-1" onClick={() => props.onDuplicate(profile())} data-testid="agent-duplicate">
                <Copy size={13} /> Duplicate
              </button>
            )}
          </Show>
          <Show when={props.editing.profile && !ro()}>
            <Show
              when={confirmDelete()}
              fallback={
                <button type="button" class="settings-link inline-flex items-center gap-1 text-wm-red" onClick={() => setConfirmDelete(true)} data-testid="agent-delete">
                  <Trash2 size={13} /> Delete…
                </button>
              }
            >
              <span class="text-xs text-text-muted">Delete {props.editing.profile!.id}?</span>
              <button type="button" class="btn-secondary settings-btn text-wm-red" disabled={busy()} onClick={() => void remove()} data-testid="agent-delete-confirm">Delete</button>
              <button type="button" class="settings-link" onClick={() => setConfirmDelete(false)}>Cancel</button>
            </Show>
          </Show>
          <span class="flex-1" />
          <Show when={!ro()} fallback={<button type="button" class="btn-secondary settings-btn" onClick={props.onClose}>Close</button>}>
            <button type="button" class="settings-link" onClick={props.onClose}>Cancel</button>
            <button type="button" class="btn-primary settings-btn" disabled={busy() || !!nameError() || !name()} onClick={() => void save()} data-testid="agent-save">
              {busy() ? 'Saving…' : 'Save'}
            </button>
          </Show>
        </div>
      }
    >
      <div class="agent-editor">
        <Show when={props.enabled !== undefined}>
          <div class="agent-enabled" data-off={props.enabled ? undefined : ''} data-testid="agent-enabled">
            <span class="min-w-0 flex-1">
              <span class="block text-[14px] text-text">Enabled</span>
              <span class="block text-xs text-text-dim">
                {props.lastOn ? 'The only enabled profile — at least one must stay on.' : props.enabled ? 'Offered to the agent.' : 'Hidden from the agent until you turn it back on.'}
              </span>
            </span>
            <SwitchField label="Enabled" checked={props.enabled ?? true} disabled={props.lastOn} onChange={props.onEnabled} testId="agent-enabled-switch" />
          </div>
        </Show>
        <Show when={ro()}>
          <p class="settings-cmd-wait"><Lock size={13} class="inline" /> Built-in profile — read-only. Duplicate it to make your own.</p>
        </Show>
        <label class="agent-field">
          <span class="agent-label">Name</span>
          <input class="settings-input agent-input font-mono" value={name()} disabled={ro()} autofocus={isNew() && desktopShell()} placeholder="reviewer" spellcheck={false} autocapitalize="off" autocorrect="off" onInput={(e) => setName(e.currentTarget.value.trim())} data-testid="agent-name" />
          <Show when={nameError()}>
            <span class="settings-field-error">{nameError()}</span>
          </Show>
        </label>
        <label class="agent-field">
          <span class="agent-label">Description</span>
          <input class="settings-input agent-input" value={description()} disabled={ro()} placeholder="What this agent is for (one line)" onInput={(e) => setDescription(e.currentTarget.value)} data-testid="agent-description" />
        </label>
        <Show when={!ro()}>
          <div class="agent-field">
            <span class="agent-label">Saved in</span>
            <EnumSelect
              field={{ key: 'agent.scope', label: 'Saved in', options: [
                { value: 'global', label: 'Global — every project' },
                ...(props.dirs.project ? [{ value: 'project', label: `This project — ${folderName(props.dirs.project.replace(/\/\.oino\/agents$/, ''))}` }] : []),
              ] }}
              value={scope()}
              disabled={ro()}
              onChange={(v) => setScope(v === 'project' ? 'project' : 'global')}
            />
          </div>
        </Show>
        <div class="agent-row2">
          <div class="agent-field">
            <span class="agent-label">Model</span>
            <ModelPickerField hostId={props.hostId} label="Model" value={model()} fallback="Inherit (session model)" disabled={ro()} testId="agent-model" onPick={(key) => setModel(key ?? '')} />
          </div>
          <div class="agent-field">
            <span class="agent-label">Thinking</span>
            <EnumSelect
              field={{ key: 'agent.thinking', label: 'Thinking', options: [{ value: 'inherit', label: 'Inherit' }, ...THINKING_LEVELS.map((l) => ({ value: l, label: l[0]!.toUpperCase() + l.slice(1) }))] }}
              value={thinking() || 'inherit'}
              disabled={ro()}
              onChange={(v) => setThinking(v === 'inherit' ? '' : v)}
            />
          </div>
        </div>
        <div class="agent-field">
          <span class="agent-label">Tools</span>
          <label class="agent-check">
            <input type="checkbox" checked={tools() === undefined} disabled={ro()} onChange={(e) => setTools(e.currentTarget.checked ? undefined : [])} data-testid="agent-tools-all" />
            All tools (except spawning other subagents)
          </label>
          <Show when={tools() !== undefined}>
            <div class="agent-tools" role="group" aria-label="Allowed tools">
              <For each={toolOptions()}>
                {(tool) => (
                  <label class="agent-tool" data-on={tools()?.includes(tool) ? '' : undefined}>
                    <input
                      type="checkbox"
                      checked={tools()?.includes(tool) ?? false}
                      disabled={ro()}
                      onChange={(e) => {
                        const on = e.currentTarget.checked;
                        setTools((list) => (on ? [...(list ?? []), tool] : (list ?? []).filter((t) => t !== tool)));
                      }}
                    />
                    <span class="font-mono">{tool}</span>
                  </label>
                )}
              </For>
            </div>
          </Show>
        </div>
        <label class="agent-field">
          <span class="agent-label">System prompt</span>
          <textarea class="settings-textarea agent-prompt" rows={8} value={prompt()} disabled={ro()} placeholder="Left empty, a standard prompt for this agent is used." onInput={(e) => setPrompt(e.currentTarget.value)} data-testid="agent-prompt" />
        </label>
        <Show when={error()}>
          <p class="settings-field-error" role="alert" data-testid="agent-error">{error()}</p>
        </Show>
      </div>
    </BoardSheet>
  );
}
