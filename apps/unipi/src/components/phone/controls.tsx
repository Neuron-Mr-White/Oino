/**
 * Phone controls (phone spec §2.4, §2.12): 48 px hit targets everywhere.
 * They render on the desktop shell too (same markup, the phone.css rules are
 * scoped to `html[data-shell=phone]`, the base rules are neutral).
 */
import { createSignal, For, onCleanup, onMount, Show, type Component, type JSX } from 'solid-js';
import { Loader2, Minus, Plus, Search, X } from 'lucide-solid';
import { haptic } from './services';

type Icon = Component<{ size?: number; strokeWidth?: number; class?: string }>;

export function IconButton(p: {
  icon: Icon;
  label: string;
  onClick: (e: MouseEvent) => void;
  badge?: number | boolean;
  pressed?: boolean;
  tone?: 'default' | 'accent' | 'danger';
  disabled?: boolean;
  testId?: string;
}) {
  const badgeText = () => (typeof p.badge === 'number' ? (p.badge > 9 ? '9+' : String(p.badge)) : '');
  return (
    <button
      type="button"
      class="ph-icon-btn"
      data-tone={p.tone && p.tone !== 'default' ? p.tone : undefined}
      aria-label={p.label}
      aria-pressed={p.pressed === undefined ? undefined : p.pressed}
      disabled={p.disabled}
      data-testid={p.testId}
      onClick={(e) => p.onClick(e)}
    >
      <p.icon size={24} strokeWidth={1.75} />
      <Show when={p.badge !== undefined && p.badge !== false && p.badge !== 0}>
        <span class="ph-badge" data-dot={typeof p.badge === 'boolean' ? '' : undefined} aria-hidden="true">
          {badgeText()}
        </span>
      </Show>
    </button>
  );
}

export type FabProps = { icon: Icon; label: string; onClick: () => void; extended?: boolean };

export function Fab(p: FabProps) {
  return (
    <button
      type="button"
      class="ph-fab"
      data-extended={p.extended !== false ? '' : undefined}
      aria-label={p.label}
      data-testid="ph-fab"
      onClick={() => p.onClick()}
    >
      <p.icon size={24} strokeWidth={2} />
      <span class="ph-fab-label">{p.label}</span>
    </button>
  );
}

export function Button(p: {
  kind?: 'primary' | 'secondary' | 'ghost' | 'danger';
  block?: boolean;
  disabled?: boolean;
  loading?: boolean;
  onClick: () => void;
  children: JSX.Element;
  testId?: string;
}) {
  return (
    <button
      type="button"
      class="ph-btn"
      data-kind={p.kind ?? 'secondary'}
      data-block={p.block ? '' : undefined}
      disabled={p.disabled || p.loading}
      aria-busy={p.loading ? true : undefined}
      data-testid={p.testId}
      onClick={() => p.onClick()}
    >
      <Show when={p.loading}>
        <Loader2 size={18} class="ph-spin" />
      </Show>
      {p.children}
    </button>
  );
}

export function Segmented<T extends string>(p: {
  value: T;
  options: readonly { value: T; label: string }[];
  onChange: (v: T) => void;
  label: string;
}) {
  return (
    <div class="ph-segmented" role="radiogroup" aria-label={p.label}>
      <For each={p.options}>
        {(opt) => (
          <button
            type="button"
            role="radio"
            class="ph-seg"
            aria-checked={p.value === opt.value}
            onClick={() => {
              if (p.value === opt.value) return;
              haptic('selection');
              p.onChange(opt.value);
            }}
          >
            {opt.label}
          </button>
        )}
      </For>
    </div>
  );
}

