/**
 * Kanboard settings (UNI-131) — the web's SettingsDialog as a sheet, in
 * the app's Settings look: Summaries (model, instruction with Customize /
 * Reset to default, the agent command read-only), Sessions (sessions at
 * once, new tasks per turn, chain gate), Archive (auto-archive, cold
 * storage). The model list (the web's ModelCombobox) opens as a page
 * inside the sheet — searchable, grouped by provider, "pi default" first,
 * Refresh. These are the daemon's settings: they apply to every project
 * on that PC. Unsaved changes ask before closing.
 */
import { batch, createEffect, createMemo, createSignal, For, on, Show } from 'solid-js';
import { ArrowLeft, Check, ChevronRight, Minus, Plus, RefreshCw, Search } from 'lucide-solid';
import { api, describe, type Settings } from '../../lib/board/api';
import { boardStore } from '../../lib/board/store';
import { boardUi } from '../../lib/board/ui';
import { BoardSheet } from './BoardSheet';

type Cat = 'summaries' | 'sessions' | 'archive';
const CATS: Array<[Cat, string]> = [
  ['summaries', 'Summaries'],
  ['sessions', 'Sessions'],
  ['archive', 'Archive'],
];

type Draft = {
  model: string;
  instruction: string;
  custom: boolean;
  maxSessions: number;
  turnAddLimit: number;
  chainGate: string;
  archiveDays: number;
  retentionDays: number;
};

function draftOf(settings: Settings): Draft {
  const limits = settings.sessions ?? settings.runner;
  return {
    model: settings.summaryModel,
    instruction: settings.summaryInstruction,
    custom: settings.summaryInstruction !== settings.defaultSummaryInstruction,
    maxSessions: limits?.maxSessions ?? 2,
    turnAddLimit: limits?.turnAddLimit ?? 20,
    chainGate: limits?.chainGate ?? 'in_review',
    archiveDays: settings.archive?.archiveAfterDays ?? 0,
    retentionDays: settings.archive?.retentionDays ?? 90,
  };
}

export function Stepper(props: { label: string; value: number; min: number; onChange: (value: number) => void; zero?: string }) {
  return (
    <div class="num-stepper" role="group" aria-label={props.label}>
      <button type="button" aria-label={`Decrease ${props.label}`} disabled={props.value <= props.min} onClick={() => props.onChange(Math.max(props.min, props.value - 1))}>
        <Minus size={16} />
      </button>
      <input
        type="number"
        inputmode="numeric"
        min={props.min}
        aria-label={props.label}
        value={props.value}
        onInput={(event) => {
          const raw = event.currentTarget.value;
          if (raw === '') return;
          props.onChange(Math.max(props.min, Math.floor(Number(raw)) || props.min));
        }}
        onBlur={(event) => (event.currentTarget.value = String(props.value))}
      />
      <button type="button" aria-label={`Increase ${props.label}`} onClick={() => props.onChange(props.value + 1)}>
        <Plus size={16} />
      </button>
    </div>
  );
}

function Row(props: { label: string; hint?: string; children: any }) {
  return (
    <div class="kb-setting">
      <div class="min-w-0 flex-1">
        <p class="kb-setting-label">{props.label}</p>
        <Show when={props.hint}>
          <p class="kb-setting-hint">{props.hint}</p>
        </Show>
      </div>
      <div class="shrink-0">{props.children}</div>
    </div>
  );
}

