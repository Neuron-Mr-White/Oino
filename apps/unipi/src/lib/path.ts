/**
 * Path-chip presentation: label + colour for each `PathKind`
 * (DESIGN.md §1.1, §3.3). Colour + label always pair, so the chip is
 * never colour-only. `unknown` is shown honestly rather than guessed.
 */
import type { PathKind } from './schemas';

export const PATH_LABELS: Record<PathKind, string> = {
  lan: 'LAN',
  tailnet: 'Tailnet',
  direct: 'Direct',
  relay: 'Relay',
  local: 'Local',
  unknown: 'Unknown',
};

/** Tailwind text colour class for each path kind (cyan direct routes, amber relay, muted unknown). */
export const PATH_COLORS: Record<PathKind, string> = {
  lan: 'text-wm-cyan',
  tailnet: 'text-wm-cyan',
  direct: 'text-wm-cyan',
  relay: 'text-amber',
  local: 'text-wm-green',
  unknown: 'text-text-dim',
};

export function pathLabel(path: PathKind): string {
  return PATH_LABELS[path];
}

export function pathColorClass(path: PathKind): string {
  return PATH_COLORS[path];
}
