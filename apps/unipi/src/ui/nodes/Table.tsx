/**
 * `Table` (ui-protocol §4.5.1): a `<table>`-free grid whose
 * `grid-template-columns` come from the horizontal solve of `widths`
 * (columnSpacing = gap); sticky header; rows 32 / 44 px × `row.height`;
 * selection like `List` (local, `state.selected` wins on change); virtualized
 * above 100 rows. Phone (§13.6 rule 3): horizontal scroll, first column
 * sticky, no column hiding.
 */
import { For, Show, createMemo, createSignal, onCleanup, onMount, type JSX } from 'solid-js';
import { useEntry } from '../actions';
import { useLocal } from '../localState';
import { colPx, observeSize, shell } from '../layout/units';
import { normConstraint, solveAxis } from '../layout/solve';
import { styleCss } from '../theme';
import type { Cell, CellObj, Constraint, Row, TableNode, Text } from '../types';
import { VIRTUAL_THRESHOLD, VirtualWindow, useVirtual } from '../virtual';
import { navIndex } from './List';
import { TextView } from './Text';
import { baseStyle, cx, itemKey, type NodeProps } from './props';

const cellObj = (c: Cell): CellObj =>
  c && typeof c === 'object' && !Array.isArray(c) && 'content' in c ? (c as CellObj) : { content: c as Text };

function cssTrack(c: Constraint | undefined): string {
  const n = normConstraint(c);
  switch (n.k) {
    case 'Length':
      return `calc(${n.n} * var(--ui-col))`;
    case 'Percentage':
      return `${n.n}%`;
    case 'Ratio':
      return `${((n.a / Math.max(n.b, 1)) * 100).toFixed(4)}%`;
    case 'Min':
      return `minmax(calc(${n.n} * var(--ui-col)), 1fr)`;
    case 'Max':
      return `minmax(0, calc(${n.n} * var(--ui-col)))`;
    case 'Fill':
      return `${Math.max(n.n, 0.0001)}fr`;
  }
}

/** Minimum columns a constraint needs before the phone table scrolls sideways. */
function minCols(c: Constraint | undefined): number {
  const n = normConstraint(c);
  if (n.k === 'Length' || n.k === 'Min' || n.k === 'Max') return n.n;
  return 12;
}

function RowCells(props: { row: Row | undefined; header?: boolean }) {
  return (
    <For each={props.row?.cells ?? []}>
      {(c) => {
        const cell = () => cellObj(c);
        const css = () => styleCss(cell().style);
        return (
          <div role={props.header ? 'columnheader' : 'gridcell'} class={cx('ui-cell', css().class)} style={css().style}>
            <TextView text={cell().content} />
          </div>
        );
      }}
    </For>
  );
}

