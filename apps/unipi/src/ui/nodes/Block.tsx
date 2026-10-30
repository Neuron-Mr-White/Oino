/**
 * `Block` (ui-protocol §4.5.1) and the `block` chrome every leaf may carry.
 * `<section class="ui-block">`: borders 1 px `--color-line` (radius 12
 * Rounded / 4 Plain, 3 px double, 2 px thick); a 20 px (`--ui-row`) title row
 * at 13/600; padding in units; `collapsible` → disclosure button (local state,
 * prop wins on change, `onToggle` → collapsed); `rail` → 3 px left bar. The
 * chrome never spends more than the TUI's cells (§4.3 chrome rule).
 */
import { For, Show, createMemo, createUniqueId, type JSX } from 'solid-js';
import { ChevronRight } from 'lucide-solid';
import { useEntry } from '../actions';
import { useLocal } from '../localState';
import { styleCss, tokenVar } from '../theme';
import type { Alignment, BlockNode, BlockProps, Line, Padding, Title } from '../types';
import { NodeView } from '../NodeView';
import { InlineLine } from './Text';
import { childPath, cx, type Mode, type NodeProps } from './props';

type TitleItem = { content: Line; alignment?: Alignment; position: 'Top' | 'Bottom' };

const isTitleObj = (v: unknown): v is { content: Line; alignment?: Alignment; position?: 'Top' | 'Bottom' } =>
  !!v &&
  typeof v === 'object' &&
  !Array.isArray(v) &&
  'content' in v &&
  ('alignment' in v || 'position' in v || typeof (v as { content: unknown }).content !== 'string');

/** `title: Line | Title[]` → positioned titles. An array is a Title list only when an item looks like one. */
export function titlesOf(title: BlockProps['title'], fallbackAlign: Alignment | undefined): TitleItem[] {
  if (title == null) return [];
  if (Array.isArray(title) && title.some(isTitleObj)) {
    return (title as Title[]).map((t) =>
      isTitleObj(t)
        ? { content: t.content, alignment: t.alignment ?? fallbackAlign, position: t.position ?? 'Top' }
        : { content: t as Line, alignment: fallbackAlign, position: 'Top' as const },
    );
  }
  return [{ content: title as Line, alignment: fallbackAlign, position: 'Top' }];
}

function sides(borders: BlockProps['borders']): { t: boolean; r: boolean; b: boolean; l: boolean } {
  if (borders === 'ALL') return { t: true, r: true, b: true, l: true };
  if (Array.isArray(borders)) return { t: borders.includes('TOP'), r: borders.includes('RIGHT'), b: borders.includes('BOTTOM'), l: borders.includes('LEFT') };
  return { t: false, r: false, b: false, l: false };
}

function paddingOf(p: Padding | undefined): { left: number; right: number; top: number; bottom: number } {
  if (typeof p === 'number') return { left: p, right: p, top: p, bottom: p };
  if (p && typeof p === 'object') return { left: p.left ?? 0, right: p.right ?? 0, top: p.top ?? 0, bottom: p.bottom ?? 0 };
  return { left: 0, right: 0, top: 0, bottom: 0 };
}

function TitleRow(props: { titles: TitleItem[]; style: BlockProps['titleStyle']; lead?: JSX.Element; bottom?: boolean; id?: string }) {
  const at = (a: Alignment) => props.titles.filter((t) => (t.alignment ?? 'Left') === a);
  return (
    <div class={cx('ui-block-title', props.bottom && 'ui-block-title-bottom')} id={props.id}>
      {props.lead}
      <span class="ui-block-t ui-block-t-left">
        <For each={at('Left')}>{(t) => <InlineLine line={t.content} style={props.style} />}</For>
      </span>
      <span class="ui-block-t ui-block-t-center">
        <For each={at('Center')}>{(t) => <InlineLine line={t.content} style={props.style} />}</For>
      </span>
      <span class="ui-block-t ui-block-t-right">
        <For each={at('Right')}>{(t) => <InlineLine line={t.content} style={props.style} />}</For>
      </span>
    </div>
  );
}

