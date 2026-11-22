/**
 * `Scrollbar` (ui-protocol §4.5.1): a read-only position indicator (thin
 * track + thumb, DESIGN scrollbar colours) for paged content. Scrollable
 * widgets use native scrollbars via their own `scrollbar` prop.
 */
import { tokenVar } from '../theme';
import type { ScrollbarNode } from '../types';
import { baseStyle, cx, type NodeProps } from './props';

export function thumbOf(state: ScrollbarNode['state'] | undefined): { start: number; size: number } {
  const len = Math.max(0, state?.contentLength ?? 0);
  if (len === 0) return { start: 0, size: 1 };
  const view = Math.max(1, state?.viewportContentLength || 1);
  const size = Math.min(1, view / Math.max(len, view));
  const pos = Math.min(Math.max(0, state?.position ?? 0), Math.max(0, len - 1));
  const start = len <= 1 ? 0 : (pos / (len - 1)) * (1 - size);
  return { start, size };
}

export function Scrollbar(props: NodeProps<ScrollbarNode>) {
  const vertical = () => (props.node.orientation ?? 'VerticalRight').startsWith('Vertical');
  const thumb = () => thumbOf(props.node.state);
  const thumbColor = () => tokenVar(props.node.thumbStyle?.fg) ?? 'var(--ui-scroll-thumb)';
  const trackColor = () => tokenVar(props.node.trackStyle?.fg) ?? 'var(--ui-scroll-track)';
  return (
    <div
      class={cx('ui-scrollbar', baseStyle(props.node).class)}
      data-orientation={vertical() ? 'v' : 'h'}
      data-side={(props.node.orientation ?? 'VerticalRight').replace(/^(Vertical|Horizontal)/, '').toLowerCase()}
      role="scrollbar"
      aria-orientation={vertical() ? 'vertical' : 'horizontal'}
      aria-valuemin={0}
      aria-valuemax={Math.max(0, (props.node.state?.contentLength ?? 0) - 1)}
      aria-valuenow={props.node.state?.position ?? 0}
      style={{ ...baseStyle(props.node).style, background: trackColor() }}
    >
      <div
        class="ui-scrollbar-thumb"
        style={
          vertical()
            ? { top: `${thumb().start * 100}%`, height: `${thumb().size * 100}%`, background: thumbColor() }
            : { left: `${thumb().start * 100}%`, width: `${thumb().size * 100}%`, background: thumbColor() }
        }
      />
    </div>
  );
}
