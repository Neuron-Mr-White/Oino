/**
 * Model picker target (WP-A14): the three entry points — the composer's model
 * chip, Ctrl/Cmd+M and `/model` (Z-70) — all call `openModelPicker`; the
 * shell mounts one `ModelPickerLayer` that renders it. A module signal like
 * `session/modalTarget` so a component can open it without importing the
 * sheet (and its data fetches) into the composer.
 */
import { createSignal } from 'solid-js';

export type ModelPickerTarget = { hostId: string; sessionId: string };

const [target, setTarget] = createSignal<ModelPickerTarget | undefined>();

/** The open picker's target (`undefined` = closed). */
export const modelPickerTarget = target;

/** Opens the picker for a session (already open = no-op). */
export function openModelPicker(next: ModelPickerTarget): void {
  setTarget((cur) => (cur && cur.hostId === next.hostId && cur.sessionId === next.sessionId ? cur : { ...next }));
}

/** Closes it. */
export function closeModelPicker(): void {
  setTarget(undefined);
}
