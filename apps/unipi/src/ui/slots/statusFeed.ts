/**
 * Status bar feed (WP-U10, G-4; ui-protocol §6.2 `status` + `footer` rows,
 * app spec §3.10): the session registry's `setStatus` / `setFrame` segments
 * and `setFooter` entries land in the conversation frame's status strip
 * (A3's `StatusStrip`) — in registry order (`order` desc, `seq` asc, §6.4).
 * A `footer` entry renders its tree as one segment (auto mode, one row), so
 * there is never a second footer surface.
 */
import { createEffect, onCleanup } from 'solid-js';
import { clearStatusScope, setStatusSegments, type StatusSegment } from '../../components/shell/StatusStrip';
import { plainLine } from '../nodes/Text';
import type { UiRegistry } from '../registry';
import type { Color, UiEntry } from '../types';

/** ui-protocol `tone` (Color) → the strip's tone word. */
export function toneOf(tone: Color | undefined): StatusSegment['tone'] {
  switch (tone) {
    case 'error':
    case 'errorBg':
    case 'diffRemoved':
      return 'error';
    case 'warning':
    case 'warningBg':
      return 'warn';
    case 'success':
    case 'successBg':
    case 'diffAdded':
      return 'ok';
    case 'info':
    case 'infoBg':
    case 'accent':
    case 'link':
      return 'info';
    default:
      return undefined;
  }
}

/** One status/footer entry as a strip segment. */
export function segmentOf(entry: UiEntry): StatusSegment & { key: string } {
  if (entry.slot === 'footer' && entry.view.kind === 'tree') {
    return { key: entry.id, title: entry.title, text: entry.title ?? '', node: entry.view.root };
  }
  if (entry.view.kind === 'status') return { key: entry.id, text: plainLine(entry.view.text), tone: toneOf(entry.tone) };
  if (entry.view.kind === 'lines') return { key: entry.id, text: entry.view.lines.join(' '), tone: toneOf(entry.tone) };
  if (entry.view.kind === 'tree') return { key: entry.id, title: entry.title, text: entry.title ?? '', node: entry.view.root };
  return { key: entry.id, text: entry.title ?? '' };
}

/** The ordered segments of both slots (status first by registry order each). */
export function statusListOf(registry: UiRegistry): Array<StatusSegment & { key: string }> {
  return [...registry.entriesFor('status')(), ...registry.entriesFor('footer')()].map(segmentOf);
}

/**
 * Feeds a scope's strip from the registry. Installed **once** per conversation
 * (identity accessors, not values): the effect re-points at a new scope or
 * registry when the route does, and its cleanup clears the old strip.
 */
export function feedStatusStrip(opts: { scope: () => string; registry: () => UiRegistry | undefined }): void {
  createEffect(() => {
    const scope = opts.scope();
    const registry = opts.registry();
    onCleanup(() => clearStatusScope(scope));
    setStatusSegments(scope, registry ? statusListOf(registry) : []);
  });
}
