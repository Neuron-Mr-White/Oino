/**
 * Per-user long-horizon view preferences ([lh-ui]): "Show errors" (default on) hides errored /
 * failed / superseded nodes from the progress bar, the list, the diagram and the timeline when
 * off. Persisted in localStorage; one shared signal so every open LH view follows it.
 */
import { createSignal } from 'solid-js';

const KEY = 'unipi.lh.showErrors';

function read(): boolean {
  try {
    return localStorage.getItem(KEY) !== 'off';
  } catch {
    return true;
  }
}

const [show, setShow] = createSignal(read());

export const showErrors = show;

export function setShowErrors(on: boolean): void {
  setShow(on);
  try {
    localStorage.setItem(KEY, on ? 'on' : 'off');
  } catch {
    /* private mode: the choice lasts for the session */
  }
}

/** Re-reads the stored value (tests). */
export function resetLhPrefs(): void {
  setShow(read());
}
