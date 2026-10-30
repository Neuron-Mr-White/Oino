/**
 * The editor screen's target (UNI-155): set before navigating to
 * `/files/edit` so the editor knows which host+path to open. One slot
 * (the editor is a single full-screen layer, like the terminal).
 */
import { createSignal } from 'solid-js';

export type EditorTarget = {
  hostId: string;
  path: string;
  line?: number;
  /** UNI-191 "Edit as text": open even a binary file as text (read-only
   *  until the user taps "Edit anyway"), via fs_read's `as_text`. */
  asText?: boolean;
};

const [target, setTargetSignal] = createSignal<EditorTarget | null>(null);

export function setEditorTarget(next: EditorTarget): void {
  setTargetSignal(next);
}

export function getEditorTarget(): EditorTarget | null {
  return target();
}

export function clearEditorTarget(): void {
  setTargetSignal(null);
}

/**
 * UNI-117 desktop: the Files list stays next to the editor, so another
 * file can be opened while one is being edited. The open editor installs
 * a guard; `switchEditorTarget` asks it first (a dirty editor answers
 * false and asks Save / Discard itself, then opens `next`).
 */
type Guard = (next: EditorTarget) => boolean;
let guard: Guard | undefined;
export function setEditorGuard(next: Guard | undefined): void {
  guard = next;
}
/** Opens `next` in the editor unless the open one holds unsaved changes (then it asks). */
export function switchEditorTarget(next: EditorTarget): boolean {
  if (guard && !guard(next)) return false;
  setTargetSignal(next);
  return true;
}
