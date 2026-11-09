import { createSignal, For, Show, type JSX } from 'solid-js';
import { ChevronDown } from 'lucide-solid';
import { Switch as PhoneSwitch } from '../phone';
import { SwitchField } from '../Switch';
import { layout } from '../../lib/layout';

/** A section heading with a quiet count. */
export function SectionHead(props: { title: string; count?: number; aside?: JSX.Element }) {
  return <div class="sa-sec-head"><h2>{props.title}<Show when={props.count !== undefined}><span>{props.count}</span></Show></h2>{props.aside}</div>;
}

/** Shows the first `limit` rows; "Show N more" reveals the rest. */
export function Limited<T>(props: { items: readonly T[]; limit: number; expanded?: { get: () => boolean; set: (value: boolean) => void }; children: (item: T, index: () => number) => JSX.Element }) {
  const [local, setLocal] = createSignal(false);
  const all = () => props.expanded ? props.expanded.get() : local();
  const setAll = (value: boolean) => props.expanded ? props.expanded.set(value) : setLocal(value);
  const shown = () => (all() ? props.items : props.items.slice(0, props.limit));
  const hidden = () => props.items.length - props.limit;
  return <>
    <For each={shown()}>{(item, i) => props.children(item, i)}</For>
    <Show when={hidden() > 0}><button type="button" class="sa-more" aria-expanded={all()} onClick={() => setAll(!all())}>{all() ? 'Show less' : `Show ${hidden()} more`}</button></Show>
  </>;
}

/** Native select with the app's chip look: a real menu on every platform, 44 px on the phone. */
export function ChipSelect(props: { value: string; label: string; options: ReadonlyArray<{ value: string; label: string }>; disabled?: boolean; onChange: (value: string) => void; class?: string }) {
  return <span class={`sa-select ${props.class ?? ''}`} data-disabled={props.disabled ? '' : undefined}>
    <select aria-label={props.label} value={props.value} disabled={props.disabled} onChange={(e) => { const el = e.currentTarget; props.onChange(el.value); /* Controlled: keep showing the host's value until it changes. */ queueMicrotask(() => { el.value = props.value; }); }}>
      <For each={props.options}>{(o) => <option value={o.value} selected={o.value === props.value}>{o.label}</option>}</For>
    </select>
    <ChevronDown size={14} aria-hidden="true" />
  </span>;
}

/** The app switch on desktop; the 48 px phone switch on the phone shell. */
export function Toggle(props: { checked: boolean; label: string; disabled?: boolean; onChange: (v: boolean) => void }) {
  return <Show when={layout().desktop()} fallback={<PhoneSwitch checked={props.checked} label={props.label} disabled={props.disabled} onChange={props.onChange} />}>
    <SwitchField checked={props.checked} label={props.label} disabled={props.disabled} onChange={props.onChange} />
  </Show>;
}
