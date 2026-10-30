/**
 * The Files screen's initial target (UNI-155): set before navigating to
 * `/files` from "Files here" (chat header menu) or a terminal's known
 * cwd, so the Files screen opens straight into that host+directory
 * instead of the roots list. Read-once: the Files screen consumes it on
 * mount and clears it, so navigating back to Files later (bottom nav)
 * starts from the last-browsed directory again.
 */
import { createSignal } from 'solid-js';

export type FilesTarget = { hostId: string; path: string };

const [target, setTargetSignal] = createSignal<FilesTarget | null>(null);

export function setFilesTarget(next: FilesTarget): void {
  setTargetSignal(next);
}

/** Reads and clears the pending target, if any. */
export function consumeFilesTarget(): FilesTarget | null {
  const current = target();
  if (current) setTargetSignal(null);
  return current;
}
