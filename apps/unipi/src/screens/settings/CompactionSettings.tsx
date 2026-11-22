import { Archive } from 'lucide-solid';
import { createResource, createSignal, Show } from 'solid-js';
import { piSettings, setPiSetting } from '../../settings/hostCmd';
import { settingsHostId } from '../../settings/target';
import { Row, Section } from './rows';

export default function CompactionSettings() {
  const [settings, { refetch }] = createResource(settingsHostId, piSettings);
  const [error, setError] = createSignal<string>();
  const [saving, setSaving] = createSignal(false);
  const compaction = () => {
    const reply = settings();
    return (reply?.ok ? reply.value.compaction : undefined) as { tokenLimit?: number; tokenLimitPerModel?: Record<string, number> } | undefined;
  };
  const save = async (key: string, value: unknown) => {
    if (saving()) return;
    setSaving(true); setError(undefined);
    try {
      const reply = await setPiSetting(key, value, 'global');
      if (!reply.ok) throw new Error(reply.error.message);
      await refetch();
    } catch (e) { setError(e instanceof Error ? e.message : 'Could not save compaction settings'); }
    finally { setSaving(false); }
  };
  return <Section title="Compaction" id="sec-compaction" icon={<Archive size={14} />}>
    <Show when={!settings.loading} fallback={<p role="status" class="p-4 text-text-muted">Loading compaction settings…</p>}>
      <Row label="Input token limit" description="Automatic compaction uses provider input usage, including cache reads. Model limits may lower this value.">
        <input aria-label="Input token limit" type="number" min="1" step="1" placeholder="250000" value={compaction()?.tokenLimit ?? ''} disabled={saving() || settings()?.ok === false}
          onChange={(event) => {
            const value = Number(event.currentTarget.value);
            if (!Number.isSafeInteger(value) || value <= 0) { setError('Enter a positive whole token count.'); return; }
            void save('compaction.tokenLimit', value);
          }} />
      </Row>
      <Row label="Model token limits" description="JSON object of model names or family prefixes and positive token counts.">
        <textarea aria-label="Model token limits" class="whitespace-pre-wrap" value={JSON.stringify(compaction()?.tokenLimitPerModel ?? {}, null, 2)} disabled={saving() || settings()?.ok === false}
          onChange={(event) => {
            try {
              const value: unknown = JSON.parse(event.currentTarget.value);
              if (!value || typeof value !== 'object' || Array.isArray(value) || Object.entries(value).some(([name, count]) => !name.trim() || typeof count !== 'number' || !Number.isSafeInteger(count) || count <= 0)) throw new Error('Use model names with positive whole token counts.');
              void save('compaction.tokenLimitPerModel', value);
            } catch (e) { setError(e instanceof Error ? e.message : 'Enter a valid JSON object.'); }
          }} />
      </Row>
    </Show>
    <Show when={error() || settings()?.ok === false}><p role="alert" class="p-4 text-text-muted">{error() ?? 'Could not load compaction settings. Reopen Settings to retry.'}</p></Show>
  </Section>;
}
