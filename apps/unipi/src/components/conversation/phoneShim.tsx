/**
 * TEMPORARY shim of the phone primitives (phone spec §2.12) that Package B (chat)
 * uses, built on today's `BoardSheet`. Package A ships the real ones in
 * `components/phone`; once they land this file is deleted and the imports point
 * at `../phone`. Same signatures as the spec, nothing more.
 */
import { createSignal, For, Show, type Component, type JSX } from 'solid-js';
import { Check } from 'lucide-solid';
import { BoardSheet } from '../board/BoardSheet';
import { hapticTick } from '../../lib/haptics';
import './phoneChat.css';

export type Detent = 'content' | 'half' | 'full';

export function Sheet(p: {
  open: boolean;
  onClose: () => void;
  label: string;
  title?: JSX.Element | false;
  headerAction?: JSX.Element;
  detents?: Detent[];
  initialDetent?: Detent;
  footer?: JSX.Element;
  guarded?: boolean | (() => boolean);
  class?: string;
  testId?: string;
  children: JSX.Element;
}): JSX.Element {
  const full = () => (p.initialDetent ?? p.detents?.[0]) === 'full';
  return (
    <BoardSheet
      open={p.open}
      onClose={p.onClose}
      label={p.label}
      title={p.title === false ? undefined : p.title}
      headerAction={p.headerAction}
      footer={p.footer}
      guarded={typeof p.guarded === 'function' ? p.guarded() : p.guarded}
      class={`${full() ? 'ph-sheet-full ' : ''}${p.class ?? ''}`}
      testId={p.testId}
    >
      {p.children}
    </BoardSheet>
  );
}

export type ActionItem = {
  id: string;
  label: string;
  icon?: Component<{ size?: number }>;
  description?: string;
  value?: string;
  checked?: boolean;
  tone?: 'danger';
  disabled?: boolean;
  confirm?: string;
  onSelect: () => void;
};

export function ActionSheet(p: { open: boolean; onClose: () => void; title?: string; subtitle?: string; sections: ActionItem[][]; testId?: string }): JSX.Element {
  return (
    <BoardSheet open={p.open} onClose={p.onClose} label={p.title ?? 'Actions'} title={p.title} class="ph-action-sheet" testId={p.testId}>
      <Show when={p.subtitle}>
        <p class="ph-action-sub">{p.subtitle}</p>
      </Show>
      <For each={p.sections.filter((s) => s.length > 0)}>
        {(section) => (
          <div class="ph-action-section" role="group">
            <For each={section}>
              {(item) => {
                const Icon = item.icon;
                return (
                  <button
                    type="button"
                    class="ph-action-row"
                    data-tone={item.tone}
                    data-testid={`action-${item.id}`}
                    disabled={item.disabled}
                    aria-checked={item.checked === undefined ? undefined : item.checked}
                    role={item.checked === undefined ? undefined : 'menuitemradio'}
                    onClick={() => {
                      if (item.confirm && !window.confirm(item.confirm)) return;
                      p.onClose();
                      item.onSelect();
                    }}
                  >
                    <Show when={Icon}>{(I) => <span class="ph-action-icon">{(() => { const C = I(); return <C size={22} />; })()}</span>}</Show>
                    <span class="ph-action-text">
                      <span class="ph-action-label">{item.label}</span>
                      <Show when={item.description}>
                        <span class="ph-action-desc">{item.description}</span>
                      </Show>
                    </span>
                    <Show when={item.value}>
                      <span class="ph-action-value">{item.value}</span>
                    </Show>
                    <Show when={item.checked}>
                      <Check size={20} class="ph-action-check" />
                    </Show>
                  </button>
                );
              }}
            </For>
          </div>
        )}
      </For>
    </BoardSheet>
  );
}

export function IconButton(p: {
  icon: Component<{ size?: number }>;
  label: string;
  onClick: (e: MouseEvent) => void;
  badge?: number | boolean;
  pressed?: boolean;
  tone?: 'default' | 'accent' | 'danger';
  disabled?: boolean;
  testId?: string;
}): JSX.Element {
  const Icon = p.icon;
  return (
    <button type="button" class="ph-icon-btn" aria-label={p.label} aria-pressed={p.pressed} data-tone={p.tone} disabled={p.disabled} data-testid={p.testId} onClick={(e) => p.onClick(e)}>
      <Icon size={24} />
    </button>
  );
}

export function Segmented<T extends string>(p: { value: T; options: readonly { value: T; label: string }[]; onChange: (v: T) => void; label: string }): JSX.Element {
  return (
    <div class="ph-segmented" role="radiogroup" aria-label={p.label}>
      <For each={p.options}>
        {(o) => (
          <button type="button" role="radio" aria-checked={o.value === p.value} class="ph-seg" onClick={() => p.onChange(o.value)}>
            {o.label}
          </button>
        )}
      </For>
    </div>
  );
}

export function haptic(kind: 'selection' | 'impact' | 'success' | 'warning'): void {
  void hapticTick(true, kind === 'selection' ? 'tick' : 'tap');
}

const [hiddenReasons, setHiddenReasons] = createSignal<ReadonlySet<string>>(new Set());
export const tabBarHiddenReasons = hiddenReasons;
export function setTabBarHidden(reason: string, hidden: boolean): void {
  const next = new Set(hiddenReasons());
  if (hidden) next.add(reason);
  else next.delete(reason);
  setHiddenReasons(next);
}
