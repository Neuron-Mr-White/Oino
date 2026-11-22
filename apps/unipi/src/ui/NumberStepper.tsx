/**
 * THE number control — one stepper everywhere (settings form, the Subagents
 * "max running", Appearance / Terminal / Host settings, session Limits): a
 * 32 px box with − [value unit] +, the value typed or nudged (↑/↓, buttons).
 * Pure view over a value; `undefined` is "unset" (placeholder shown).
 */
import { Show, createSignal } from 'solid-js';
import { Minus, Plus } from 'lucide-solid';
import './number-stepper.css';

export type NumberStepperProps = {
  value: number | undefined;
  /** Called with the clamped number, or `undefined` when the box is emptied. */
  onChange: (value: number | undefined) => void;
  label: string;
  min?: number;
  max?: number;
  step?: number;
  integer?: boolean;
  unit?: string;
  /** Shown while unset; also the base a first nudge starts from. */
  fallback?: number;
  placeholder?: string;
  disabled?: boolean;
  /** Display text of a committed value (`1.0`); typing still shows the raw draft. */
  format?: (value: number) => string;
  testId?: string;
  inputId?: string;
  /** Emptying the box is not "unset" (it reverts to the value). */
  required?: boolean;
};

export function NumberStepper(props: NumberStepperProps) {
  const step = () => props.step ?? 1;
  const min = () => props.min ?? Number.NEGATIVE_INFINITY;
  const max = () => props.max ?? Number.POSITIVE_INFINITY;
  const clamp = (v: number) => {
    const rounded = Math.round(v * 100) / 100;
    return Math.min(max(), Math.max(min(), props.integer ? Math.round(rounded) : rounded));
  };
  const base = () => props.value ?? props.fallback ?? (Number.isFinite(min()) ? min() : 0);
  const [draft, setDraft] = createSignal<string | undefined>();
  const shown = () => draft() ?? (props.value === undefined ? '' : props.format ? props.format(props.value) : String(props.value));
  const commit = (text: string) => {
    setDraft(undefined);
    const trimmed = text.trim();
    if (trimmed === '') {
      if (!props.required && props.value !== undefined) props.onChange(undefined);
      return;
    }
    const parsed = Number(trimmed);
    if (Number.isFinite(parsed)) props.onChange(clamp(parsed));
  };
  const nudge = (dir: 1 | -1) => {
    setDraft(undefined);
    props.onChange(clamp(base() + dir * step()));
  };
  return (
    <div class="settings-number" role="group" aria-label={props.label} data-disabled={props.disabled ? '' : undefined}>
      <button
        type="button"
        class="settings-number-step"
        aria-label={`Decrease ${props.label}`}
        tabindex={-1}
        disabled={props.disabled || (props.value !== undefined && props.value <= min())}
        onClick={() => nudge(-1)}
      >
        <Minus size={14} />
      </button>
      <span class="settings-number-box">
        <input
          id={props.inputId}
          type="text"
          inputmode={props.integer ? 'numeric' : 'decimal'}
          class="settings-number-input"
          aria-label={`${props.label} value`}
          role="spinbutton"
          aria-valuenow={props.value}
          aria-valuemin={props.min}
          aria-valuemax={props.max}
          autocomplete="off"
          spellcheck={false}
          disabled={props.disabled}
          placeholder={props.placeholder ?? (props.fallback !== undefined ? String(props.fallback) : 'default')}
          value={shown()}
          data-testid={props.testId}
          onInput={(e) => setDraft(e.currentTarget.value)}
          onBlur={(e) => draft() !== undefined && commit(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === 'ArrowUp') { e.preventDefault(); nudge(1); }
            else if (e.key === 'ArrowDown') { e.preventDefault(); nudge(-1); }
            else if (e.key === 'Enter') commit(e.currentTarget.value);
            else if (e.key === 'Escape') setDraft(undefined);
          }}
        />
        <Show when={props.unit}>
          <span class="settings-unit">{props.unit}</span>
        </Show>
      </span>
      <button
        type="button"
        class="settings-number-step"
        aria-label={`Increase ${props.label}`}
        tabindex={-1}
        disabled={props.disabled || (props.value !== undefined && props.value >= max())}
        onClick={() => nudge(1)}
      >
        <Plus size={14} />
      </button>
    </div>
  );
}
