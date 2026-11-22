import { createEffect, createSignal } from 'solid-js';
import { toAppError } from '@unipi/protocol';
import { FieldRow, type EditContext } from '../../../ui/settings/fields';
import { ModelPickerField } from '../../../ui/settings/ModelPickerField';
import { parseField } from '../../../ui/settings/types';
import { NumberStepper } from '../../../ui/NumberStepper';
import '../../../ui/settings/settings-form.css';
import { LEAD_DEFAULTS, LEAD_SETTING_FIELDS as F, LeadSettingsSchema, superagentApi, type LeadSettings } from '../api';
import type { SuperagentState } from '../state';

type NumberKey = 'retries' | 'warmMinutes' | 'questionTimeoutMinutes';
const field = (key: NumberKey) => parseField({ key: `superagent.lead.${key}`, type: 'number', label: F[key].label, description: F[key].help, default: F[key].def, min: F[key].min, max: F[key].max, integer: true });
const MODEL_FIELD = parseField({ key: 'superagent.lead.fallbackModel', type: 'model', label: F.fallbackModel.label, description: F.fallbackModel.help });

/**
 * The "Leads" settings (OIN-80): how a lead recovers from provider errors, how long an idle one stays loaded,
 * how long a question waits for the owner. Each change is checked with the schema, then saved with `superagent.configure`;
 * a refusal stays on its row and the old value comes back.
 */
export function LeadSettingsSection(props: { hostId: string; sa: SuperagentState }) {
  const [values, setValues] = createSignal<LeadSettings>(props.sa.status()?.leadSettings ?? LEAD_DEFAULTS);
  const [errors, setErrors] = createSignal<Partial<Record<keyof LeadSettings, string>>>({});
  const [saving, setSaving] = createSignal(false);
  // Follow the host's values (another device, a refresh) unless a save is in flight.
  createEffect(() => { const stored = props.sa.status()?.leadSettings; if (stored && !saving()) setValues(stored); });
  const ctx: EditContext = { ns: 'superagent', values: {}, layer: 'global', global: {}, project: {}, diagnostics: [], hostId: props.hostId, set: () => {} };

  async function commit(key: keyof LeadSettings, value: unknown) {
    if (saving()) return;
    const before = values();
    const parsed = LeadSettingsSchema.safeParse({ ...before, [key]: value });
    if (!parsed.success) { setErrors((old) => ({ ...old, [key]: parsed.error.issues.find((i) => i.path[0] === key)?.message ?? parsed.error.issues[0]?.message ?? 'Not valid.' })); return; }
    setErrors((old) => ({ ...old, [key]: undefined }));
    if (JSON.stringify(parsed.data) === JSON.stringify(before)) return;
    setValues(parsed.data); setSaving(true);
    try {
      const status = props.sa.status();
      const result = await superagentApi.configure(props.hostId, { enabled: status?.enabled ?? false, ...(status ? { pausedGraphPolicy: status.pausedGraphPolicy } : {}), lead: parsed.data });
      if (!result.ok) { setValues(before); setErrors((old) => ({ ...old, [key]: result.error.message || 'The host refused.' })); return; }
      await props.sa.refresh();
    } catch (e) { setValues(before); setErrors((old) => ({ ...old, [key]: toAppError(e).message })); }
    finally { setSaving(false); }
  }

  return <div class="settings-group sa-lead-settings" data-testid="sa-lead-settings" aria-label="Leads">
    <FieldRow field={field('retries')} ctx={ctx} error={errors().retries}>
      <NumberStepper label={F.retries.label} value={values().retries} min={F.retries.min} max={F.retries.max} integer required fallback={F.retries.def} disabled={saving()} testId="lead-retries" onChange={(v) => void commit('retries', v)} />
    </FieldRow>
    <FieldRow field={MODEL_FIELD} ctx={ctx} error={errors().fallbackModel}>
      <ModelPickerField hostId={props.hostId} label={F.fallbackModel.label} value={values().fallbackModel ?? ''} fallback="Your default model" disabled={saving()} testId="lead-fallback-model" onPick={(key) => void commit('fallbackModel', key ?? null)} />
    </FieldRow>
    <FieldRow field={field('warmMinutes')} ctx={ctx} error={errors().warmMinutes}>
      <NumberStepper label={F.warmMinutes.label} value={values().warmMinutes} min={F.warmMinutes.min} max={F.warmMinutes.max} integer required fallback={F.warmMinutes.def} disabled={saving()} testId="lead-warm" onChange={(v) => void commit('warmMinutes', v)} />
    </FieldRow>
    <FieldRow field={field('questionTimeoutMinutes')} ctx={ctx} error={errors().questionTimeoutMinutes}>
      <NumberStepper label={F.questionTimeoutMinutes.label} value={values().questionTimeoutMinutes} min={F.questionTimeoutMinutes.min} max={F.questionTimeoutMinutes.max} integer required fallback={F.questionTimeoutMinutes.def} disabled={saving()} testId="lead-question-timeout" onChange={(v) => void commit('questionTimeoutMinutes', v)} />
    </FieldRow>
    <span class="sr-only" aria-live="polite">{saving() ? 'Saving' : ''}</span>
  </div>;
}