export function Table(props: NodeProps<TableNode>) {
  const entry = useEntry();
  let el!: HTMLDivElement;
  const [width, setWidth] = createSignal(0);
  onMount(() => onCleanup(observeSize(el, (s) => setWidth(s.width))));

  const rows = createMemo(() => (Array.isArray(props.node.rows) ? props.node.rows : []).map((r, i) => ({ key: itemKey(r, i), row: r })));
  const columns = () => {
    const n = Math.max(props.node.widths?.length ?? 0, props.node.header?.cells.length ?? 0, ...rows().map((r) => r.row.cells?.length ?? 0));
    return Array.from({ length: n }, (_, i) => props.node.widths?.[i]);
  };
  const spacing = () => props.node.columnSpacing ?? 1;
  const phone = () => shell() === 'phone';
  const minWidthCols = () => columns().reduce<number>((sum, c) => sum + minCols(c), 0) + spacing() * Math.max(0, columns().length - 1);
  const template = createMemo(() => {
    const cols = columns();
    const w = width();
    const minPx = minWidthCols() * colPx();
    const solveW = phone() ? Math.max(w, minPx) : w;
    if (solveW > 0) {
      const segs = solveAxis({ constraints: cols, spacing: spacing(), scale: colPx() }, solveW);
      return segs.map(([, size]) => `${size}px`).join(' ');
    }
    return cols.map(cssTrack).join(' ');
  });
  const gridStyle = (): JSX.CSSProperties => ({
    'grid-template-columns': template(),
    'column-gap': `calc(${spacing()} * var(--ui-col))`,
  });

  const [selected, setSelected] = useLocal<string | undefined>(entry.entryId, props.path, 'selected', {
    prop: () => props.node.state?.selected,
    init: () => undefined,
  });
  const select = (k: string) => {
    if (selected() === k) return;
    setSelected(k);
    entry.send(props.node.onSelect, k, props.path);
  };
  const activate = (k: string) => entry.send(props.node.onActivate, k, props.path);
  const onRowClick = (k: string) => {
    if (props.node.onActivate && !props.node.onSelect) {
      setSelected(k);
      activate(k);
      return;
    }
    select(k);
    if (phone()) activate(k);
  };

  const [scrollEl, setScrollEl] = createSignal<HTMLElement>();
  const virtual = () => rows().length > VIRTUAL_THRESHOLD;
  const rowPxFor = (r: Row | undefined) => {
    const h = Math.max(1, r?.height ?? 1);
    const base = phone() ? 44 : 32;
    return h === 1 ? base : base + (h - 1) * 20;
  };
  const virt = useVirtual({
    count: () => (virtual() ? rows().length : 0),
    scroller: scrollEl,
    estimate: (i) => rowPxFor(rows()[i]?.row) + (rows()[i]?.row.bottomMargin ?? 0) * 20,
    key: (i) => rows()[i]?.key ?? i,
    fixed: true,
  });

  const onKey = (e: KeyboardEvent) => {
    const list = rows();
    const cur = list.findIndex((r) => r.key === selected());
    const next = navIndex(e.key, cur, list.length);
    if (next !== undefined) {
      e.preventDefault();
      const k = list[next]!.key;
      select(k);
      if (virtual()) virt.scrollToIndex(next, { align: 'auto' });
      else el.querySelector<HTMLElement>(`[data-index="${next}"]`)?.scrollIntoView?.({ block: 'nearest' });
    } else if (e.key === 'Enter' && selected() !== undefined) {
      e.preventDefault();
      activate(selected()!);
    }
  };

  const hl = () => styleCss(props.node.rowHighlightStyle);
  const symbol = () => props.node.highlightSymbol;
  const showSymbol = () => !!symbol() && ((props.node.highlightSpacing ?? 'WhenSelected') === 'Always' || ((props.node.highlightSpacing ?? 'WhenSelected') === 'WhenSelected' && selected() !== undefined));

  const BodyRow = (p: { index: number }) => {
    const r = () => rows()[p.index];
    const sel = () => r()?.key === selected();
    const css = () => styleCss(r()?.row.style);
    return (
      <div
        role="row"
        class={cx('ui-trow', css().class, sel() && hl().class)}
        data-index={p.index}
        data-selected={sel() ? '' : undefined}
        aria-selected={sel()}
        style={{
          ...gridStyle(),
          ...css().style,
          ...(sel() ? hl().style : {}),
          'min-height': `${rowPxFor(r()?.row)}px`,
          'margin-bottom': r()?.row.bottomMargin ? `calc(${r()!.row.bottomMargin} * var(--ui-row))` : undefined,
        }}
        onClick={() => r() && onRowClick(r()!.key)}
        onDblClick={() => r() && props.node.onSelect && activate(r()!.key)}
      >
        <Show when={showSymbol()}>
          <span class="ui-row-symbol ui-trow-symbol" aria-hidden="true">
            {sel() ? symbol() : ''}
          </span>
        </Show>
        <RowCells row={r()?.row} />
      </div>
    );
  };

  return (
    <div
      ref={el}
      class={cx('ui-table', baseStyle(props.node).class)}
      data-mode={props.mode}
      data-sticky={props.node.sticky === 'FirstColumn' || phone() ? '' : undefined}
      data-scrollbar={props.node.scrollbar ? '' : undefined}
      style={baseStyle(props.node).style}
    >
      <div
        ref={setScrollEl}
        class="ui-table-scroll"
        role="grid"
        tabindex="0"
        aria-rowcount={rows().length}
        onKeyDown={onKey}
        style={{ '--ui-table-min': phone() ? `calc(${minWidthCols()} * var(--ui-col))` : undefined } as JSX.CSSProperties}
      >
        <div class="ui-table-inner">
          <Show when={props.node.header}>
            <div role="row" class={cx('ui-trow ui-thead', styleCss(props.node.header?.style).class)} style={{ ...gridStyle(), ...styleCss(props.node.header?.style).style }}>
              <RowCells row={props.node.header} header />
            </div>
          </Show>
          <Show when={virtual()} fallback={<For each={rows()}>{(_, i) => <BodyRow index={i()} />}</For>}>
            <VirtualWindow virt={virt} fixed>
              {(i) => <BodyRow index={i} />}
            </VirtualWindow>
          </Show>
          <Show when={props.node.footer}>
            <div role="row" class={cx('ui-trow ui-tfoot', styleCss(props.node.footer?.style).class)} style={{ ...gridStyle(), ...styleCss(props.node.footer?.style).style }}>
              <RowCells row={props.node.footer} />
            </div>
          </Show>
        </div>
      </div>
    </div>
  );
}
