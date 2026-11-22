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

/** F-13: an extension's raw status text → a human label; `null` hides it (zero counts, internal words). */
export function humanStatus(raw: string): string | null {
  const text = raw
    .replace(/[\p{Extended_Pictographic}\u25CF\u2022\uFE0F]/gu, '')
    .replace(/\s+/g, ' ')
    .trim();
  if (!text) return null;
  const mem = /^(\d+)\s+mem(?:ories|ory)?$/i.exec(text);
  if (mem) return Number(mem[1]) > 0 ? `${mem[1]} ${mem[1] === '1' ? 'memory' : 'memories'}` : null;
  const skills = /^skills:\s*(\d+)\s*\/\s*(\d+)\s+listed$/i.exec(text);
  if (skills) return Number(skills[1]) > 0 ? `${skills[1]} ${skills[1] === '1' ? 'skill' : 'skills'} ready` : null;
  if (/^skills:\s*waiting/i.test(text)) return null;
  if (/^graph$/i.test(text)) return null;
  const zero = /^(.*?)[\s:]+0$/.exec(text);
  if (zero) return null;
  return text;
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
  const out: Array<StatusSegment & { key: string }> = [];
  for (const entry of [...registry.entriesFor('status')(), ...registry.entriesFor('footer')()]) {
    const seg = segmentOf(entry);
    if (seg.node) {
      out.push(seg);
      continue;
    }
    const text = humanStatus(seg.text);
    if (text !== null) out.push({ ...seg, text });
  }
  return out;
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