export function BoardSettingsSheet(props: { pcName?: string }) {
  const store = boardStore();
  const ui = boardUi();
  const [loaded, setLoaded] = createSignal<Settings | null>(null);
  const [loadError, setLoadError] = createSignal<string | undefined>();
  const [draft, setDraft] = createSignal<Draft | null>(null);
  const [cat, setCat] = createSignal<Cat>('summaries');
  const [page, setPage] = createSignal<'form' | 'model'>('form');
  const [models, setModels] = createSignal<string[]>([]);
  const [modelsState, setModelsState] = createSignal<'loading' | 'ok' | 'error'>('loading');
  const [modelsError, setModelsError] = createSignal('');
  const [saving, setSaving] = createSignal(false);
  const [saveError, setSaveError] = createSignal<string | undefined>();
  const [confirmDiscard, setConfirmDiscard] = createSignal(false);
  const hostId = () => store.target()?.hostId;

  async function loadModels(refresh = false) {
    const id = hostId();
    if (!id) return;
    setModelsState('loading');
    try {
      const payload = await api.models(id, refresh);
      setModels(payload.models ?? []);
      setModelsError('');
      setModelsState('ok');
    } catch (error) {
      setModelsError(describe(error));
      setModelsState('error');
    }
  }

  createEffect(
    on(ui.settingsOpen, (open) => {
      if (!open) return;
      const id = hostId();
      batch(() => {
        setLoaded(null);
        setDraft(null);
        setLoadError(undefined);
        setSaveError(undefined);
        setConfirmDiscard(false);
        setPage('form');
        setCat('summaries');
        setModels([]);
      });
      if (!id) return;
      api
        .settings(id)
        .then((settings) => {
          setLoaded(settings);
          setDraft(draftOf(settings));
        })
        .catch((error) => setLoadError(describe(error)));
      void loadModels();
    }),
  );

  const patch = <K extends keyof Draft>(key: K, value: Draft[K]) => setDraft((current) => (current ? { ...current, [key]: value } : current));
  const dirty = createMemo(() => {
    const base = loaded();
    const now = draft();
    if (!base || !now) return false;
    const orig = draftOf(base);
    return (Object.keys(orig) as (keyof Draft)[]).some((key) => orig[key] !== now[key]);
  });

  const close = () => {
    setConfirmDiscard(false);
    ui.setSettingsOpen(false);
  };
  const requestClose = () => {
    if (page() === 'model') return setPage('form');
    if (confirmDiscard() || !dirty()) return close();
    setConfirmDiscard(true);
  };

  async function save() {
    const id = hostId();
    const now = draft();
    if (!id || !now || saving()) return;
    setSaving(true);
    setSaveError(undefined);
    try {
      await api.saveSettings(id, {
        summaryModel: now.model.trim(),
        summaryInstruction: now.custom ? now.instruction : '',
        maxSessions: now.maxSessions,
        turnAddLimit: now.turnAddLimit,
        chainGate: now.chainGate,
        archiveAfterDays: now.archiveDays,
        retentionDays: now.retentionDays,
      });
      ui.toast('Settings saved', 'success');
      close();
      void store.refresh(); // rules carry chainGate / maxSessions
    } catch (error) {
      setSaveError(describe(error));
    } finally {
      setSaving(false);
    }
  }

  return (
    <BoardSheet
      open={ui.settingsOpen()}
      onClose={requestClose}
      label={page() === 'model' ? 'Summary model' : 'Board settings'}
      testId="board-settings-sheet"
      class="board-sheet-tall"
      guarded={dirty()}
      title={
        <span class="flex items-center gap-2 min-w-0">
          <Show when={page() === 'model'}>
            <button type="button" class="ghost-icon-button -ml-2" aria-label="Back to settings" onClick={() => setPage('form')}>
              <ArrowLeft size={20} />
            </button>
          </Show>
          <span class="truncate">{page() === 'model' ? 'Summary model' : 'Board settings'}</span>
          <Show when={page() === 'form' && props.pcName}>
            <span class="new-task-project truncate">{props.pcName}</span>
          </Show>
        </span>
      }
      footer={
        <Show
          when={page() === 'form'}
          fallback={
            <button type="button" class="pair-secondary" onClick={() => setPage('form')}>
              Done
            </button>
          }
        >
          <Show
            when={!confirmDiscard()}
            fallback={
              <div class="discard-strip" role="alertdialog" aria-label="Discard changes?">
                <span class="flex-1 text-[15px]">Discard your changes?</span>
                <button type="button" class="board-text-button" onClick={() => setConfirmDiscard(false)}>
                  Keep editing
                </button>
                <button type="button" class="discard-button" onClick={close}>
                  Discard
                </button>
              </div>
            }
          >
            <div class="flex gap-3">
              <button type="button" class="pair-secondary flex-1" onClick={requestClose}>
                Cancel
              </button>
              <button type="button" class="pair-primary flex-[1.4]" disabled={saving() || !draft() || !dirty()} onClick={() => void save()}>
                {saving() ? 'Saving…' : 'Save'}
              </button>
            </div>
          </Show>
        </Show>
      }
    >
      <Show when={page() === 'form'}>
        <div class="seg seg-cats" role="tablist" aria-label="Settings categories">
          <For each={CATS}>
            {([key, label]) => (
              <button type="button" role="tab" class="seg-option" aria-selected={cat() === key} aria-checked={cat() === key} onClick={() => setCat(key)}>
                {label}
              </button>
            )}
          </For>
        </div>
        <Show when={loadError()}>
          <p class="sheet-error" role="alert">
            {loadError()}
          </p>
        </Show>
        <Show when={draft()} fallback={<Show when={!loadError()}><div class="skeleton kb-skeleton" /></Show>}>
          {(now) => (
            <>
              <Show when={cat() === 'summaries'}>
                <div class="kb-group">
                  <button type="button" class="kb-setting kb-setting-button" aria-label={`Summary model: ${now().model || 'pi default'}`} onClick={() => setPage('model')}>
                    <div class="min-w-0 flex-1">
                      <p class="kb-setting-label">Model</p>
                      <p class="kb-setting-value truncate" classList={{ dim: !now().model }}>
                        {now().model || 'pi default'}
                      </p>
                    </div>
                    <ChevronRight size={18} class="text-text-dim shrink-0" />
                  </button>
                  <div class="kb-setting kb-setting-stack">
                    <div class="flex items-center w-full">
                      <p class="kb-setting-label flex-1">Instruction</p>
                      <Show
                        when={now().custom}
                        fallback={
                          <button type="button" class="board-text-button" onClick={() => patch('custom', true)}>
                            Customize
                          </button>
                        }
                      >
                        <button
                          type="button"
                          class="board-text-button"
                          onClick={() => {
                            patch('instruction', loaded()?.defaultSummaryInstruction ?? '');
                            patch('custom', false);
                          }}
                        >
                          Reset to default
                        </button>
                      </Show>
                    </div>
                    <Show when={now().custom} fallback={<p class="kb-setting-hint">Default: changelog bullets in Conventional Commit style</p>}>
                      <textarea
                        class="sheet-textarea tall"
                        aria-label="Summary instruction"
                        value={now().instruction}
                        onInput={(event) => patch('instruction', event.currentTarget.value)}
                      />
                    </Show>
                    <p class="kb-setting-hint">Plain-language style rules are always applied.</p>
                  </div>
                  <Show when={(loaded()?.piCommand ?? []).length > 0}>
                    <div class="kb-setting kb-setting-stack">
                      <p class="kb-setting-label">Agent command</p>
                      <code class="kb-command">{loaded()!.piCommand.join(' ')}</code>
                      <p class="kb-setting-hint">Runs on {props.pcName ?? 'the PC'} to write summaries. Set it in the kanboard config there.</p>
                    </div>
                  </Show>
                </div>
              </Show>
              <Show when={cat() === 'sessions'}>
                <div class="kb-group">
                  <Row label="Sessions at once" hint="Distinct agent sessions holding In Progress tasks per project.">
                    <Stepper label="Sessions at once" value={now().maxSessions} min={1} onChange={(value) => patch('maxSessions', value)} />
                  </Row>
                  <Row label="New tasks per turn" hint={`How many tasks an agent may add per turn${now().turnAddLimit === 0 ? ' — unlimited' : ''}. 0 = unlimited.`}>
                    <Stepper label="New tasks per turn" value={now().turnAddLimit} min={0} onChange={(value) => patch('turnAddLimit', value)} />
                  </Row>
                  <div class="kb-setting kb-setting-stack">
                    <p class="kb-setting-label">Chain gate</p>
                    <p class="kb-setting-hint">When a dependency counts as satisfied.</p>
                    <div class="seg w-full mt-1" role="radiogroup" aria-label="Chain gate">
                      <For each={['in_review', 'done']}>
                        {(gate) => (
                          <button type="button" role="radio" class="seg-option" aria-checked={now().chainGate === gate} onClick={() => patch('chainGate', gate)}>
                            {gate === 'done' ? 'Done' : 'In Review'}
                          </button>
                        )}
                      </For>
                    </div>
                  </div>
                </div>
              </Show>
              <Show when={cat() === 'archive'}>
                <div class="kb-group">
                  <Row label="Auto-archive after" hint={now().archiveDays === 0 ? 'Off — Done / Cancelled tasks stay on the board.' : `Done / Cancelled tasks leave the board after ${now().archiveDays} day${now().archiveDays === 1 ? '' : 's'}.`}>
                    <Stepper label="Auto-archive after days" value={now().archiveDays} min={0} onChange={(value) => patch('archiveDays', value)} />
                  </Row>
                  <Row label="Cold storage after" hint={now().retentionDays === 0 ? 'Off — archived tasks stay.' : `Archived tasks move to cold storage after ${now().retentionDays} days.`}>
                    <Stepper label="Cold storage after days" value={now().retentionDays} min={0} onChange={(value) => patch('retentionDays', value)} />
                  </Row>
                </div>
              </Show>
              <p class="sheet-hint mt-3">These settings belong to the board on {props.pcName ?? 'this PC'} and apply to all its projects.</p>
              <Show when={saveError()}>
                <p class="sheet-error" role="alert">
                  {saveError()}
                </p>
              </Show>
            </>
          )}
        </Show>
      </Show>
      <Show when={page() === 'model' && draft()}>
        <ModelPicker
          models={models()}
          state={modelsState()}
          error={modelsError()}
          value={draft()!.model}
          onChange={(value) => {
            patch('model', value);
            setPage('form');
          }}
          onRefresh={() => void loadModels(true)}
        />
      </Show>
    </BoardSheet>
  );
}

