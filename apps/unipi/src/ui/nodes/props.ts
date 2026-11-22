/** Props shared by every node component (ui-protocol §13.3). */
import type { JSX } from 'solid-js';
import { styleCss } from '../theme';
import type { Common, NodePath } from '../types';

/**
 * Sizing mode of the node's box (§4.4): `definite` = the parent gave it a
 * fixed height (fill it, scroll inside); `auto` = grow with content up to the
 * slot max (`--ui-auto-max`), then scroll.
 */
export type Mode = 'auto' | 'definite';

export type NodeProps<T> = { node: T; path: NodePath; mode: Mode };

export function childPath(path: NodePath, seg: string): NodePath {
  return [...path, seg];
}

/** The node's base style (`style` prop) as class + inline style. */
export function baseStyle(node: Common | undefined): { class: string; style: JSX.CSSProperties } {
  return styleCss(node?.style);
}

export function cx(...parts: Array<string | undefined | false | null>): string {
  return parts.filter(Boolean).join(' ');
}

/** Item key of a collection member (§4.1). */
export function itemKey(item: unknown, index: number): string {
  if (item && typeof item === 'object' && !Array.isArray(item)) {
    const k = (item as { key?: unknown }).key;
    if (typeof k === 'string' && k) return k;
  }
  return `#${index}`;
}
