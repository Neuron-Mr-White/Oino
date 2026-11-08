/**
 * "Lead model…" (sidebar project menu): the Superagent lead's model + thinking level for one
 * board project. Empty = the preset's own. Saved through `superagent.configure_project_lead`;
 * a running lead keeps its model until it is restarted.
 */
import { createSignal, For, Show, type JSX } from 'solid-js';
import { toAppError } from '@unipi/protocol';
import { superagentApi, type ProjectRow } from '../superagent/api';
import { BoardSheet } from '../board/BoardSheet';
import { ModelPickerField } from '../../ui/settings/ModelPickerField';
import '../../ui/settings/settings-form.css';
import { THINKING_ORDER, thinkingLabel } from '../../lib/model/thinking';
import { showActionNotice } from '../../lib/timeline/actions';

const trimRoot = (path: string) => path.replace(/[\\/]+$/, '');

/** The Superagent board project whose folder is `cwd` (trailing slashes ignored); none → undefined. */
export function leadProjectFor(projects: readonly ProjectRow[] | null | undefined, cwd: string | undefined): ProjectRow | undefined {
  const want = cwd ? trimRoot(cwd) : '';
  if (!want) return undefined;
  return projects?.find((p) => !!p.root && trimRoot(p.root) === want);
}

export function LeadModelDialog(props: { hostId: string; project: ProjectRow; name?: string; onClose: () => void }): JSX.Element {
  const name = () => props.project.name || props.name || props.project.projectId;
  const [model, setModel] = createSignal(props.project.leadModel?.trim() ?? '');
  const [thinking, setThinking] = createSignal(props.project.leadThinking?.trim() ?? '');
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal('');
  const isSet = () => !!(props.project.leadModel?.trim() || props.project.leadThinking?.trim());
  const levels = () => (thinking() && !(THINKING_ORDER as readonly string[]).includes(thinking()) ? [...THINKING_ORDER, thinking()] : [...THINKING_ORDER]);

  async function save(nextModel: string | null, nextThinking: string | null, notice: string) {
    if (busy()) return;
    setBusy(true);
    setError('');
    try {
      const result = await superagentApi.configureProjectLead(props.hostId, props.project.projectId, nextModel, nextThinking);
      if (!result.ok) return setError(result.error.message);
      showActionNotice(notice);
      props.onClose();
    } catch (e) {
      setError(toAppError(e).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <BoardSheet
      open
      onClose={props.onClose}
      label={`Lead model · ${name()}`}
      testId="lead-model-dialog"
      footer={
        <div class="leadm-foot">
          <Show when={isSet()}>
            <button type="button" class="settings-btn" disabled={busy()} data-testid="lead-model-reset" onClick={() => void save(null, null, `Lead model reset for ${name()}`)}>
              Reset
            </button>
          </Show>
          <span class="flex-1" />
          <button type="button" class="settings-btn" disabled={busy()} data-testid="lead-model-cancel" onClick={props.onClose}>
            Cancel
          </button>
          <button
            type="button"
            class="btn-primary settings-btn"
            disabled={busy()}
            data-testid="lead-model-save"
            onClick={() => void save(model().trim() || null, thinking() || null, `Lead model saved for ${name()}`)}
          >
            Save
          </button>
        </div>
      }
    >
      <div class="leadm" data-testid="lead-model-body">
        <p class="leadm-help">Used when Superagent or auto-start starts this project’s lead. A running lead keeps its model until it is restarted.</p>
        <div class="leadm-field">
          <span class="leadm-label">Model</span>
          <ModelPickerField
            hostId={props.hostId}
            label="Model"
            value={model()}
            fallback="Default (preset)"
            disabled={busy()}
            testId="lead-model-pick"
            onPick={(key) => setModel(key ?? '')}
          />
        </div>
        <div class="leadm-field">
          <label class="leadm-label" for="leadm-thinking">Thinking</label>
          <select
            id="leadm-thinking"
            class="settings-input settings-select leadm-select"
            disabled={busy()}
            value={thinking()}
            data-testid="lead-model-thinking"
            onChange={(e) => setThinking(e.currentTarget.value)}
          >
            <option value="" selected={thinking() === ''}>Default</option>
            <For each={levels()}>{(level) => <option value={level} selected={thinking() === level}>{thinkingLabel(level)}</option>}</For>
          </select>
        </div>
        <Show when={error()}>
          <p class="leadm-error" role="alert" data-testid="lead-model-error">{error()}</p>
        </Show>
      </div>
    </BoardSheet>
  );
}
