/**
 * UNI-256: local files from the OS in the desktop app (Tauri, desktop only).
 *
 * WebKitGTK gives the page no `File`s for a drop from the file manager and
 * an empty paste event for a clipboard image, so the core does it:
 *  - Tauri's window drag & drop (`dragDropEnabled: true`) reports the dropped
 *    PATHS; `startNativeDrop` routes them to the drop zone under the pointer
 *    (`registerDropZone`), which reads them as `File`s (`readLocalFiles`);
 *  - `clipboardFiles()` asks the core for the clipboard's copied files or
 *    screenshot (src-tauri/src/local_files.rs).
 * The core only reads back paths the user dropped or pasted.
 */
import { invoke } from '@tauri-apps/api/core';
import { dropPoint, mimeForName, type LocalFile } from './composer/paste';
import { isTauriDesktop } from './platform';

export type { LocalFile };

export async function clipboardFiles(): Promise<LocalFile[]> {
  if (!isTauriDesktop()) return [];
  return (await invoke('clipboard_files')) as LocalFile[];
}

export async function describeLocal(paths: string[]): Promise<LocalFile[]> {
  return (await invoke('local_files', { paths })) as LocalFile[];
}

/** Reads local files (dropped/pasted) into `File`s, typed by extension. */
export async function readLocalFiles(files: readonly LocalFile[]): Promise<File[]> {
  return Promise.all(
    files.map(async (f) => {
      const raw = (await invoke('local_file_bytes', { path: f.path })) as ArrayBuffer | number[];
      const bytes = raw instanceof ArrayBuffer ? new Uint8Array(raw) : Uint8Array.from(raw);
      return new File([bytes], f.name, { type: mimeForName(f.name) });
    }),
  );
}

// ── native drop routing ────────────────────────────────────────────────

export type DropZone = {
  /** The zone's element (undefined / hidden: not a target right now). */
  el: () => HTMLElement | undefined;
  /** Is it accepting drops right now (e.g. no dialog open)? */
  enabled?: () => boolean;
  /** The pointer is over it with files (true) or left / dropped (false). */
  hover: (over: boolean) => void;
  drop: (paths: string[]) => void;
};

const zones = new Set<DropZone>();

/** Registers a drop zone; returns its unregister function. */
export function registerDropZone(zone: DropZone): () => void {
  zones.add(zone);
  return () => {
    zones.delete(zone);
  };
}

/** The registered zone under a CSS-px point (innermost wins). */
export function zoneAt(x: number, y: number, doc: Document = document): DropZone | undefined {
  const hit = doc.elementFromPoint(x, y);
  if (!hit) return undefined;
  let best: { zone: DropZone; el: HTMLElement } | undefined;
  for (const zone of zones) {
    const el = zone.el();
    if (!el || !el.isConnected || el.closest('[hidden]') || (zone.enabled && !zone.enabled())) continue;
    if (el.contains(hit) && (!best || best.el.contains(el))) best = { zone, el };
  }
  return best?.zone;
}

let started = false;
/** Subscribes once to Tauri's window drag & drop (desktop app). */
export async function startNativeDrop(): Promise<void> {
  if (started || !isTauriDesktop()) return;
  started = true;
  try {
    const { getCurrentWebview } = await import('@tauri-apps/api/webview');
    let hovered: DropZone | undefined;
    let dragging = false;
    const setHovered = (z: DropZone | undefined) => {
      if (z === hovered) return;
      hovered?.hover(false);
      hovered = z;
      z?.hover(true);
    };
    await getCurrentWebview().onDragDropEvent((event) => {
      const p = event.payload;
      if (p.type === 'leave') {
        dragging = false;
        setHovered(undefined);
        return;
      }
      const pt = dropPoint(p.position, window.devicePixelRatio || 1);
      if (p.type === 'enter') dragging = p.paths.length > 0;
      if (!dragging) return;
      const zone = zoneAt(pt.x, pt.y);
      if (p.type === 'drop') {
        dragging = false;
        setHovered(undefined);
        if (zone && p.paths.length) zone.drop(p.paths);
        return;
      }
      setHovered(zone);
    });
  } catch (error) {
    started = false;
    console.warn('[drop] native drag & drop unavailable', error);
  }
}
