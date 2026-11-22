/**
 * `Code` (ui-protocol §4.5.2): the chat code-block look (`.md .cb`: sticky
 * language + Copy header, `--md-cb-*` tokens), lazy CodeMirror/Lezer
 * highlighting, `lineNumbers` from `startLine`, `highlightLines` on
 * `surface-3`, `wrap`. Sources over 2 000 lines render in 500-line chunks
 * with `content-visibility: auto` (§13.4).
 */
import { For, Show, createEffect, createMemo, createResource } from 'solid-js';
import { copyText } from '../../lib/board/share';
import { canonicalLang, highlightHtml, isSupportedLang } from '../../lib/markdown';
import type { CodeNode } from '../types';
import { setTrustedHtml } from './Markdown';
import { baseStyle, cx, type NodeProps } from './props';

const CHUNK = 500;
const CHUNK_AFTER = 2000;

export function Code(props: NodeProps<CodeNode>) {
  const source = () => (typeof props.node.source === 'string' ? props.node.source.replace(/\n$/, '') : '');
  const lang = () => props.node.language?.trim() || 'text';
  const lines = createMemo(() => source().split('\n'));
  const start = () => Math.max(1, props.node.startLine ?? 1);
  const marked = createMemo(() => new Set(props.node.highlightLines ?? []));
  const [hl] = createResource(
    () => (isSupportedLang(lang()) ? ([source(), canonicalLang(lang())] as const) : undefined),
    async ([src, l]) => (await highlightHtml(src, l))?.split('\n'),
  );
  const chunks = createMemo(() => {
    const n = lines().length;
    if (n <= CHUNK_AFTER) return [[0, n] as const];
    const out: Array<readonly [number, number]> = [];
    for (let i = 0; i < n; i += CHUNK) out.push([i, Math.min(n, i + CHUNK)] as const);
    return out;
  });
  let copyBtn!: HTMLButtonElement;
  const copy = () =>
    void copyText(source()).then((ok) => {
      copyBtn.textContent = ok ? 'Copied' : 'Copy';
      setTimeout(() => (copyBtn.textContent = 'Copy'), 1200);
    });

  const Line = (p: { index: number }) => {
    const html = () => hl()?.[p.index];
    const hlAttr = () => (marked().has(start() + p.index) ? '' : undefined);
    return (
      <Show
        when={html() !== undefined}
        fallback={
          <span class="cb-line ui-code-line" data-hl={hlAttr()}>
            {lines()[p.index] || '\u200b'}
          </span>
        }
      >
        {/* Highlighted spans (built from escaped text) once the grammar is in. */}
        <span class="cb-line ui-code-line" data-hl={hlAttr()} ref={(el) => createEffect(() => setTrustedHtml(el, html() || '\u200b'))} />
      </Show>
    );
  };

  return (
    <div class={cx('md ui-code', baseStyle(props.node).class)} data-mode={props.mode} style={baseStyle(props.node).style}>
      <div class={cx('cb', props.node.lineNumbers && 'line-numbers', props.node.wrap && 'wrapped')}>
        <div class="cb-head">
          <span class="cb-lang">{lang()}</span>
          <span class="cb-head-actions">
            <button ref={copyBtn} type="button" class="cb-copy" onClick={copy}>
              Copy
            </button>
          </span>
        </div>
        <pre style={{ 'counter-reset': `cb-line ${start() - 1}` }}>
          <code data-lang={lang()}>
            <For each={chunks()}>
              {([from, to]) => (
                <Show when={chunks().length > 1} fallback={<For each={Array.from({ length: to - from }, (_, i) => from + i)}>{(i) => <Line index={i} />}</For>}>
                  <span class="ui-code-chunk">
                    <For each={Array.from({ length: to - from }, (_, i) => from + i)}>{(i) => <Line index={i} />}</For>
                  </span>
                </Show>
              )}
            </For>
          </code>
        </pre>
      </div>
    </div>
  );
}
