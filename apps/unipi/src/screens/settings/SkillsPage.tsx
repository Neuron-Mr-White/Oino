/**
 * Settings › Skills (WP-A25, G-10; owner 2026-11-05): the `skills` namespace
 * (proxy, exposure, decision model) rendered by the form engine, and one
 * "Manage skills" row opening the wide native Skills manager (every skill,
 * Enabled / Must show, global or project). The old Skills hub (a copy of this
 * form) and the cramped Skill editor overlay are gone.
 */
import { createSignal, Show } from 'solid-js';
import { ChevronRight, Sparkles } from 'lucide-solid';
import { CapabilityPage } from './CapabilityPage';
import { SkillsManager } from '../../components/skills/SkillsManager';
import { settingsHostId } from '../../settings/target';

/** Opens the manager from anywhere (a project's context menu passes its folder). */
const [managerFor, setManagerFor] = createSignal<{ hostId: string; cwd?: string; name?: string } | undefined>();
export function openSkillsManager(target: { hostId: string; cwd?: string; name?: string }): void {
  setManagerFor(target);
}
export function SkillsManagerLayer() {
  return (
    <Show when={managerFor()} keyed>
      {(t) => <SkillsManager hostId={t.hostId} cwd={t.cwd} projectName={t.name} onClose={() => setManagerFor(undefined)} />}
    </Show>
  );
}

export function SkillsPage() {
  const hostId = () => settingsHostId();
  return (
    <CapabilityPage
      page="skills"
      title="Skills"
      icon={Sparkles}
      namespace="skills"
      sectionId="sec-skills"
      blurb="Which skills the agent sees, which it must use, and what stays in the vault."
      hideFields={['states']}
      extraManage={
        <div class="settings-manage-row">
          <button type="button" class="settings-namespace-row" data-testid="manage-skills.manager" onClick={() => openSkillsManager({ hostId: hostId() })}>
            <span class="min-w-0 flex-1">
              <span class="block text-[15px] text-text truncate">Manage skills</span>
              <span class="block text-xs text-text-dim truncate">Every skill: Enabled / Must show, globally or per project</span>
            </span>
            <ChevronRight size={15} class="text-text-dim shrink-0" />
          </button>
        </div>
      }
    />
  );
}
