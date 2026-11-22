import { createMemo, createSignal, For, Show } from 'solid-js';
import { Pencil, Plus, Star, Trash2 } from 'lucide-solid';
import { z } from 'zod';
import { confirmSheet } from '../phone';
import { SheetFrame } from '../phone/Sheet';
import { AutonomySchema, MODES, PresetSchema, REVIEWS, type Preset } from './api';
import { MODE_LABEL } from './format';
import type { SuperagentState } from './state';
import { ChipSelect, Toggle } from './parts';

/** Compact preset picker (Start). Falls back to nothing when the host has no presets. */
export function PresetSelect(props: { sa: SuperagentState; value: string | null; onChange: (id: string) => void }) {
  const list = () => props.sa.presets().list;
  return <Show when={list().length}>
    <ChipSelect class="sa-preset-select" label="Preset" value={props.value ?? ''} onChange={props.onChange}
      options={list().map((p) => ({ value: p.id, label: p.id === props.sa.presets().defaultId ? `${p.label} (default)` : p.label }))} />
  </Show>;
}

const FormSchema = z.object({
  label: z.string().trim().min(1, 'Name the preset.').max(40, 'Keep the name under 40 characters.'),
  maxParallel: z.string().trim().refine((v) => v === '' || (/^\d+$/.test(v) && Number(v) >= 1 && Number(v) <= 16), 'Parallel work is 1 to 16, or empty.'),
});
const slug = (label: string) => label.trim().toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '') || 'preset';
const blank = (): Preset => ({ id: '', label: '', description: '', model: null, thinking: null, autonomy: 'high', mode: 'none', maxParallel: null, review: 'none', profiles: null, priority: false });

function Editor(props: { sa: SuperagentState; initial: Preset; onDone: () => void }) {
  const isNew = !props.initial.id;
  const [draft, setDraft] = createSignal<Preset>(props.initial);
  const [parallel, setParallel] = createSignal(props.initial.maxParallel === null ? '' : String(props.initial.maxParallel));
  const [problem, setProblem] = createSignal('');
  const patch = (p: Partial<Preset>) => setDraft((d) => ({ ...d, ...p }));
  async function save() {
    const parsed = FormSchema.safeParse({ label: draft().label, maxParallel: parallel() });
    if (!parsed.success) { setProblem(parsed.error.issues[0]!.message); return; }
    let id = draft().id;
    if (isNew) {
      const taken = new Set(props.sa.presets().list.map((p) => p.id));
      const base = slug(parsed.data.label);
      id = base;
      for (let n = 2; taken.has(id); n++) id = `${base}-${n}`;
    }
    const next = PresetSchema.parse({ ...draft(), id, label: parsed.data.label, maxParallel: parallel().trim() === '' ? null : Number(parallel()) });
    setProblem('');
    if (await props.sa.savePreset(next)) props.onDone();
  }
  return <form class="sa-preset-form" onSubmit={(e) => { e.preventDefault(); void save(); }}>
    <label class="sa-field"><span>Name</span><input aria-label="Preset name" value={draft().label} onInput={(e) => patch({ label: e.currentTarget.value })} /></label>
    <label class="sa-field"><span>Description</span><input aria-label="Preset description" value={draft().description} onInput={(e) => patch({ description: e.currentTarget.value })} /></label>
    <div class="sa-field-row">
      <label class="sa-field"><span>Mode</span><ChipSelect label="Preset mode" value={draft().mode} options={MODES.map((m) => ({ value: m, label: MODE_LABEL[m] }))} onChange={(v) => patch({ mode: v as Preset['mode'] })} /></label>
      <label class="sa-field"><span>Autonomy</span><ChipSelect label="Preset autonomy" value={draft().autonomy} options={AutonomySchema.options.map((a) => ({ value: a, label: a[0]!.toUpperCase() + a.slice(1) }))} onChange={(v) => patch({ autonomy: v as Preset['autonomy'] })} /></label>
    </div>
    <div class="sa-field-row">
      <label class="sa-field"><span>Review</span><ChipSelect label="Preset review" value={draft().review} options={REVIEWS.map((r) => ({ value: r, label: r[0]!.toUpperCase() + r.slice(1) }))} onChange={(v) => patch({ review: v as Preset['review'] })} /></label>
      <label class="sa-field"><span>Parallel work</span><input aria-label="Preset parallel work" inputmode="numeric" placeholder="Auto" value={parallel()} onInput={(e) => setParallel(e.currentTarget.value)} /></label>
    </div>
    <div class="sa-field-inline"><span>Urgent: jump the queue</span><Toggle label="Priority" checked={draft().priority} onChange={(v) => patch({ priority: v })} /></div>
    <Show when={problem() || props.sa.error()}><p class="sa-message" role="alert">{problem() || props.sa.error()}</p></Show>
    <div class="sa-actions"><button type="submit" class="sa-btn" data-tone="primary" disabled={props.sa.isBusy('preset')}>Save preset</button><button type="button" class="sa-btn" onClick={props.onDone}>Cancel</button></div>
  </form>;
}

