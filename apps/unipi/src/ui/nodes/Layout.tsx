/**
 * `Layout` (ui-protocol §4.3, §13.3). Horizontal layouts always solve their
 * (definite) width; vertical ones solve their height in definite mode and are
 * a flex column in auto mode (`Length` rows fixed, the rest natural). The box
 * is measured by the shared ResizeObserver; until the first measurement a CSS
 * flex approximation of the same constraints renders, so there is no flash.
 * Horizontal layouts narrower than `collapse` columns (default 60) stack
 * vertically in auto mode (§13.6 rule 1).
 */
import { For, createMemo, createSignal, onCleanup, onMount, type JSX } from 'solid-js';
import { NodeView } from '../NodeView';
import { colPx, observeSize, rowPx } from '../layout/units';
import { marginOf, normConstraint, solveAxis, type Segment } from '../layout/solve';
import type { Constraint, Flex, LayoutNode, UiNode } from '../types';
import { baseStyle, childPath, cx, itemKey, type Mode, type NodeProps } from './props';

const JUSTIFY: Record<Flex, string> = {
  Legacy: 'flex-start',
  Start: 'flex-start',
  End: 'flex-end',
  Center: 'center',
  SpaceBetween: 'space-between',
  SpaceAround: 'space-around',
};

/** CSS flex approximation of one constraint along the main axis (pre-measure / jsdom). */
function flexCss(c: Constraint | undefined, unit: string, horizontal: boolean): JSX.CSSProperties {
  const n = normConstraint(c);
  const minProp = horizontal ? 'min-width' : 'min-height';
  const maxProp = horizontal ? 'max-width' : 'max-height';
  switch (n.k) {
    case 'Length':
      return { flex: `0 0 calc(${n.n} * ${unit})` };
    case 'Percentage':
      return { flex: `0 0 ${n.n}%` };
    case 'Ratio':
      return { flex: `0 0 ${((n.a / Math.max(n.b, 1)) * 100).toFixed(4)}%` };
    case 'Min':
      return { flex: '1 1 0', [minProp]: `calc(${n.n} * ${unit})` };
    case 'Max':
      return { flex: `0 1 calc(${n.n} * ${unit})`, [maxProp]: `calc(${n.n} * ${unit})` };
    case 'Fill':
      return { flex: `${Math.max(n.n, 1e-6)} 1 0` };
  }
}

export function Layout(props: NodeProps<LayoutNode>) {
  let el!: HTMLDivElement;
  const [box, setBox] = createSignal<{ width: number; height: number } | undefined>();
  onMount(() => {
    const stop = observeSize(el, setBox);
    onCleanup(stop);
  });

  const horizontal = () => props.node.direction === 'Horizontal';
  const collapse = () => props.node.collapse ?? 60;
  /** Horizontal but too narrow: stack vertically in auto mode. */
  const collapsed = () => {
    const w = box()?.width;
    return horizontal() && collapse() > 0 && w !== undefined && w > 0 && w < collapse() * colPx();
  };
  const axisH = () => horizontal() && !collapsed();
  const children = () => (Array.isArray(props.node.children) ? props.node.children : []);
  const constraintAt = (i: number) => props.node.constraints?.[i];
  const spacing = () => props.node.spacing ?? 0;
  const margin = () => marginOf(props.node.margin);
  const childMode = (i: number): Mode => {
    if (axisH()) return props.mode;
    if (collapsed()) return 'auto';
    if (props.mode === 'definite') return 'definite';
    return normConstraint(constraintAt(i)).k === 'Length' ? 'definite' : 'auto';
  };

  /** Solved segments (px) when the main axis is measurable. */
  const segments = createMemo<Segment[] | undefined>(() => {
    const b = box();
    if (!b || collapsed()) return undefined;
    const spec = { constraints: children().map((_, i) => constraintAt(i)), flex: props.node.flex ?? 'Start', spacing: spacing() };
    if (axisH()) return b.width > 0 ? solveAxis({ ...spec, scale: colPx() }, b.width) : undefined;
    if (props.mode === 'definite') return b.height > 0 ? solveAxis({ ...spec, scale: rowPx() }, b.height) : undefined;
    return undefined;
  });

  const itemStyle = (i: number): JSX.CSSProperties => {
    const segs = segments();
    const prop = axisH() ? 'width' : 'height';
    if (segs && segs[i]) {
      const [start, size] = segs[i]!;
      const prevEnd = i === 0 ? 0 : segs[i - 1]![0] + segs[i - 1]![1];
      const margin = axisH() ? 'margin-left' : 'margin-top';
      return { flex: '0 0 auto', [prop]: `${size}px`, [margin]: `${start - prevEnd}px` };
    }
    if (collapsed()) return {};
    if (!axisH() && props.mode === 'auto') {
      const n = normConstraint(constraintAt(i));
      return n.k === 'Length' ? { flex: '0 0 auto', height: `calc(${n.n} * var(--ui-row))` } : { flex: '0 0 auto' };
    }
    return flexCss(constraintAt(i), axisH() ? 'var(--ui-col)' : 'var(--ui-row)', axisH());
  };

  const rootStyle = (): JSX.CSSProperties => {
    const m = margin();
    const solved = !!segments();
    const gapUnit = axisH() ? 'var(--ui-col)' : 'var(--ui-row)';
    return {
      ...baseStyle(props.node).style,
      'flex-direction': axisH() ? 'row' : 'column',
      padding: m.horizontal || m.vertical ? `calc(${m.vertical} * var(--ui-row)) calc(${m.horizontal} * var(--ui-col))` : undefined,
      gap: solved || !spacing() ? undefined : `calc(${spacing()} * ${gapUnit})`,
      'justify-content': solved ? undefined : JUSTIFY[props.node.flex ?? 'Start'],
    };
  };

  return (
    <div
      ref={el}
      class={cx('ui-layout', baseStyle(props.node).class)}
      data-dir={axisH() ? 'h' : 'v'}
      data-collapsed={collapsed() ? '' : undefined}
      data-mode={props.mode}
      style={rootStyle()}
    >
      <For each={children()}>
        {(child: UiNode, i) => (
          <div class="ui-layout-item" data-mode={childMode(i())} style={itemStyle(i())}>
            <NodeView node={child} path={childPath(props.path, itemKey(child, i()))} mode={childMode(i())} />
          </div>
        )}
      </For>
    </div>
  );
}
