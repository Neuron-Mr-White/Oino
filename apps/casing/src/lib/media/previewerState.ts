/**
 * Previewer open/close state (UNI-157): a tiny signal store so any chip
 * (chat image, board attachment, file-path token) can open the shared
 * full-screen viewer without prop-drilling through every ancestor.
 */
import { createSignal } from 'solid-js';
import type { MediaKind } from './previewer';

export type PreviewItem = { url: string; mime: string; name?: string };

export type PreviewRequest =
  | { kind: 'image'; items: PreviewItem[]; index: number }
  | {
      kind: Exclude<MediaKind, 'image'>;
      name: string;
      mime: string;
      text?: string;
      url?: string;
      /** Where the file came from (a host path), when known — enables
       * Download / Open with… in the viewer header (UNI-169). */
      source?: { hostId: string; path: string };
      /** UNI-225: why there is no preview (too large to load inline) —
       * the "other" page then says so instead of "No preview for …". */
      reason?: 'too-large';
    }
  | {
      /** A folder's previewable siblings, paged lazily across media kinds
       * (UNI-168): swipe at the screen edge (or the chevrons) moves to the
       * previous/next file, whatever its kind. */
      kind: 'list';
      hostId: string;
      paths: string[];
      index: number;
    };

const [request, setRequest] = createSignal<PreviewRequest | undefined>();

export function previewerRequest() {
  return request;
}
export function openPreviewer(req: PreviewRequest): void {
  setRequest(req);
}
export function closePreviewer(): void {
  setRequest(undefined);
}
