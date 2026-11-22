/**
 * Media intake for a board text field (OIN-95): OS file drop and paste, the way the chat composer takes them.
 * Chromium / WebView2 hand over `File`s; WebKitGTK (the Linux app) gives none for an OS drop or a clipboard
 * image, so the desktop core reads them (UNI-256). NoteField, the description editor and the new-task sheet
 * all use this; each passes what "attach" means for it (upload now, or hold until the task exists).
 */
import { onCleanup, onMount } from 'solid-js';
import { createFileDrop } from '../../lib/board/fileDrop';
import { classifyPaste } from '../../lib/composer/paste';
import { clipboardFiles, describeLocal, readLocalFiles, registerDropZone, startNativeDrop, type LocalFile } from '../../lib/localFiles';
import { isTauriDesktop } from '../../lib/platform';
import { boardUi } from '../../lib/board/ui';

/** The most the desktop core reads back for one pasted/dropped file (the chat composer's limit). */
export const MAX_LOCAL_READ = 200 * 1024 * 1024;

export function createMediaIntake(opts: {
  /** Takes the files (upload, hold, insert a chip …). */
  attach: (files: File[]) => void | Promise<void>;
  /** The drop target for the desktop core's native drop routing. */
  zone: () => HTMLElement | undefined;
  disabled?: () => boolean;
  /** Plain text that came with a native paste that held no files. */
  pasteText: (text: string) => void;
}) {
  const drop = createFileDrop((files) => void opts.attach(files));

  async function attachLocal(files: readonly LocalFile[]): Promise<boolean> {
    const usable = files.filter((f) => !f.isDir && f.size <= MAX_LOCAL_READ);
    if (usable.length < files.length) {
      const skipped = files.length - usable.length;
      boardUi().toast(
        skipped > 1
          ? `${skipped} items can't be attached (folders, or over 200 MB)`
          : `${files.find((f) => f.isDir || f.size > MAX_LOCAL_READ)!.name} can't be attached (a folder, or over 200 MB)`,
        'error',
      );
    }
    if (usable.length === 0) return files.length > 0;
    await opts.attach(await readLocalFiles(usable));
    return true;
  }

  onMount(() => {
    if (!isTauriDesktop()) return;
    void startNativeDrop();
    const off = registerDropZone({
      el: opts.zone,
      enabled: () => !opts.disabled?.(),
      hover: drop.setOver,
      drop: (paths) => {
        void describeLocal(paths).then(attachLocal).catch((error: unknown) => {
          console.warn('[drop] reading the dropped files failed', error);
          boardUi().toast("Couldn't read the dropped files", 'error');
        });
      },
    });
    onCleanup(off);
  });

  /** For the field's `onPaste`: files are taken (and the default paste cancelled); plain text pastes as usual. */
  const onPaste = (event: ClipboardEvent) => {
    const paste = classifyPaste(event.clipboardData, isTauriDesktop());
    if (paste.kind === 'files') {
      event.preventDefault();
      void opts.attach(paste.files);
    } else if (paste.kind === 'native') {
      event.preventDefault();
      const fallback = paste.text;
      void clipboardFiles()
        .then(attachLocal)
        .catch((error: unknown) => {
          console.warn('[paste] native clipboard read failed', error);
          return false;
        })
        .then((attached) => {
          if (!attached && fallback) opts.pasteText(fallback);
        });
    }
  };

  return { drop, onPaste };
}
