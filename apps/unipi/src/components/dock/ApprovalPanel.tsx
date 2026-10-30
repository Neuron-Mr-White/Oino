/**
 * The `approval` dialog (WP-A9, Z-47; ui-protocol §3.2 `approval` row):
 * unipi's permission card in the bottom dock — origin badge, summary, the
 * `detail` node in a 40 vh scroll area, an optional "tell the agent what to
 * do instead" note, and the protocol's options by kind (allow once / always
 * allow / deny / deny with note).
 *
 * The answer is `{optionId, feedback?}` (the shape `unipi.workflow` reads);
 * an option that needs a note (`deny_note`) first focuses the note instead
 * of denying silently.
 *
 * No autofocus and no bare-key shortcuts: a keystroke meant for the composer
 * must never approve a shell command (v1 review P1-4). Esc handling belongs
 * to the conversation's escape rules (A11).
 */
import { createSignal, Index, Show } from 'solid-js';
import { ShieldAlert, X } from 'lucide-solid';
import { NodeView } from '../../ui/NodeView';
import type { UiRequest } from '../../ui/types';

type ApprovalOption = { optionId?: string; label?: string; kind?: string };

const str = (v: unknown): string | undefined => (typeof v === 'string' ? v : undefined);

/** The `kind` → button look (+ where it sits in the row): protocol order is kept. */
function optionTone(kind: string | undefined): string {
  if (kind === 'allow_once') return 'primary';
  if (kind === 'reject_once') return 'danger';
  if (kind === 'reject_always') return 'danger ghost';
  return 'ghost';
}

/** An approval option that cannot be answered without telling the agent why. */
const needsNote = (option: ApprovalOption): boolean => option.optionId === 'deny_note';

export function ApprovalPanel(props: { dialog: UiRequest; onAnswer: (value: unknown) => void }) {
  const d = () => props.dialog;
  const options = (): ApprovalOption[] => (Array.isArray(d().options) ? (d().options as ApprovalOption[]) : []);
  const [note, setNote] = createSignal('');
  const [wantNote, setWantNote] = createSignal(false);
  let noteEl: HTMLTextAreaElement | undefined;

  const hasNote = () => note().trim().length > 0;
  const summary = () => str(d().summary) ?? str(d().title) ?? 'Allow this action?';
  const origin = () => str(d().toolName) ?? str(d().extensionId) ?? 'Unipi';
  const placeholder = () => str((d().feedback as { placeholder?: unknown } | undefined)?.placeholder) ?? 'Tell the agent what to do instead (optional)';

  const pick = (option: ApprovalOption) => {
    if (needsNote(option) && !hasNote() && !wantNote()) {
      setWantNote(true);
      queueMicrotask(() => noteEl?.focus());
      return;
    }
    const value: { optionId?: string; feedback?: string } = { ...(option.optionId ? { optionId: option.optionId } : {}) };
    if (hasNote()) value.feedback = note().trim();
    props.onAnswer(value);
  };

  return (
    <div class="dlg dlg-approval" data-testid="approval-panel">
      <div class="dlg-head">
        <ShieldAlert size={16} class="dlg-head-icon" />
        <span class="dlg-eyebrow">Approval</span>
        <span class="ap-origin" data-testid="approval-origin">
          {origin()}
        </span>
        <span class="flex-1" />
        <button type="button" class="dlg-x" aria-label="Deny and close" onClick={() => props.onAnswer(null)}>
          <X size={18} />
        </button>
      </div>
      <p class="dlg-question" data-testid="approval-summary">
        {summary()}
      </p>
      <Show when={d().detail}>
        {(detail) => (
          <div class="ap-detail" data-testid="approval-detail">
            <NodeView node={detail() as never} path={['detail']} mode="auto" />
          </div>
        )}
      </Show>
      <Show when={wantNote() || hasNote()}>
        <textarea
          ref={noteEl}
          class="dlg-text ap-note"
          rows={2}
          placeholder={placeholder()}
          aria-label="Note to the agent"
          value={note()}
          onInput={(e) => setNote(e.currentTarget.value)}
        />
      </Show>
      <div class="ap-options">
        <Index each={options()}>
          {(option, i) => (
            <button type="button" class={`dlg-btn ap-btn ${optionTone(option().kind)}`} onClick={() => pick(option())}>
              <Show when={options().length > 1}>
                <span class="ap-num">{i + 1}</span>
              </Show>
              {option().label ?? option().optionId ?? 'Allow'}
            </button>
          )}
        </Index>
        <Show when={!options().length}>
          {/* No options from the extension: the pi default (first answer wins) is deny. */}
          <button type="button" class="dlg-btn ap-btn danger" onClick={() => props.onAnswer(null)}>
            Deny
          </button>
        </Show>
      </div>
      <Show when={wantNote()}>
        <p class="ap-hint">Add a note so the agent knows what to do instead — or tap Deny again to deny without one.</p>
      </Show>
    </div>
  );
}
