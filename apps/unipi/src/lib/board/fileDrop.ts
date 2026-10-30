/**
 * Drop files from the OS onto a board text field (UNI-117, the kanboard web
 * UI's TaskPanel/NewTaskDialog drop handlers). Only where the platform can
 * (`can('fileDragDrop')`: desktop app, web); on the phone these are no-ops.
 * A card drag (CARD_MIME) is never mistaken for files.
 */
import { createSignal, type Accessor } from 'solid-js';
import { can } from '../platform';

export function hasFiles(event: DragEvent): boolean {
  const types = event.dataTransfer?.types;
  return !!types && Array.from(types).includes('Files');
}

export function filesOf(event: DragEvent | ClipboardEvent): File[] {
  const list = 'dataTransfer' in event ? event.dataTransfer?.files : event.clipboardData?.files;
  return list ? [...list] : [];
}

/** `{ over, handlers }` — spread `handlers` on the drop zone, style it by `over()`. */
export function createFileDrop(onFiles: (files: File[]) => void): {
  over: Accessor<boolean>;
  handlers: { onDragOver: (event: DragEvent) => void; onDragLeave: (event: DragEvent) => void; onDrop: (event: DragEvent) => void };
} {
  const [over, setOver] = createSignal(false);
  const enabled = () => can('fileDragDrop');
  return {
    over,
    handlers: {
      onDragOver: (event) => {
        if (!enabled() || !hasFiles(event)) return;
        event.preventDefault();
        if (event.dataTransfer) event.dataTransfer.dropEffect = 'copy';
        setOver(true);
      },
      onDragLeave: (event) => {
        if (!(event.currentTarget as HTMLElement).contains(event.relatedTarget as Node | null)) setOver(false);
      },
      onDrop: (event) => {
        if (!enabled() || !hasFiles(event)) return;
        event.preventDefault();
        setOver(false);
        const files = filesOf(event);
        if (files.length > 0) onFiles(files);
      },
    },
  };
}

/** Ctrl/⌘+Enter (the web UI's "save / send" chord). */
export function isSubmitChord(event: KeyboardEvent): boolean {
  return event.key === 'Enter' && (event.ctrlKey || event.metaKey) && !event.isComposing;
}