/** List, save, delete and set the default preset. Model and thinking keep their value when editing. */
export function PresetManager(props: { sa: SuperagentState; open: boolean; onClose: () => void }) {
  const [editing, setEditing] = createSignal<Preset>();
  const list = createMemo(() => props.sa.presets().list);
  const defaultId = () => props.sa.presets().defaultId;
  async function remove(p: Preset) {
    if (await confirmSheet({ title: `Delete ${p.label}?`, body: 'Tickets already started with it keep running.', confirm: 'Delete preset', tone: 'danger' })) await props.sa.deletePreset(p.id);
  }
  return <SheetFrame open={props.open} onClose={() => { if (editing()) setEditing(undefined); else props.onClose(); }} label="Presets" level={2} title={editing() ? (editing()!.id ? 'Edit preset' : 'New preset') : 'Presets'} detents={['full']}
    headerAction={undefined}>
    <div class="sa-presets" data-testid="sa-preset-manager">
      <Show when={editing()} fallback={<>
        <Show when={list().length} fallback={<p class="sa-empty">No presets yet.</p>}>
          <ul class="sa-preset-list">
            <For each={list()}>{(p) => <li class="sa-preset-row">
              <div class="sa-preset-main"><strong>{p.label}</strong><span class="sa-sub">{MODE_LABEL[p.mode]} · {p.autonomy} autonomy{p.priority ? ' · urgent' : ''}</span></div>
              <button type="button" class="sa-icon-btn sa-star" aria-label={p.id === defaultId() ? `${p.label} is the default` : `Make ${p.label} the default`} title={p.id === defaultId() ? 'Default preset' : 'Make default'} aria-pressed={p.id === defaultId()} disabled={p.id === defaultId() || props.sa.isBusy('preset')} onClick={() => void props.sa.setDefaultPreset(p.id)}><Star size={16} fill={p.id === defaultId() ? 'currentColor' : 'none'} /></button>
              <button type="button" class="sa-icon-btn" aria-label={`Edit ${p.label}`} onClick={() => setEditing(p)}><Pencil size={16} /></button>
              <button type="button" class="sa-icon-btn" aria-label={`Delete ${p.label}`} title={p.id === defaultId() ? 'Make another preset the default first' : undefined} disabled={p.id === defaultId() || props.sa.isBusy('preset')} onClick={() => void remove(p)}><Trash2 size={16} /></button>
            </li>}</For>
          </ul>
        </Show>
        <button type="button" class="sa-btn sa-add" onClick={() => setEditing(blank())}><Plus size={16} /> New preset</button>
        <Show when={props.sa.error()}><p class="sa-message" role="alert">{props.sa.error()}</p></Show>
      </>}>{(initial) => <Editor sa={props.sa} initial={initial()} onDone={() => setEditing(undefined)} />}</Show>
    </div>
  </SheetFrame>;
}
