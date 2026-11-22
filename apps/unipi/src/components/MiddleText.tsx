/**
 * One line of text that gives way in the MIDDLE when it does not fit: the start shrinks to an
 * ellipsis and the tail (a file's extension and the last characters before it, a folder path's
 * last folder) always stays. Plain CSS can only cut at the end, so the text is two spans; the
 * full text stays whole in the DOM (copy, search, screen readers) and the caller sets `title`.
 */
import './middleText.css';

/** Splits `text` at the point the tail starts; no tail when the text is too short to be worth cutting. */
export function splitMiddle(text: string, opts: { file?: boolean; tail?: number } = {}): [head: string, tail: string] {
  const ext = opts.file ? /\.[A-Za-z0-9]{1,8}$/.exec(text)?.[0] : undefined;
  const keep = opts.tail ?? (ext ? ext.length + 4 : 12);
  if (text.length <= keep + 4) return [text, ''];
  return [text.slice(0, text.length - keep), text.slice(text.length - keep)];
}

export function MiddleText(props: { text: string; file?: boolean; tail?: number; class?: string }) {
  const parts = () => splitMiddle(props.text, { file: props.file, tail: props.tail });
  return (
    <span class={`mid${props.class ? ` ${props.class}` : ''}`}>
      <span class="mid-head">{parts()[0]}</span>
      <span class="mid-tail">{parts()[1]}</span>
    </span>
  );
}