export type BlockFrameProps = {
  block: BlockProps;
  path: string[];
  mode: Mode;
  onToggle?: string;
  children?: JSX.Element;
};

/** Block chrome around `children` (the `Block` node's child, or a leaf with a `block` prop). */
export function BlockFrame(props: BlockFrameProps) {
  const entry = useEntry();
  const titleId = createUniqueId();
  const b = () => props.block;
  const titles = createMemo(() => titlesOf(b().title, b().titleAlignment));
  const top = () => titles().filter((t) => t.position === 'Top');
  const bottom = () => titles().filter((t) => t.position === 'Bottom');
  const [collapsed, setCollapsed] = useLocal<boolean>(entry.entryId, props.path, 'collapsed', {
    prop: () => b().collapsed,
    init: () => false,
  });
  const isCollapsed = () => !!b().collapsible && collapsed();
  const toggle = () => {
    const next = !collapsed();
    setCollapsed(next);
    entry.send(props.onToggle, next, props.path);
  };
  const frame = (): JSX.CSSProperties => {
    const s = sides(b().borders);
    const type = b().borderType ?? 'Plain';
    const width = type === 'Double' ? '3px' : type === 'Thick' ? '2px' : '1px';
    const line = type === 'Double' ? 'double' : 'solid';
    const color = tokenVar(b().borderStyle?.fg) ?? 'var(--ui-border)';
    const pad = paddingOf(b().padding);
    const base = styleCss(b().style);
    const rail = tokenVar(b().rail);
    return {
      ...base.style,
      'border-style': line,
      'border-color': color,
      'border-width': `${s.t ? width : 0} ${s.r ? width : 0} ${s.b ? width : 0} ${s.l ? width : 0}`,
      '--ui-pad-l': `calc(${pad.left} * var(--ui-col))`,
      '--ui-pad-r': `calc(${pad.right} * var(--ui-col))`,
      '--ui-pad-t': `calc(${pad.top} * var(--ui-row))`,
      '--ui-pad-b': `calc(${pad.bottom} * var(--ui-row))`,
      ...(rail ? { '--ui-rail': rail } : {}),
    } as JSX.CSSProperties;
  };
  const bordered = () => {
    const s = sides(b().borders);
    return s.t || s.r || s.b || s.l;
  };
  const showTop = () => top().length > 0 || !!b().collapsible;
  return (
    <section
      class={cx('ui-block', styleCss(b().style).class)}
      data-bordered={bordered() ? '' : undefined}
      data-type={(b().borderType ?? 'Plain').toLowerCase()}
      data-rail={b().rail ? '' : undefined}
      data-mode={props.mode}
      data-collapsed={isCollapsed() ? '' : undefined}
      aria-labelledby={showTop() ? titleId : undefined}
      style={frame()}
    >
      <Show when={showTop()}>
        <TitleRow
          id={titleId}
          titles={top()}
          style={b().titleStyle}
          lead={
            <Show when={b().collapsible}>
              <button
                type="button"
                class="ui-block-toggle"
                aria-expanded={!isCollapsed()}
                aria-label={isCollapsed() ? 'Expand' : 'Collapse'}
                onClick={toggle}
              >
                <ChevronRight size={14} class="ui-block-chevron" aria-hidden="true" />
              </button>
            </Show>
          }
        />
      </Show>
      <Show when={!isCollapsed()}>
        <div class="ui-block-body">{props.children}</div>
      </Show>
      <Show when={bottom().length > 0}>
        <TitleRow titles={bottom()} style={b().titleStyle} bottom />
      </Show>
    </section>
  );
}

export function Block(props: NodeProps<BlockNode>) {
  return (
    <BlockFrame block={props.node} path={props.path} mode={props.mode} onToggle={props.node.onToggle}>
      <NodeView node={props.node.child} path={childPath(props.path, 'child')} mode={props.mode} />
    </BlockFrame>
  );
}