function ModelPicker(props: { models: string[]; state: 'loading' | 'ok' | 'error'; error: string; value: string; onChange: (value: string) => void; onRefresh: () => void }) {
  const [needle, setNeedle] = createSignal('');
  const groups = createMemo(() => {
    const n = needle().trim().toLowerCase();
    const map = new Map<string, string[]>();
    for (const entry of props.models) {
      if (n && !entry.toLowerCase().includes(n)) continue;
      const provider = entry.includes('/') ? entry.split('/')[0]! : 'other';
      map.set(provider, [...(map.get(provider) ?? []), entry]);
    }
    return [...map.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  });
  const Option = (o: { value: string; label: string }) => (
    <button type="button" role="option" class="search-row" aria-selected={props.value === o.value} onClick={() => props.onChange(o.value)}>
      <span class="min-w-0 flex-1 truncate">{o.label}</span>
      <Show when={props.value === o.value}>
        <Check size={18} class="model-check" />
      </Show>
    </button>
  );
  return (
    <div class="picker-page">
      <div class="search-field">
        <Search size={18} class="text-text-dim shrink-0" />
        <input placeholder="Filter models" aria-label="Filter models" value={needle()} onInput={(event) => setNeedle(event.currentTarget.value)} />
        <button type="button" class="search-clear" aria-label="Refresh the model list" onClick={props.onRefresh}>
          <RefreshCw size={17} class={props.state === 'loading' ? 'refresh-spinning' : ''} />
        </button>
      </div>
      <div class="search-results" role="listbox" aria-label="Summary models">
        <Option value="" label="pi default" />
        <Show when={props.value && !needle().trim()}>
          <h4 class="board-section-label">Selected</h4>
          <Option value={props.value} label={props.value} />
        </Show>
        <Show when={props.state === 'loading' && props.models.length === 0}>
          <p class="text-sm text-text-dim py-3">Loading models…</p>
        </Show>
        <Show when={props.state === 'error'}>
          <p class="sheet-error" role="alert">
            {props.error}
          </p>
        </Show>
        <For each={groups()}>
          {([provider, entries]) => (
            <>
              <h4 class="board-section-label">{provider}</h4>
              <For each={entries}>{(entry) => <Option value={entry} label={entry.slice(provider.length + 1) || entry} />}</For>
            </>
          )}
        </For>
        <Show when={props.state === 'ok' && groups().length === 0 && props.models.length > 0}>
          <p class="text-sm text-text-dim py-3">No model matches.</p>
        </Show>
        <button type="button" class="section-more" onClick={props.onRefresh}>
          Refresh the model list
        </button>
      </div>
    </div>
  );
}