export function FilterChips<T extends string>(p: {
  value: T[] | T;
  options: readonly { value: T; label: string; count?: number }[];
  multi?: boolean;
  onChange: (v: any) => void;
  label: string;
}) {
  const isOn = (v: T) => (Array.isArray(p.value) ? p.value.includes(v) : p.value === v);
  const toggle = (v: T) => {
    haptic('selection');
    if (p.multi) {
      const cur = Array.isArray(p.value) ? p.value : [p.value];
      p.onChange(cur.includes(v) ? cur.filter((x) => x !== v) : [...cur, v]);
    } else p.onChange(v);
  };
  let scroller: HTMLDivElement | undefined;
  const [edge, setEdge] = createSignal({ start: false, end: false });
  const measure = () => {
    if (!scroller) return;
    const { scrollLeft, scrollWidth, clientWidth } = scroller;
    setEdge({ start: scrollLeft > 2, end: scrollLeft + clientWidth < scrollWidth - 2 });
  };
  onMount(() => {
    measure();
    if (typeof ResizeObserver !== 'undefined' && scroller) {
      const ro = new ResizeObserver(measure);
      ro.observe(scroller);
      onCleanup(() => ro.disconnect());
    }
  });
  return (
    <div
      ref={scroller}
      class="ph-chips"
      role={p.multi ? 'group' : 'radiogroup'}
      aria-label={p.label}
      data-fade-start={edge().start ? '' : undefined}
      data-fade-end={edge().end ? '' : undefined}
      onScroll={measure}
    >
      <For each={p.options}>
        {(opt) => (
          <button
            type="button"
            class="ph-chip"
            role={p.multi ? 'checkbox' : 'radio'}
            aria-checked={isOn(opt.value)}
            onClick={() => toggle(opt.value)}
          >
            <span>{opt.label}</span>
            <Show when={opt.count !== undefined}>
              <span class="ph-chip-count">{opt.count}</span>
            </Show>
          </button>
        )}
      </For>
    </div>
  );
}

export function Switch(p: { checked: boolean; onChange: (v: boolean) => void; label: string; disabled?: boolean; testId?: string }) {
  return (
    <button
      type="button"
      role="switch"
      class="ph-switch"
      aria-checked={p.checked}
      aria-label={p.label}
      disabled={p.disabled}
      data-testid={p.testId}
      onClick={(e) => {
        e.stopPropagation();
        haptic('selection');
        p.onChange(!p.checked);
      }}
    >
      <span class="ph-switch-track">
        <span class="ph-switch-thumb" />
      </span>
    </button>
  );
}

export function Stepper(p: {
  value: number;
  min: number;
  max: number;
  step?: number;
  format?: (n: number) => string;
  onChange: (n: number) => void;
  label: string;
}) {
  const step = () => p.step ?? 1;
  const clamp = (n: number) => Math.min(p.max, Math.max(p.min, Math.round(n / step()) * step()));
  const go = (dir: 1 | -1) => {
    const next = clamp(p.value + dir * step());
    if (next !== p.value) {
      haptic('selection');
      p.onChange(next);
    }
  };
  return (
    <div class="ph-stepper" role="group" aria-label={p.label}>
      <button type="button" class="ph-step-btn" aria-label={`Decrease ${p.label}`} disabled={p.value <= p.min} onClick={() => go(-1)}>
        <Minus size={20} />
      </button>
      <output class="ph-step-value" aria-live="polite">
        {p.format ? p.format(p.value) : p.value}
      </output>
      <button type="button" class="ph-step-btn" aria-label={`Increase ${p.label}`} disabled={p.value >= p.max} onClick={() => go(1)}>
        <Plus size={20} />
      </button>
    </div>
  );
}

export function SearchField(p: { value: string; onInput: (q: string) => void; placeholder: string; autofocus?: boolean }) {
  let input: HTMLInputElement | undefined;
  return (
    <div class="ph-search search-field" role="search">
      <Search size={18} class="ph-search-icon" aria-hidden="true" />
      <input
        ref={input}
        type="search"
        enterkeyhint="search"
        autocomplete="off"
        autocapitalize="off"
        spellcheck={false}
        placeholder={p.placeholder}
        aria-label={p.placeholder}
        value={p.value}
        autofocus={p.autofocus}
        onInput={(e) => p.onInput(e.currentTarget.value)}
      />
      <Show when={p.value}>
        <button
          type="button"
          class="ph-search-clear"
          aria-label="Clear search"
          onClick={() => {
            p.onInput('');
            input?.focus();
          }}
        >
          <X size={18} />
        </button>
      </Show>
    </div>
  );
}

