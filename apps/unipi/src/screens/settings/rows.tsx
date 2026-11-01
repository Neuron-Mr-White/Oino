/**
 * Shared Settings primitives (WP-A24): the section card, the icon row, the
 * stepper and the segmented control every Settings section uses (moved out of
 * the old one-file Settings screen; DESIGN.md §3.10 settings group).
 */
import { For, Show, createContext, useContext, type JSX } from 'solid-js';
import { Info, Minus, Plus } from 'lucide-solid';

/** Which section the two-column layout shows (undefined = all, phone). */
export const SectionNav = createContext<{ active: () => string | undefined }>();

/** Section wrapper: a titled card in the settings list (DESIGN.md §3.10).
 *  `id` anchors search jumps (`sec-<section id>`). */
export function Section(props: { title: string; icon: JSX.Element; children: JSX.Element; id?: string; navTitle?: string }) {
  const nav = useContext(SectionNav);
  // `navTitle`: the nav section this card belongs to when its own title differs
  // (Sharing › "This computer", Notifications › "What this PC sends").
  const hide = () => !!nav?.active() && nav.active() !== (props.navTitle ?? props.title);
  return (
    <section class="mb-7" id={props.id} data-section={props.title} hidden={hide() ? true : undefined}>
      <h2 class="section-label settings-section-label">
        {props.icon}
        {props.title}
      </h2>
      <div class="settings-group rounded-card bg-surface-1">{props.children}</div>
    </section>
  );
}

const rowIcons: Record<string, typeof Info> = {
  'Font size': Info, 'Line height': Info, 'Cursor style': Info, 'Cursor blink': Info,
  'Scrollback': Info, 'Phone-size policy': Info, 'Keep screen on': Info, 'Reopen last session': Info,
  'Haptics': Info, 'Composer autocorrect': Info, 'Toolbar': Info,
  'Relay': Info, 'Path preference': Info, 'Default project': Info, 'Show backlog': Info,
  'Version': Info, 'Platform': Info, 'Protocol': Info, 'Licences': Info,
};

export function Row(props: { label: string; description?: string; children?: JSX.Element; icon?: JSX.Element }) {
  const Icon = rowIcons[props.label] || Info;
  return (
    <div class="settings-row">
      <span class="settings-icon">{props.icon ?? <Icon size={18} />}</span>
      <div class="min-w-0">
        <p class="text-[15px] text-text">{props.label}</p>
        <Show when={props.description}>
          <p class="text-xs text-text-dim mt-0.5">{props.description}</p>
        </Show>
      </div>
      <div class="shrink-0 text-right text-text-muted">{props.children}</div>
    </div>
  );
}

export function Stepper(props: {
  value: number;
  min: number;
  max: number;
  step: number;
  label: string;
  suffix?: string;
  format?: (v: number) => string;
  onChange: (v: number) => void;
}) {
  const fmt = props.format ?? ((v: number) => String(v));
  const dec = () => props.onChange(Math.max(props.min, round(props.value - props.step)));
  const inc = () => props.onChange(Math.min(props.max, round(props.value + props.step)));
  function round(v: number) {
    // Avoid float drift (e.g. 1.2 - 0.1 = 1.0999999...).
    return Math.round(v * 100) / 100;
  }
  return (
    <div class="settings-stepper" role="group" aria-label={props.label}>
      <button type="button" class="rounded-input bg-surface-2 w-8 h-8 flex items-center justify-center" onClick={dec} aria-label={`Decrease ${props.label}`}>
        <Minus size={14} />
      </button>
      <span class="w-12 text-center text-sm tabular-nums">{fmt(props.value)}{props.suffix}</span>
      <button type="button" class="rounded-input bg-surface-2 w-8 h-8 flex items-center justify-center" onClick={inc} aria-label={`Increase ${props.label}`}>
        <Plus size={14} />
      </button>
    </div>
  );
}

export function Segmented<T extends string>(props: { value: T; options: readonly T[]; labels?: Partial<Record<T, string>>; onChange: (v: T) => void; label: string }) {
  return (
    <div role="radiogroup" aria-label={props.label} class="flex rounded-pill bg-surface-3 p-1 gap-1">
      <For each={props.options}>
        {(opt) => (
          <button
            type="button"
            role="radio"
            aria-checked={props.value === opt}
            class={`px-3 py-1 text-xs rounded-pill transition-colors ${props.value === opt ? 'bg-surface-2 text-text' : 'text-text-muted'}`}
            onClick={() => props.onChange(opt)}
          >
            {props.labels?.[opt] ?? opt}
          </button>
        )}
      </For>
    </div>
  );
}
