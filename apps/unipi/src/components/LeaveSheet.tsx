/**
 * UNI-253: "Turn off for this phone" / "Turn off on the PC", and the confirm
 * that says what each one actually does.
 *
 * A phone can always remove *itself* from a feature that is on for it; turning
 * the feature off for everyone is only offered when this device has it (the
 * host refuses otherwise). Neither row can enable anything — `feature_leave`
 * only ever removes access.
 */
import { For, Show, createSignal } from 'solid-js';
import { PowerOff } from 'lucide-solid';
import { FEATURE_LABEL, isLeaveUnsupported, leaveFeature, leaveScopes } from '../lib/sharing';
import type { FeatureKind } from '@unipi/protocol';
import { BoardSheet } from './board/BoardSheet';

export function LeaveSheet(props: {
  open: boolean;
  onClose: () => void;
  hostId: string;
  feature: FeatureKind;
  /** Is the feature on for this device right now? Decides whether "on the PC" is offered. */
  allowed: boolean;
}) {
  const [busy, setBusy] = createSignal<'self' | 'all' | undefined>();
  const [error, setError] = createSignal<string | undefined>();
  const [done, setDone] = createSignal<string | undefined>();
  const rows = () => leaveScopes(props.feature, props.allowed);

  const leave = async (scope: 'self' | 'all') => {
    setBusy(scope);
    setError(undefined);
    const r = await leaveFeature(props.hostId, props.feature, scope);
    setBusy(undefined);
    if (!r.ok) {
      setError(isLeaveUnsupported(r.error) ? 'This PC runs an older UniPi that can’t do this yet.' : r.error.message);
      return;
    }
    setDone(r.value.left ? `Done. ${r.value.now_enabled ? 'Still on for the other phones.' : 'It is off for everyone now.'}` : 'Nothing to undo — this phone didn’t have it.');
  };

  return (
    <BoardSheet open={props.open} onClose={props.onClose} label={`Turn off ${FEATURE_LABEL[props.feature].toLowerCase()}`} testId="leave-sheet">
      <Show
        when={!done()}
        fallback={
          <div class="settings-group rounded-card bg-surface-2/60">
            <p class="text-sm" data-testid="leave-done">
              {done()}
            </p>
            <button type="button" class="btn btn-primary btn-block" onClick={props.onClose}>
              Done
            </button>
          </div>
        }
      >
        <p class="preview-muted" style={{ margin: '0 0 8px' }}>
          Turning a feature off never turns anything on.
        </p>
        <div class="settings-group rounded-card bg-surface-2/60">
          <For each={rows()}>
            {(row) => (
              <button type="button" class="settings-row" data-testid={`leave-${row.scope}`} disabled={!!busy()} onClick={() => void leave(row.scope)}>
                <span class="settings-icon">
                  <PowerOff size={18} />
                </span>
                <span class="min-w-0 flex-1 text-left">
                  <span class="block text-[15px] text-text">{row.label}</span>
                  <span class="block text-xs text-text-dim mt-0.5">{row.detail}</span>
                </span>
              </button>
            )}
          </For>
        </div>
        <Show when={error()}>
          <p class="setup-error-message mt-2" role="alert" data-testid="leave-error">
            {error()}
          </p>
        </Show>
      </Show>
    </BoardSheet>
  );
}

/** The button that opens the sheet, wherever a feature is on for this phone. */
export function LeaveButton(props: { onClick: () => void; label?: string; testId: string }) {
  return (
    <button type="button" class="btn btn-secondary btn-sm" data-testid={props.testId} onClick={props.onClick}>
      <PowerOff size={14} /> {props.label ?? 'Turn off'}
    </button>
  );
}
