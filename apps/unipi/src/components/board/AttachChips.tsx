/**
 * The attachments of a note, a reply or a description as chips (D2 / P3-1) — the chat composer's
 * attachment chip: a thumbnail (an image) or a file icon, the file's name, its size or kind, and ✕. The
 * text field itself shows only the words (`lib/board/attachDraft.ts`); ✕ takes the file's link out of the text.
 */
import { createResource, Show } from 'solid-js';
import { FileText, ImageOff } from 'lucide-solid';
import { formatBytes } from '../../lib/board/attach';
import type { DraftRef } from '../../lib/board/attachDraft';
import './note-field.css';

function DraftChip(props: { item: DraftRef; thumb: (ref: DraftRef) => Promise<string | undefined> | string | undefined; size?: number; onRemove: () => void }) {
  const [url] = createResource(
    () => (props.item.image ? props.item : undefined),
    (ref) => Promise.resolve(props.thumb(ref)).catch(() => undefined),
  );
  return (
    <span class="draft-chip" role="listitem" data-kind={props.item.image ? 'image' : 'file'} data-testid="draft-chip" title={props.item.name}>
      <Show
        when={props.item.image && url()}
        fallback={<span class="draft-chip-ic">{props.item.image ? <ImageOff size={14} /> : <FileText size={14} />}</span>}
      >
        {(src) => <img class="draft-chip-thumb" src={src()} alt="" />}
      </Show>
      <span class="draft-chip-body">
        <span class="draft-chip-name">{props.item.name}</span>
        <span class="draft-chip-meta">{props.size !== undefined ? formatBytes(props.size) : props.item.image ? 'Image' : 'File'}</span>
      </span>
      <button type="button" class="draft-chip-x" aria-label={`Remove ${props.item.name}`} onClick={props.onRemove}>
        ✕
      </button>
    </span>
  );
}

export function AttachChips(props: {
  refs: readonly DraftRef[];
  /** A URL for an image's thumbnail (an object URL of the held file, or the stored file's). */
  thumb: (ref: DraftRef) => Promise<string | undefined> | string | undefined;
  size?: (ref: DraftRef) => number | undefined;
  onRemove: (ref: DraftRef) => void;
}) {
  return (
    <Show when={props.refs.length > 0}>
      <div class="draft-chips" role="list" aria-label="Attached files">
        {props.refs.map((ref) => (
          <DraftChip item={ref} thumb={props.thumb} size={props.size?.(ref)} onRemove={() => props.onRemove(ref)} />
        ))}
      </div>
    </Show>
  );
}
