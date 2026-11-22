import { createResource, createSignal, Show } from 'solid-js';
import { ShieldCheck } from 'lucide-solid';
import { AUTONOMY_LEVELS, isAutonomyLevel } from '../../lib/session/autonomy';
import { piSettings, setPiSetting } from '../../settings/hostCmd';
import { settingsHostId } from '../../settings/target';
import { Row, Section, Segmented } from './rows';

export function AutonomySettings() {
  const [settings, { refetch }] = createResource(settingsHostId, piSettings);
  const [saving, setSaving] = createSignal(false);
  const [error, setError] = createSignal<string>();
  const level = () => {
    const result = settings();
    const value = ((result?.ok ? result.value : undefined)?.autonomy as { defaultLevel?: unknown } | undefined)?.defaultLevel;
    return isAutonomyLevel(value) ? value : 'medium';
  };
  const save = async (value: string) => {
    if (!isAutonomyLevel(value) || saving()) return;
    setSaving(true); setError(undefined);
    try {
      const reply = await setPiSetting('autonomy.defaultLevel', value, 'global');
      if (!reply.ok) throw new Error(reply.error.message);
      await refetch();
    } catch (e) { setError(e instanceof Error ? e.message : 'Could not save autonomy'); }
    finally { setSaving(false); }
  };
  return <Section title="Autonomy" id="sec-autonomy" icon={<ShieldCheck size={14} />}>
    <Show when={!settings.loading} fallback={<p class="p-4 text-text-muted" role="status">Loading autonomy…</p>}>
      <Row label="Default autonomy" description="For new interactive chats and cron. Plan mode stays separate.">
        <fieldset disabled={saving() || settings()?.ok === false} aria-label="Default autonomy">
          <Segmented label="Default autonomy" options={AUTONOMY_LEVELS} value={level()} labels={{ low: 'Low', medium: 'Medium', high: 'High' }} onChange={(v) => void save(v)} />
        </fieldset>
      </Row>
    </Show>
    <Show when={error() || settings()?.ok === false}><p class="p-4 text-text-muted" role="alert">{error() ?? (settings()?.ok === false ? 'Could not load autonomy. Reopen Settings to retry.' : '')}</p></Show>
  </Section>;
}
