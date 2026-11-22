/**
 * ratatui `Span` / `Line` / `Text` (ui-protocol §4.2). Spans are `<span>`s
 * with modifier classes and token colours; a span `bg` is an inline chip;
 * `link` opens externally via `openLink` (never navigates the webview).
 */
import { For, Show, type JSX } from 'solid-js';
import { openLink } from '../../lib/markdown/links';
import { styleCss } from '../theme';
import type { Alignment, Line, LineObj, Span, SpanObj, Style, Text, TextObj } from '../types';

export function spanOf(s: Span): SpanObj {
  return typeof s === 'string' ? { content: s } : s && typeof s === 'object' ? s : { content: String(s ?? '') };
}

/** Normalizes a Line (`string | Span[] | {spans}`). A string with newlines stays one line (ratatui `Line::from`). */
export function lineOf(l: Line | undefined | null): LineObj {
  if (l == null) return { spans: [] };
  if (typeof l === 'string') return { spans: [l] };
  if (Array.isArray(l)) return { spans: l };
  if (typeof l === 'object' && Array.isArray((l as LineObj).spans)) return l as LineObj;
  return { spans: [] };
}

/** Normalizes a Text (`string` splits on `\n`, `Line[]`, `{lines}`). */
export function textOf(t: Text | undefined | null): TextObj {
  if (t == null) return { lines: [] };
  if (typeof t === 'string') return { lines: t.split('\n') };
  if (Array.isArray(t)) return { lines: t };
  if (typeof t === 'object' && Array.isArray((t as TextObj).lines)) return t as TextObj;
  return { lines: [] };
}

export function plainLine(l: Line | undefined | null): string {
  return lineOf(l)
    .spans.map((s) => spanOf(s).content)
    .join('');
}

export function plainText(t: Text | undefined | null): string {
  return textOf(t)
    .lines.map((l) => plainLine(l))
    .join('\n');
}

export function alignClass(a: Alignment | undefined): string | undefined {
  return a === 'Center' ? 'ui-al-center' : a === 'Right' ? 'ui-al-right' : undefined;
}

function cls(...parts: Array<string | undefined | false>): string {
  return parts.filter(Boolean).join(' ');
}

export function SpanView(props: { span: Span }) {
  const span = () => spanOf(props.span);
  const css = () => styleCss(span().style, { chip: true });
  const onClick = (event: MouseEvent) => {
    const href = span().link;
    if (!href) return;
    event.preventDefault();
    event.stopPropagation();
    void openLink(href);
  };
  return (
    <Show
      when={span().link}
      fallback={
        <span class={css().class || undefined} style={css().style}>
          {span().content}
        </span>
      }
    >
      <a class={cls('ui-link', css().class)} style={css().style} href={span().link} onClick={onClick} rel="noopener noreferrer">
        {span().content}
      </a>
    </Show>
  );
}

/** One line as an inline run of spans (wrap in a block element to make it a row). */
export function Spans(props: { line: Line | undefined | null }) {
  return <For each={lineOf(props.line).spans}>{(s) => <SpanView span={s} />}</For>;
}

/** A line as a block row (`<div class="ui-line">`), with its own style and alignment. */
export function LineView(props: { line: Line | undefined | null; class?: string; inheritAlign?: Alignment }) {
  const line = () => lineOf(props.line);
  const css = () => styleCss(line().style);
  return (
    <div class={cls('ui-line', props.class, alignClass(line().alignment ?? props.inheritAlign), css().class)} style={css().style}>
      <Spans line={line()} />
      {/* Keep empty rows one row tall. */}
      <Show when={line().spans.length === 0 || plainLine(line()) === ''}>{'\u200b'}</Show>
    </div>
  );
}

/** Inline line (no row box): titles, labels, badges. */
export function InlineLine(props: { line: Line | undefined | null; class?: string; style?: Style }) {
  const line = () => lineOf(props.line);
  const css = () => styleCss(props.style ?? line().style);
  return (
    <span class={cls(props.class, css().class) || undefined} style={css().style}>
      <Spans line={line()} />
    </span>
  );
}

/** A Text as block rows. */
export function TextView(props: { text: Text | undefined | null; align?: Alignment; class?: string; style?: JSX.CSSProperties }) {
  const text = () => textOf(props.text);
  const css = () => styleCss(text().style);
  return (
    <div class={cls(props.class, css().class) || undefined} style={{ ...css().style, ...props.style }}>
      <For each={text().lines}>{(l) => <LineView line={l} inheritAlign={text().alignment ?? props.align} />}</For>
    </div>
  );
}
