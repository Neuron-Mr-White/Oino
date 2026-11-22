/**
 * The settings model control: a thin trigger for the ONE model dialog the
 * composer's model chips open (`HostModelDialog`, titled by the field's label):
 * Now line, Single / Fusion tabs, search over every model, Cycle set, "Use at
 * startup" and "Use default" (clears the field). It needs no open chat. The
 * button shows the model's display name + provider; an empty value shows the
 * field's documented fallback ("Session model" / "Inherit"). Typing an id is
 * only the dialog's collapsed "Advanced" escape hatch.
 */
import { Show, createMemo, createResource, createSignal } from 'solid-js';
import { ChevronDown, Cpu } from 'lucide-solid';
import { HostModelDialog, loadHostModels } from '../../components/model/HostModelPicker';
import { cachedModels } from '../../components/draft/hostCache';
import { modelKey, modelName } from '../../lib/model/picker';
import { prettyModelId } from '../../lib/model/displayName';

export function ModelPickerField(props: {
  hostId: string;
  label: string;
  /** `provider/model` (empty = unset). */
  value: string;
  /** What an empty value means ("Session model", "Inherit"…). */
  fallback: string;
  disabled?: boolean;
  testId: string;
  onPick: (key: string | undefined) => void;
}) {
  const [open, setOpen] = createSignal(false);
  // The button's display name: the remembered list, else this PC's (no chat needed).
  const [names] = createResource(
    () => props.hostId,
    async (hostId) => cachedModels(hostId) ?? (await loadHostModels(hostId)).models,
  );
  const known = createMemo(() => {
    const value = props.value.trim();
    if (!value) return undefined;
    return (names.latest ?? cachedModels(props.hostId) ?? []).find((m) => modelKey(m) === value);
  });
  const name = () => {
    const value = props.value.trim();
    if (!value) return props.fallback;
    const model = known();
    return model ? modelName(model) : prettyModelId(value);
  };
  const provider = () => {
    const value = props.value.trim();
    return value.includes('/') ? value.slice(0, value.indexOf('/')) : '';
  };
  const close = () => setOpen(false);
  let btn: HTMLButtonElement | undefined;
  return (
    <>
      <button
        type="button"
        class="settings-model-btn"
        aria-label={`${props.label}: ${name()}. Change`}
        aria-haspopup="dialog"
        aria-expanded={open()}
        disabled={props.disabled}
        data-empty={props.value.trim() ? undefined : ''}
        data-testid={props.testId}
        ref={btn}
        onClick={() => (open() ? close() : setOpen(true))}
      >
        <Cpu size={14} class="settings-model-ic" aria-hidden="true" />
        <span class="settings-model-name">{name()}</span>
        <Show when={provider()}>
          <span class="settings-model-prov">{provider()}</span>
        </Show>
        <ChevronDown size={14} class="settings-model-chev" aria-hidden="true" />
      </button>
      <Show when={open()}>
        <HostModelDialog
          hostId={props.hostId}
          title={props.label}
          testId={`model-pop-${props.testId}`}
          onClose={close}
          trigger={() => btn}
          value={props.value}
          fallback={props.fallback}
          onPick={props.onPick}
        />
      </Show>
    </>
  );
}
